// SPDX-License-Identifier: FSL-1.1-ALv2

package service

import (
	"context"
	"errors"
	"strings"
	"testing"

	"connectrpc.com/connect"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/apperr"
	"github.com/delinoio/apps/servers/mcpa-api/internal/auth"
)

type countingUsageReader struct{ calls int }

func (r *countingUsageReader) GetUsage(context.Context, UsageQuery) (*mcpav1.GetUsageResponse, error) {
	r.calls++
	return &mcpav1.GetUsageResponse{}, nil
}

func (r *countingUsageReader) ListCallRecords(context.Context, UsageQuery) (*mcpav1.ListCallRecordsResponse, error) {
	r.calls++
	return &mcpav1.ListCallRecordsResponse{}, nil
}

func TestUsageRPCsRejectInvalidFiltersBeforeReading(t *testing.T) {
	models := make([]string, 101)
	for i := range models {
		models[i] = "m"
	}
	cases := []struct {
		name, field string
		filter      *mcpav1.CallFilter
	}{
		{"empty model", "filter.models", &mcpav1.CallFilter{Models: []string{""}}},
		{"NUL model", "filter.models", &mcpav1.CallFilter{Models: []string{"a\x00b"}}},
		{"control model", "filter.models", &mcpav1.CallFilter{Models: []string{"a\tb"}}},
		{"invalid UTF-8 model", "filter.models", &mcpav1.CallFilter{Models: []string{string([]byte{0xff})}}},
		{"model over 200 bytes", "filter.models", &mcpav1.CallFilter{Models: []string{strings.Repeat("m", 201)}}},
		{"101 models", "filter.models", &mcpav1.CallFilter{Models: models}},
		{"wrapped provider", "filter.providers", &mcpav1.CallFilter{Providers: []mcpav1.Provider{65537}}},
		{"negative wrapped provider", "filter.providers", &mcpav1.CallFilter{Providers: []mcpav1.Provider{-65535}}},
		{"undefined provider", "filter.providers", &mcpav1.CallFilter{Providers: []mcpav1.Provider{99}}},
		{"wrapped status", "filter.statuses", &mcpav1.CallFilter{Statuses: []mcpav1.CallStatus{65537}}},
		{"negative wrapped status", "filter.statuses", &mcpav1.CallFilter{Statuses: []mcpav1.CallStatus{-65535}}},
		{"undefined status", "filter.statuses", &mcpav1.CallFilter{Statuses: []mcpav1.CallStatus{99}}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			reader := &countingUsageReader{}
			svc := New(Config{})
			svc.SetHooks(Hooks{Usage: reader})
			ctx := auth.WithPrincipal(context.Background(), auth.NewUserPrincipal(mcpav1.ActorKind_ACTOR_KIND_CONSOLE, "user", "subject", nil))
			for _, call := range []struct {
				name string
				run  func() error
			}{
				{"GetUsage", func() error {
					_, err := svc.GetUsage(ctx, connect.NewRequest(&mcpav1.GetUsageRequest{Filter: tc.filter}))
					return err
				}},
				{"ListCallRecords", func() error {
					_, err := svc.ListCallRecords(ctx, connect.NewRequest(&mcpav1.ListCallRecordsRequest{Filter: tc.filter}))
					return err
				}},
			} {
				t.Run(call.name, func(t *testing.T) {
					var got *apperr.Error
					if err := call.run(); !errors.As(err, &got) || got.Reason != mcpav1.ErrorReason_ERROR_REASON_INVALID_INPUT ||
						got.Code != connect.CodeInvalidArgument || !strings.Contains(got.Message, tc.field) {
						t.Fatalf("error %v, want INVALID_INPUT naming %s", err, tc.field)
					}
				})
			}
			if reader.calls != 0 {
				t.Fatalf("usage reader called %d times for invalid filter", reader.calls)
			}
		})
	}
}

func TestUsageFilterAcceptsDefinedValuesAndDuplicates(t *testing.T) {
	for _, filter := range []*mcpav1.CallFilter{
		nil,
		{},
		{Models: []string{strings.Repeat("é", 100), strings.Repeat("é", 100)},
			Providers: []mcpav1.Provider{mcpav1.Provider_PROVIDER_UNSPECIFIED, mcpav1.Provider_PROVIDER_CODEX, mcpav1.Provider_PROVIDER_CODEX},
			Statuses:  []mcpav1.CallStatus{mcpav1.CallStatus_CALL_STATUS_UNSPECIFIED, mcpav1.CallStatus_CALL_STATUS_SUCCEEDED, mcpav1.CallStatus_CALL_STATUS_SUCCEEDED}},
	} {
		if err := validateFilter(filter); err != nil {
			t.Fatalf("valid filter %v rejected: %v", filter, err)
		}
	}
}
