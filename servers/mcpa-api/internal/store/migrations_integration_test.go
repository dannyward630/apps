// SPDX-License-Identifier: FSL-1.1-ALv2

//go:build integration

package store

import (
	"context"
	"crypto/sha256"
	"fmt"
	"net/url"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/jackc/pgx/v5"
)

func migrationTestStore(t *testing.T) *Store {
	t.Helper()
	raw := os.Getenv("TEST_DATABASE_URL")
	if raw == "" {
		t.Skip("TEST_DATABASE_URL is not set")
	}
	ctx := context.Background()
	schema := "it_" + strings.ReplaceAll(NewID().String(), "-", "")
	admin, err := Open(ctx, raw, 10*time.Second, ModeMaintenance)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := admin.Pool.Exec(ctx, "CREATE SCHEMA "+pgx.Identifier{schema}.Sanitize()); err != nil {
		admin.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() {
		defer admin.Close()
		if _, err := admin.Pool.Exec(context.Background(), "DROP SCHEMA "+pgx.Identifier{schema}.Sanitize()+" CASCADE"); err != nil {
			t.Errorf("drop test schema: %v", err)
		}
	})
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatal("TEST_DATABASE_URL is not a valid URL")
	}
	q := u.Query()
	q.Set("search_path", schema)
	u.RawQuery = q.Encode()
	s, err := Open(ctx, u.String(), 10*time.Second, ModeMaintenance)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(s.Close)
	return s
}

