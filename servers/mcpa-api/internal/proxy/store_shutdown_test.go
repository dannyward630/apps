// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"context"
	"log/slog"
	"testing"
	"time"
)

type shutdownPersister struct{}

func (shutdownPersister) PersistCredentialSecret(ctx context.Context, _, _ string, _ []byte) error {
	<-ctx.Done()
	return ctx.Err()
}

func TestAsyncPersisterCloseUsesShutdownContext(t *testing.T) {
	p := newAsyncPersister(shutdownPersister{}, slog.Default(), time.Hour)
	p.enqueue("resource", "credential", []byte("payload"))

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	started := time.Now()
	p.close(ctx)
	if elapsed := time.Since(started); elapsed > time.Second {
		t.Fatalf("close took %s after its shutdown context expired", elapsed)
	}
	if ctx.Err() == nil {
		t.Fatal("shutdown context did not expire")
	}
}
