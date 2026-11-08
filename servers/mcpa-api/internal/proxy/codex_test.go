// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/credential"
)

type codexRequest struct {
	method        string
	url           string
	authorization string
}

type codexRecordingTransport struct {
	mu       sync.Mutex
	requests []codexRequest
}

func (r *codexRecordingTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	r.mu.Lock()
	r.requests = append(r.requests, codexRequest{
		method:        req.Method,
		url:           req.URL.String(),
		authorization: req.Header.Get("Authorization"),
	})
	r.mu.Unlock()
	return &http.Response{
		StatusCode: http.StatusUnauthorized,
		Header:     http.Header{"Content-Type": []string{"application/json"}},
		Body:       io.NopCloser(strings.NewReader(`{"error":{"message":"fixture rejection"}}`)),
		Request:    req,
	}, nil
}

func (r *codexRecordingTransport) snapshot() []codexRequest {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]codexRequest(nil), r.requests...)
}

func TestCodexCredentialDestinations(t *testing.T) {
	const model = "gpt-5.3-codex"
	for _, tc := range []struct {
		name    string
		kind    mcpav1.CredentialKind
		secret  string
		baseURL string
		wantURL string
		wantKey string
	}{
		{
			name:    "API key default",
			kind:    mcpav1.CredentialKind_CREDENTIAL_KIND_API_KEY,
			secret:  "sk-codex-fixture",
			wantURL: "https://api.openai.com/v1/responses",
			wantKey: "Bearer sk-codex-fixture",
		},
		{
			name:    "API key custom base URL",
			kind:    mcpav1.CredentialKind_CREDENTIAL_KIND_API_KEY,
			secret:  "sk-codex-fixture",
			baseURL: "https://llm.example.com/v1",
			wantURL: "https://llm.example.com/v1/responses",
			wantKey: "Bearer sk-codex-fixture",
		},
		{
			name:    "OAuth",
			kind:    mcpav1.CredentialKind_CREDENTIAL_KIND_OAUTH,
			secret:  `{"type":"codex","access_token":"oauth-codex-fixture","expired":"2099-01-01T00:00:00Z"}`,
			wantURL: "https://chatgpt.com/backend-api/codex/responses",
			wantKey: "Bearer oauth-codex-fixture",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if tc.baseURL != "" {
				if err := credential.ValidateUpstreamURL(tc.baseURL); err != nil {
					t.Fatal(err)
				}
			}
			transport := &codexRecordingTransport{}
			h := newHarness(t, func(o *Options) { o.BaseTransport = transport })
			resourceID := newResourceID()
			key := h.addKey(resourceID)
			h.addCredential(Credential{
				ID: newResourceID(), ResourceID: resourceID,
				Provider: mcpav1.Provider_PROVIDER_CODEX, Kind: tc.kind,
				Secret: []byte(tc.secret), BaseURL: tc.baseURL,
			})
			h.call(http.MethodPost, "/r/"+resourceID+"/v1/responses", key,
				`{"model":"`+model+`","input":"hello"}`, nil)
			got := transport.snapshot()
			if len(got) != 1 {
				t.Fatalf("provider requests = %d, want 1", len(got))
			}
			if got[0].method != http.MethodPost || got[0].url != tc.wantURL || got[0].authorization != tc.wantKey {
				t.Fatalf("provider request = %+v, want POST %s with expected bearer key", got[0], tc.wantURL)
			}
		})
	}
}
