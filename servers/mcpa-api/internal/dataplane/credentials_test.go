// SPDX-License-Identifier: FSL-1.1-ALv2

package dataplane

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"reflect"
	"testing"
	"time"
)

func TestCredentialListenerBackoff(t *testing.T) {
	tests := []struct {
		name      string
		connected []bool
		want      []time.Duration
	}{
		{
			name:      "consecutive failures before LISTEN",
			connected: []bool{false, false, false, false, false, false, false, false},
			want:      []time.Duration{time.Second, 2 * time.Second, 4 * time.Second, 8 * time.Second, 16 * time.Second, 32 * time.Second, time.Minute, time.Minute},
		},
		{
			name:      "successful LISTEN resets the delay",
			connected: []bool{false, false, true, true, false},
			want:      []time.Duration{time.Second, 2 * time.Second, time.Second, time.Second, time.Second},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			s := &CredentialSync{log: slog.New(slog.NewTextHandler(io.Discard, nil))}
			var waits []time.Duration
			attempt := 0
			err := s.listenWith(ctx, func(context.Context) (bool, error) {
				connected := tt.connected[attempt]
				attempt++
				return connected, errors.New("disconnected")
			}, func(delay time.Duration) <-chan time.Time {
				waits = append(waits, delay)
				if len(waits) == len(tt.want) {
					cancel()
				}
				ready := make(chan time.Time, 1)
				ready <- time.Now()
				return ready
			})
			if err != nil {
				t.Fatalf("Listen returned %v, want nil", err)
			}
			if !reflect.DeepEqual(waits, tt.want) {
				t.Fatalf("retry delays = %v, want %v", waits, tt.want)
			}
		})
	}
}

func TestCredentialListenerCancellationDuringWait(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	s := &CredentialSync{log: slog.New(slog.NewTextHandler(io.Discard, nil))}
	attempts := 0
	waits := 0
	err := s.listenWith(ctx, func(context.Context) (bool, error) {
		attempts++
		return true, errors.New("disconnected")
	}, func(delay time.Duration) <-chan time.Time {
		waits++
		if delay != time.Second {
			t.Errorf("retry delay = %s, want 1s", delay)
		}
		cancel()
		return make(chan time.Time)
	})
	if err != nil || attempts != 1 || waits != 1 {
		t.Fatalf("cancelled Listen returned %v after %d attempts and %d waits", err, attempts, waits)
	}
}

func TestCredentialListenerCancellationDuringConnection(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	s := &CredentialSync{log: slog.New(slog.NewTextHandler(io.Discard, nil))}
	err := s.listenWith(ctx, func(context.Context) (bool, error) {
		cancel()
		return true, context.Canceled
	}, func(time.Duration) <-chan time.Time {
		t.Fatal("cancelled listener scheduled a retry")
		return nil
	})
	if err != nil {
		t.Fatalf("cancelled Listen returned %v, want nil", err)
	}
}
