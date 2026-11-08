// SPDX-License-Identifier: FSL-1.1-ALv2

package store

import (
	"context"
	"errors"
	"fmt"

	"github.com/jackc/pgx/v5"
)

// CheckSealedKeyIDs refuses sealed rows that the configured keyring cannot
// open. It reads key IDs only, never ciphertexts or customer data.
func CheckSealedKeyIDs(ctx context.Context, s *Store, configured []string) error {
	for _, table := range sealedTables {
		var missing string
		err := s.Pool.QueryRow(ctx, fmt.Sprintf(`SELECT DISTINCT key_id FROM %s
			WHERE key_id <> ALL($1::text[]) ORDER BY key_id LIMIT 1`, table.name), configured).Scan(&missing)
		if errors.Is(err, pgx.ErrNoRows) {
			continue
		}
		if err != nil {
			return fmt.Errorf("check stored %s master-key IDs: %w", table.name, err)
		}
		return fmt.Errorf("stored %s reference master key %q that is not configured", table.name, missing)
	}
	return nil
}