func testMigrationDigests(t *testing.T, s *Store, ms []Migration) {
	t.Helper()
	ctx := context.Background()
	rows, err := s.Pool.Query(ctx, `SELECT version, name, digest FROM `+migrationsTable+` ORDER BY version`)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	count := 0
	for rows.Next() {
		var version int
		var name, digest string
		if err := rows.Scan(&version, &name, &digest); err != nil {
			t.Fatal(err)
		}
		if count >= len(ms) {
			t.Fatalf("unexpected migration row %04d_%s", version, name)
		}
		if version != count+1 || name != ms[count].Name {
			t.Fatalf("migration row %d = %04d_%s", count, version, name)
		}
		if want := fmt.Sprintf("%x", sha256.Sum256([]byte(ms[count].SQL))); digest != want {
			t.Errorf("migration %04d digest = %s, want %s", version, digest, want)
		}
		count++
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	if count != len(ms) {
		t.Fatalf("recorded %d migrations, want %d", count, len(ms))
	}
}

func TestIntegrationMigrateRollsBackTableSetup(t *testing.T) {
	s := migrationTestStore(t)
	ctx := context.Background()
	ms, err := Migrations()
	if err != nil {
		t.Fatal(err)
	}
	bad := append(append([]Migration(nil), ms...), Migration{Version: 6, Name: "invalid_probe", SQL: "SELECT 'secret-marker'::integer;"})
	if _, err := s.migrate(ctx, bad); err == nil || strings.Contains(err.Error(), "secret-marker") || strings.Contains(err.Error(), "SELECT") {
		t.Fatalf("unsafe or missing migration failure: %v", err)
	}
	var exists bool
	if err := s.Pool.QueryRow(ctx, `SELECT to_regclass($1) IS NOT NULL`, migrationsTable).Scan(&exists); err != nil {
		t.Fatal(err)
	}
	if exists {
		t.Fatal("migrations table committed after a pending migration failed")
	}
	done, err := s.Migrate(ctx)
	if err != nil || len(done) != len(ms) {
		t.Fatalf("retry applied %v: %v", done, err)
	}
	testMigrationDigests(t, s, ms)
	if err := s.CheckSchema(ctx); err != nil {
		t.Fatal(err)
	}
	if done, err := s.Migrate(ctx); err != nil || len(done) != 0 {
		t.Fatalf("current migrate applied %v: %v", done, err)
	}
}

func TestIntegrationMigrateBackfillsLegacyRowsAtomically(t *testing.T) {
	s := migrationTestStore(t)
	ctx := context.Background()
	ms, err := Migrations()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.Pool.Exec(ctx, `CREATE TABLE `+migrationsTable+` (
		version integer PRIMARY KEY,
		name text NOT NULL,
		applied_at timestamptz NOT NULL DEFAULT now()
	)`); err != nil {
		t.Fatal(err)
	}
	for _, m := range ms[:digestMigrationVersion-1] {
		if _, err := s.Pool.Exec(ctx, m.SQL); err != nil {
			t.Fatal(err)
		}
		if _, err := s.Pool.Exec(ctx, `INSERT INTO `+migrationsTable+` (version, name) VALUES ($1, $2)`, m.Version, m.Name); err != nil {
			t.Fatal(err)
		}
	}
	edited := append([]Migration(nil), ms...)
	edited[1].SQL += "\n-- changed before the first digest backfill"
	if _, err := s.migrate(ctx, edited); err == nil || !strings.Contains(err.Error(), "migration 0002 digest differs from the trusted baseline") {
		t.Fatalf("edited legacy migration accepted: %v", err)
	}
	bad := append(append([]Migration(nil), ms...), Migration{Version: 6, Name: "invalid_probe", SQL: "SELECT 'secret-marker'::integer;"})
	if _, err := s.migrate(ctx, bad); err == nil || strings.Contains(err.Error(), "secret-marker") {
		t.Fatalf("unsafe or missing migration failure: %v", err)
	}
	var version int
	if err := s.Pool.QueryRow(ctx, `SELECT max(version) FROM `+migrationsTable).Scan(&version); err != nil || version != 4 {
		t.Fatalf("version after failed upgrade = %d, %v", version, err)
	}
	var hasDigest bool
	if err := s.Pool.QueryRow(ctx, `SELECT EXISTS (
		SELECT 1 FROM pg_attribute
		WHERE attrelid = to_regclass($1) AND attname = 'digest' AND NOT attisdropped
	)`, migrationsTable).Scan(&hasDigest); err != nil || hasDigest {
		t.Fatalf("digest column committed after failed upgrade: %v, %v", hasDigest, err)
	}
	done, err := s.Migrate(ctx)
	if err != nil || len(done) != 1 || done[0] != digestMigrationVersion {
		t.Fatalf("upgrade applied %v: %v", done, err)
	}
	testMigrationDigests(t, s, ms)
	if done, err := s.Migrate(ctx); err != nil || len(done) != 0 {
		t.Fatalf("current migrate applied %v: %v", done, err)
	}
}

func TestIntegrationCheckSchemaRejectsMigrationChanges(t *testing.T) {
	s := migrationTestStore(t)
	ctx := context.Background()
	ms, err := Migrations()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.Migrate(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Pool.Exec(ctx, `UPDATE `+migrationsTable+` SET digest = $1 WHERE version = 2`, strings.Repeat("0", 64)); err != nil {
		t.Fatal(err)
	}
	for _, check := range []struct {
		name string
		run  func() error
	}{
		{"check-schema", func() error { return s.CheckSchema(ctx) }},
		{"migrate", func() error { _, err := s.Migrate(ctx); return err }},
	} {
		if err := check.run(); err == nil || !strings.Contains(err.Error(), "migration 0002 digest differs from this binary") {
			t.Errorf("%s accepted changed digest: %v", check.name, err)
		}
	}
	if _, err := s.Pool.Exec(ctx, `UPDATE `+migrationsTable+` SET digest = $1, name = 'other' WHERE version = 2`, migrationDigest(ms[1])); err != nil {
		t.Fatal(err)
	}
	if err := s.CheckSchema(ctx); err == nil || !strings.Contains(err.Error(), "migration 0002 is") {
		t.Fatalf("changed migration name accepted: %v", err)
	}
	if _, err := s.Pool.Exec(ctx, `UPDATE `+migrationsTable+` SET name = $1 WHERE version = 2`, ms[1].Name); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Pool.Exec(ctx, `INSERT INTO `+migrationsTable+` (version, name, digest) VALUES (6, 'newer', $1)`, strings.Repeat("0", 64)); err != nil {
		t.Fatal(err)
	}
	if err := s.CheckSchema(ctx); err == nil || !strings.Contains(err.Error(), "deploy a newer binary") {
		t.Fatalf("unknown newer migration accepted: %v", err)
	}
}
