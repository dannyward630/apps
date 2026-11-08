// SPDX-License-Identifier: FSL-1.1-ALv2
//go:build integration

package store

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/url"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/jackc/pgx/v5"

	"github.com/delinoio/apps/servers/mcpa-api/internal/crypto"
)

func TestRotationPaginationPlanAndCounts(t *testing.T) {
	baseURL := os.Getenv("TEST_DATABASE_URL")
	if baseURL == "" {
		t.Skip("TEST_DATABASE_URL is not set; skipping PostgreSQL integration test")
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	admin, err := Open(ctx, baseURL, 10*time.Second, ModeMaintenance)
	if err != nil {
		t.Fatal(err)
	}
	schema := "it_rotation_" + strings.ReplaceAll(NewID().String(), "-", "")
	if _, err := admin.Pool.Exec(ctx, "CREATE SCHEMA "+pgx.Identifier{schema}.Sanitize()); err != nil {
		admin.Close()
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_, _ = admin.Pool.Exec(context.Background(), "DROP SCHEMA "+pgx.Identifier{schema}.Sanitize()+" CASCADE")
		admin.Close()
	})
	u, err := url.Parse(baseURL)
	if err != nil {
		t.Fatal("invalid TEST_DATABASE_URL")
	}
	q := u.Query()
	q.Set("search_path", schema)
	u.RawQuery = q.Encode()
	s, err := Open(ctx, u.String(), 10*time.Second, ModeServe)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(s.Close)
	if _, err := s.Migrate(ctx); err != nil {
		t.Fatal(err)
	}

	oldKey, newKey := bytes.Repeat([]byte{0x11}, 32), bytes.Repeat([]byte{0x22}, 32)
	old, err := crypto.NewKeyring(map[string][]byte{"old": oldKey}, "old")
	if err != nil {
		t.Fatal(err)
	}
	both, err := crypto.NewKeyring(map[string][]byte{"old": oldKey, "new": newKey}, "new")
	if err != nil {
		t.Fatal(err)
	}
	resourceID := uuid.MustParse("019a0000-0000-7000-8000-00000000f001")
	if _, err := s.Pool.Exec(ctx, `INSERT INTO resources (id, tenant_id, name, created_by_user_id)
		VALUES ($1, 'synthetic-tenant', 'rotation-bulk', 'synthetic-user')`, resourceID); err != nil {
		t.Fatal(err)
	}
	type row struct {
		id     uuid.UUID
		sealed crypto.Sealed
	}
	rows := make([]row, 5000)
	for i := range rows {
		id := uuid.MustParse(fmt.Sprintf("019a0000-0000-7000-8000-%012x", i+1))
		keyring := both
		if i >= 2500 {
			keyring = old
		}
		sealed, err := keyring.Seal([]byte(`{}`), crypto.CredentialAAD(id.String(), resourceID.String()))
		if err != nil {
			t.Fatal(err)
		}
		rows[i] = row{id: id, sealed: sealed}
	}
	conn, err := pgx.Connect(ctx, u.String())
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close(context.Background())
	_, err = conn.CopyFrom(ctx, pgx.Identifier{"credentials"},
		[]string{"id", "resource_id", "tenant_id", "provider", "kind", "name", "created_by_user_id",
			"key_id", "wrapped_key", "key_nonce", "nonce", "ciphertext"},
		pgx.CopyFromSlice(len(rows), func(i int) ([]any, error) {
			r := rows[i]
			return []any{r.id, resourceID, "synthetic-tenant", 1, 1, "rotation-bulk", "synthetic-user",
				r.sealed.KeyID, r.sealed.WrappedKey, r.sealed.KeyNonce, r.sealed.Nonce, r.sealed.Ciphertext}, nil
		}))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := conn.Exec(ctx, `ANALYZE credentials`); err != nil {
		t.Fatal(err)
	}

	cursor, rotated, examined := uuid.Nil, 0, 0
	for page := 0; ; page++ {
		var planJSON []byte
		if err := conn.QueryRow(ctx, `EXPLAIN (ANALYZE, FORMAT JSON) `+rotationBatchSQL(sealedTables[0]),
			both.ActiveKeyID(), 100, cursor).Scan(&planJSON); err != nil {
			t.Fatal(err)
		}
		var plans []struct{ Plan map[string]any }
		if err := json.Unmarshal(planJSON, &plans); err != nil || len(plans) != 1 {
			t.Fatalf("decode plan: %v", err)
		}
		if page == 1 && rowsRemovedByFilter(plans[0].Plan) > 250 {
			t.Fatal("second batch re-read the already rotated prefix")
		}
		examined += scanRowsExamined(plans[0].Plan)
		n, next, err := rotateBatch(ctx, s, both, sealedTables[0], 100, cursor)
		if err != nil {
			t.Fatal(err)
		}
		if n == 0 {
			break
		}
		rotated += n
		cursor = next
	}
	if rotated != 2500 || examined == 0 || examined > 7500 {
		t.Fatalf("rotated %d rows after examining %d of 5000 rows", rotated, examined)
	}
	report, err := RotateMasterKey(ctx, s, both, 100, slog.New(slog.NewTextHandler(io.Discard, nil)))
	if err != nil || len(report.Remaining) != 0 {
		t.Fatalf("post-rotation report = %+v, err = %v", report, err)
	}
	var oldCount int
	if err := s.Pool.QueryRow(ctx, `SELECT count(*) FROM credentials WHERE key_id = 'old'`).Scan(&oldCount); err != nil || oldCount != 0 {
		t.Fatalf("old key rows = %d, %v", oldCount, err)
	}
}

func rowsRemovedByFilter(plan map[string]any) int {
	removed, _ := plan["Rows Removed by Filter"].(float64)
	children, _ := plan["Plans"].([]any)
	for _, child := range children {
		removed += float64(rowsRemovedByFilter(child.(map[string]any)))
	}
	return int(removed)
}

func scanRowsExamined(plan map[string]any) int {
	name, _ := plan["Node Type"].(string)
	if strings.HasSuffix(name, "Scan") {
		rows, _ := plan["Actual Rows"].(float64)
		loops, _ := plan["Actual Loops"].(float64)
		removed, _ := plan["Rows Removed by Filter"].(float64)
		return int((rows + removed) * loops)
	}
	children, _ := plan["Plans"].([]any)
	for _, child := range children {
		if n := scanRowsExamined(child.(map[string]any)); n > 0 {
			return n
		}
	}
	return 0
}
