// SPDX-License-Identifier: FSL-1.1-ALv2

package service_test

import (
	"context"
	"testing"

	"connectrpc.com/connect"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/apperr"
	"github.com/delinoio/apps/servers/mcpa-api/internal/auth"
	"github.com/delinoio/apps/servers/mcpa-api/internal/service"
)

func TestRegisterApiKeyCredentialRejectsBareQueryDelimiter(t *testing.T) {
	ctx := auth.WithPrincipal(context.Background(), auth.NewUserPrincipal(mcpav1.ActorKind_ACTOR_KIND_CONSOLE, "user", "subject", nil))
	_, err := new(service.Service).RegisterApiKeyCredential(ctx, connect.NewRequest(&mcpav1.RegisterApiKeyCredentialRequest{
		Provider: mcpav1.Provider_PROVIDER_OPENAI_COMPATIBLE,
		Name:     "Test provider",
		ApiKey:   "sk-test",
		BaseUrl:  "https://x.example/v1?",
		Models:   []string{"test-model"},
	}))
	got := apperr.As(err)
	if got.Reason != mcpav1.ErrorReason_ERROR_REASON_INVALID_INPUT || got.Message != "base_url must be an https URL without credentials, query or fragment" {
		t.Fatalf("error = %+v, want INVALID_INPUT for base_url", got)
	}
}
