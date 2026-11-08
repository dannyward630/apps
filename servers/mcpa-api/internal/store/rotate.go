// SPDX-License-Identifier: FSL-1.1-ALv2

package store

import (
	"context"
	"fmt"
	"log/slog"

	"github.com/google/uuid"
	"github.com/jackc/pgx/v5"

	"github.com/delinoio/apps/servers/mcpa-api/internal/crypto"
)

// sealedTable describes one table with envelope-encrypted columns.
type sealedTable struct {
	name string
	// resourceColumn is selected as text to rebuild the additional data. It
	// is an empty literal for tables without a resource.
	resourceColumn string
	aad            func(id, resourceID string) []byte
}

var sealedTables = []sealedTable{
	{name: "credentials", resourceColumn: "resource_id::text", aad: crypto.CredentialAAD},
	{name: "provider_oauth_flows", resourceColumn: "resource_id::text", aad: crypto.OAuthFlowAAD},
	{name: "console_sessions", resourceColumn: "''", aad: func(id, _ string) []byte { return crypto.SessionAAD(id) }},
	{name: "mcp_grants", resourceColumn: "''", aad: func(id, _ string) []byte { return crypto.MCPGrantAAD(id) }},
}

// RotationReport summarizes a master-key rotation.
type RotationReport struct {
	// Rewrapped counts rows moved to the active key per table.
	Rewrapped map[string]int
	// Remaining counts rows that still reference another key ID, per table
	// and key ID. Operators remove an old key only when this is empty.
	Remaining map[string]map[string]int
}

// RotateMasterKey re-wraps every stored data key that is not under the
// active key, in batches of batchSize rows per transaction. Secret
// ciphertexts are unchanged, so the server keeps serving during rotation as
// long as both keys stay configured.
func RotateMasterKey(ctx context.Context, s *Store, kr *crypto.Keyring, batchSize int, log *slog.Logger) (RotationReport, error) {
	if batchSize <= 0 {
		batchSize = 100
	}
	report := RotationReport{Rewrapped: map[string]int{}, Remaining: map[string]map[string]int{}}
	for _, t := range sealedTables {
		// The second pass catches rows that a concurrent writer re-sealed under
		// an old key after the first pass had already advanced beyond their IDs.
		// Bound the sweep to one pass; Remaining reports later races as before.
		for pass := 0; pass < 2; pass++ {
			lastID := uuid.Nil
			for {
				n, nextID, err := rotateBatch(ctx, s, kr, t, batchSize, lastID)
				if err != nil {
					return report, fmt.Errorf("rotate %s: %w", t.name, err)
				}
				if n == 0 {
					break
				}
				lastID = nextID
				report.Rewrapped[t.name] += n
				log.Info("rewrapped data keys", "table", t.name, "rows", n, "total", report.Rewrapped[t.name], "active_key_id", kr.ActiveKeyID())
			}
		}
		remaining, err := countByKeyID(ctx, s, t.name, kr.ActiveKeyID())
		if err != nil {
			return report, err
		}
		if len(remaining) > 0 {
			report.Remaining[t.name] = remaining
		}
	}
	return report, nil
}

func rotateBatch(ctx context.Context, s *Store, kr *crypto.Keyring, t sealedTable, batchSize int, after uuid.UUID) (int, uuid.UUID, error) {
	n := 0
	lastID := after
	err := s.InTx(ctx, func(tx pgx.Tx) error {
		rows, err := tx.Query(ctx, rotationBatchSQL(t), kr.ActiveKeyID(), batchSize, after)
		if err != nil {
			return err
		}
		type item struct {
			id     uuid.UUID
			old    string
			sealed crypto.Sealed
		}
		var items []item
		for rows.Next() {
			var id uuid.UUID
			var resourceID string
			var sl crypto.Sealed
			if err := rows.Scan(&id, &resourceID, &sl.KeyID, &sl.WrappedKey, &sl.KeyNonce, &sl.Nonce, &sl.Ciphertext); err != nil {
				rows.Close()
				return err
			}
			out, _, err := kr.Rewrap(sl, t.aad(id.String(), resourceID))
			if err != nil {
				rows.Close()
				// The key ID is not secret; it tells operators which key is missing.
				return fmt.Errorf("row %s under key %q: %w", id, sl.KeyID, err)
			}
			items = append(items, item{id: id, old: sl.KeyID, sealed: out})
			lastID = id
		}
		rows.Close()
		if err := rows.Err(); err != nil {
			return err
		}
		for _, it := range items {
			if _, err := tx.Exec(ctx, fmt.Sprintf(`UPDATE %s SET key_id = $2, wrapped_key = $3, key_nonce = $4
				WHERE id = $1 AND key_id = $5`, t.name), it.id, it.sealed.KeyID, it.sealed.WrappedKey, it.sealed.KeyNonce, it.old); err != nil {
				return err
			}
		}
		n = len(items)
		return nil
	})
	return n, lastID, err
}

func rotationBatchSQL(t sealedTable) string {
	return fmt.Sprintf(`SELECT id, %s, key_id, wrapped_key, key_nonce, nonce, ciphertext
		FROM %s WHERE id > $3 AND key_id <> $1 ORDER BY id LIMIT $2 FOR UPDATE`, t.resourceColumn, t.name)
}

func countByKeyID(ctx context.Context, s *Store, table, active string) (map[string]int, error) {
	rows, err := s.Pool.Query(ctx, fmt.Sprintf(`SELECT key_id, count(*) FROM %s WHERE key_id <> $1 GROUP BY key_id`, table), active)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := map[string]int{}
	for rows.Next() {
		var id string
		var n int
		if err := rows.Scan(&id, &n); err != nil {
			return nil, err
		}
		out[id] = n
	}
	return out, rows.Err()
}
