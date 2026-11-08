// SPDX-License-Identifier: FSL-1.1-ALv2
//go:build integration

package app_test

import (
	"bytes"
	"context"
	"io"
	"log/slog"
	"strings"
	"testing"
	"time"

	"github.com/delinoio/apps/servers/mcpa-api/internal/app"
	"github.com/delinoio/apps/servers/mcpa-api/internal/config"
	"github.com/delinoio/apps/servers/mcpa-api/internal/obs"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store/storetest"
)

func TestNewRefusesUnknownStoredKeyBeforeConstruction(t *testing.T) {
	db, databaseURL := storetest.OpenWithURL(t)
	storetest.SeedSealedRow(t, db, "credentials", "old-1")
	cfg := &config.Serve{
		Database: config.Database{URL: config.Redacted(databaseURL), DependencyTimeout: 5 * time.Second},
		Keys: config.Keys{
			MasterKeys:  map[string]config.Secret{"current": bytes.Repeat([]byte{0x42}, 32)},
			ActiveKeyID: "current",
		},
	}
	created, err := app.New(context.Background(), cfg, slog.New(slog.NewTextHandler(io.Discard, nil)), obs.NewMetrics())
	if created != nil {
		created.Close()
		t.Fatal("app.New constructed an app with an unknown stored key")
	}
	if err == nil || !strings.Contains(err.Error(), "credentials") || !strings.Contains(err.Error(), "old-1") {
		t.Fatalf("error = %v, want stored-key integrity failure", err)
	}
	if strings.Contains(err.Error(), "synthetic-") {
		t.Fatalf("error contains synthetic customer or ciphertext data: %v", err)
	}
}
