// SPDX-License-Identifier: FSL-1.1-ALv2
//go:build integration

package store_test

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store/storetest"
)

var sealedTableNames = []string{"credentials", "provider_oauth_flows", "console_sessions", "mcp_grants"}

func TestCheckSealedKeyIDs(t *testing.T) {
	t.Run("empty tables", func(t *testing.T) {
		db := storetest.Open(t)
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := store.CheckSealedKeyIDs(ctx, db, []string{"current"}); err != nil {
			t.Fatal(err)
		}
	})
	t.Run("all configured", func(t *testing.T) {
		db := storetest.Open(t)
		for _, table := range sealedTableNames {
			storetest.SeedSealedRow(t, db, table, "current")
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := store.CheckSealedKeyIDs(ctx, db, []string{"current"}); err != nil {
			t.Fatal(err)
		}
	})
	for _, table := range sealedTableNames {
		t.Run(table, func(t *testing.T) {
			db := storetest.Open(t)
			storetest.SeedSealedRow(t, db, table, "old-1")
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			err := store.CheckSealedKeyIDs(ctx, db, []string{"current"})
			if err == nil || !strings.Contains(err.Error(), table) || !strings.Contains(err.Error(), "old-1") {
				t.Fatalf("error = %v, want table and missing key ID", err)
			}
			if strings.Contains(err.Error(), "synthetic-") {
				t.Fatalf("error contains synthetic customer or ciphertext data: %v", err)
			}
		})
	}
}
