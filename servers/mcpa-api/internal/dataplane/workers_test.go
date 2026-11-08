// SPDX-License-Identifier: FSL-1.1-ALv2

package dataplane

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"strings"
	"testing"
	"time"

	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
)

func TestRetentionCycleContinuesAfterRuleFailures(t *testing.T) {
	var logs bytes.Buffer
	dp := &DataPlane{
		cfg: Config{DependencyTimeout: time.Second},
		db:  &store.Store{},
		log: slog.New(slog.NewTextHandler(&logs, nil)),
		now: time.Now,
	}
	firstFailure := errors.New("first prune failed")
	secondFailure := errors.New("second prune failed")
	var visited []string
	rules := []retentionRule{
		{"first", func(context.Context, store.DB, time.Time, int) (int64, error) {
			visited = append(visited, "first")
			return 0, firstFailure
		}},
		{"middle", func(context.Context, store.DB, time.Time, int) (int64, error) {
			visited = append(visited, "middle")
			return 0, nil
		}},
		{"last", func(context.Context, store.DB, time.Time, int) (int64, error) {
			visited = append(visited, "last")
			return 0, secondFailure
		}},
	}
	err := dp.retentionCycle(context.Background(), rules, func(context.Context, store.DB) error {
		visited = append(visited, "console")
		return nil
	})
	if got := strings.Join(visited, ","); got != "first,middle,last,console" {
		t.Fatalf("execution order: %s", got)
	}
	if !errors.Is(err, firstFailure) || !errors.Is(err, secondFailure) || !strings.Contains(err.Error(), "first:") || !strings.Contains(err.Error(), "last:") {
		t.Fatalf("named rule failures: %v", err)
	}
	if got := logs.String(); !strings.Contains(got, "kind=first") || !strings.Contains(got, "kind=last") || !strings.Contains(got, "alert=true") {
		t.Fatalf("rule failure logs: %s", got)
	}
}

func TestRetentionCycleStopsOnCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	dp := &DataPlane{
		cfg: Config{DependencyTimeout: time.Second},
		db:  &store.Store{},
		log: quietLog(),
		now: time.Now,
	}
	var visited []string
	rules := []retentionRule{
		{"first", func(context.Context, store.DB, time.Time, int) (int64, error) {
			visited = append(visited, "first")
			cancel()
			return 0, ctx.Err()
		}},
		{"second", func(context.Context, store.DB, time.Time, int) (int64, error) {
			visited = append(visited, "second")
			return 0, nil
		}},
	}
	err := dp.retentionCycle(ctx, rules, func(context.Context, store.DB) error {
		visited = append(visited, "console")
		return nil
	})
	if !errors.Is(err, context.Canceled) || strings.Join(visited, ",") != "first" {
		t.Fatalf("cancellation: visited %v, error %v", visited, err)
	}
}

func TestRetentionCycleSuccessHasNoNewLogs(t *testing.T) {
	var logs bytes.Buffer
	dp := &DataPlane{
		cfg: Config{DependencyTimeout: time.Second},
		db:  &store.Store{},
		log: slog.New(slog.NewTextHandler(&logs, nil)),
		now: time.Now,
	}
	rules := []retentionRule{{"empty", func(context.Context, store.DB, time.Time, int) (int64, error) { return 0, nil }}}
	if err := dp.retentionCycle(context.Background(), rules, func(context.Context, store.DB) error { return nil }); err != nil {
		t.Fatal(err)
	}
	if logs.Len() != 0 {
		t.Fatalf("unexpected success log: %s", logs.String())
	}
}
