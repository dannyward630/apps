// SPDX-License-Identifier: FSL-1.1-ALv2

// Package storetest opens migrated PostgreSQL stores for integration tests.
package storetest

import (
	"context"
	"crypto/sha256"
	"net/url"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/jackc/pgx/v5"

	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
)

// Open connects to the disposable TEST_DATABASE_URL database, creates a
// schema private to the test, applies the migrations in it and drops it at
// cleanup. Tests that share one database therefore do not interfere. It
// skips the test when TEST_DATABASE_URL is not set.
func Open(t testing.TB) *store.Store {
	s, _ := OpenWithURL(t)
	return s
}

// OpenWithURL also returns the isolated schema URL for a second client.
func OpenWithURL(t testing.TB) (*store.Store, string) {
	t.Helper()
	raw := os.Getenv("TEST_DATABASE_URL")
	if raw == "" {
		t.Skip("TEST_DATABASE_URL is not set; skipping PostgreSQL integration tests")
	}
	ctx := context.Background()
	schema := "it_" + strings.ReplaceAll(store.NewID().String(), "-", "")
	admin, err := store.Open(ctx, raw, 10*time.Second, store.ModeMaintenance)
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
	s, err := store.Open(ctx, u.String(), 10*time.Second, store.ModeServe)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(s.Close)
	if _, err := s.Migrate(ctx); err != nil {
		t.Fatal(err)
	}
	return s, u.String()
}

// SeedSealedRow inserts synthetic sealed data for one encrypted table.
func SeedSealedRow(t testing.TB, s *store.Store, table, keyID string) {
	t.Helper()
	ctx := context.Background()
	id := store.NewID()
	resourceID := store.NewID()
	sealed := []byte("synthetic-ciphertext")
	if table == "credentials" || table == "provider_oauth_flows" {
		if _, err := s.Pool.Exec(ctx, `INSERT INTO resources (id, tenant_id, name, created_by_user_id)
			VALUES ($1, 'synthetic-tenant', $2, 'synthetic-user')`, resourceID, id.String()); err != nil {
			t.Fatal(err)
		}
	}
	var err error
	switch table {
	case "credentials":
		_, err = s.Pool.Exec(ctx, `INSERT INTO credentials
			(id, resource_id, tenant_id, provider, kind, name, created_by_user_id,
			 key_id, wrapped_key, key_nonce, nonce, ciphertext)
			VALUES ($1, $2, 'synthetic-tenant', 1, 1, 'synthetic', 'synthetic-user', $3, $4, $4, $4, $4)`,
			id, resourceID, keyID, sealed)
	case "provider_oauth_flows":
		_, err = s.Pool.Exec(ctx, `INSERT INTO provider_oauth_flows
			(id, resource_id, tenant_id, provider, kind, state, name, expire_time,
			 created_by_user_id, key_id, wrapped_key, key_nonce, nonce, ciphertext)
			VALUES ($1, $2, 'synthetic-tenant', 1, 1, 1, 'synthetic', now() + interval '1 hour',
			 'synthetic-user', $3, $4, $4, $4, $4)`, id, resourceID, keyID, sealed)
	case "console_sessions":
		hash := sha256.Sum256([]byte(id.String()))
		_, err = s.Pool.Exec(ctx, `INSERT INTO console_sessions
			(id, token_hash, user_id, subject, csrf_token, mfa, access_expire_time, expire_time,
			 key_id, wrapped_key, key_nonce, nonce, ciphertext)
			VALUES ($1, $2, 'synthetic-user', 'synthetic-subject', 'synthetic-csrf', true,
			 now() + interval '1 hour', now() + interval '1 hour', $3, $4, $4, $4, $4)`,
			id, hash[:], keyID, sealed)
	case "mcp_grants":
		_, err = s.Pool.Exec(ctx, `INSERT INTO mcp_grants
			(id, client_id, user_id, subject, expire_time, key_id,
			 wrapped_key, key_nonce, nonce, ciphertext)
			VALUES ($1, 'synthetic-client', 'synthetic-user', 'synthetic-subject',
			 now() + interval '1 hour', $2, $3, $3, $3, $3)`, id, keyID, sealed)
	default:
		t.Fatalf("unsupported sealed table %q", table)
	}
	if err != nil {
		t.Fatal(err)
	}
}
