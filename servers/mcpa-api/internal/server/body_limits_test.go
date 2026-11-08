// SPDX-License-Identifier: FSL-1.1-ALv2

package server_test

import (
	"bytes"
	"compress/gzip"
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"connectrpc.com/connect"
	"google.golang.org/protobuf/encoding/protojson"

	delcorev1 "github.com/delinoio/apps/protos/delcore/v1"
	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/protos/mcpa/v1/mcpav1connect"
	"github.com/delinoio/apps/servers/mcpa-api/internal/auth"
	"github.com/delinoio/apps/servers/mcpa-api/internal/config"
	"github.com/delinoio/apps/servers/mcpa-api/internal/delcore"
	"github.com/delinoio/apps/servers/mcpa-api/internal/obs"
	"github.com/delinoio/apps/servers/mcpa-api/internal/server"
	"github.com/delinoio/apps/servers/mcpa-api/internal/web"
)

type countingKeys struct{ calls int }

func (k *countingKeys) AuthorizeAPIKey(context.Context, string, delcore.Capability, bool) (delcore.KeyDecision, error) {
	k.calls++
	return delcore.KeyDecision{OwnerKind: delcorev1.ApiKeyOwnerKind_API_KEY_OWNER_KIND_PERSONAL, UserID: "u1", TenantID: "t1"}, nil
}

type bodyLimitService struct {
	mcpav1connect.UnimplementedMcpaServiceHandler
	registrations int
	routings      int
}

func (s *bodyLimitService) RegisterApiKeyCredential(context.Context, *connect.Request[mcpav1.RegisterApiKeyCredentialRequest]) (*connect.Response[mcpav1.RegisterApiKeyCredentialResponse], error) {
	s.registrations++
	return connect.NewResponse(&mcpav1.RegisterApiKeyCredentialResponse{}), nil
}

func (s *bodyLimitService) UpdateRouting(context.Context, *connect.Request[mcpav1.UpdateRoutingRequest]) (*connect.Response[mcpav1.UpdateRoutingResponse], error) {
	s.routings++
	return connect.NewResponse(&mcpav1.UpdateRoutingResponse{}), nil
}

func newBodyLimitHandler(keys *countingKeys, svc *bodyLimitService, logs *bytes.Buffer) http.Handler {
	log := obs.NewLogger(logs, config.Logging{Format: config.LogFormatJSON})
	authn := auth.NewAuthenticator(auth.AuthenticatorConfig{WebOrigin: webOrigin, Keys: keys})
	return server.New(server.Config{
		Service: svc, Authenticator: authn,
		Web:     web.New(web.Config{Authenticator: authn, Log: slog.New(slog.NewTextHandler(io.Discard, nil))}),
		Metrics: obs.NewMetrics(), Log: log, WebOrigin: webOrigin,
	})
}

type countingReader struct {
	r io.Reader
	n int
}

func (r *countingReader) Read(p []byte) (int, error) {
	n, err := r.r.Read(p)
	r.n += n
	return n, err
}

func postCounted(h http.Handler, path string, body []byte, encoding string) (*httptest.ResponseRecorder, int) {
	reader := &countingReader{r: bytes.NewReader(body)}
	req := httptest.NewRequest(http.MethodPost, path, reader)
	req.ContentLength = int64(len(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer dlk_valid")
	if encoding != "" {
		req.Header.Set("Content-Encoding", encoding)
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec, reader.n
}

func gzipBody(t *testing.T, body []byte) []byte {
	t.Helper()
	var out bytes.Buffer
	zw := gzip.NewWriter(&out)
	if _, err := zw.Write(body); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return out.Bytes()
}

func TestManagementDecodedBodyLimitPrecedesAuthentication(t *testing.T) {
	// Unknown fields are normally discarded, so a smaller body reaches auth.
	body := []byte(`{"padding":"` + strings.Repeat("a", 300<<10) + `"}`)
	for _, tc := range []struct {
		name, encoding string
		body           []byte
	}{
		{name: "plain", body: body},
		{name: "gzip", body: gzipBody(t, body), encoding: "gzip"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var logs bytes.Buffer
			keys := &countingKeys{}
			svc := &bodyLimitService{}
			rec, read := postCounted(newBodyLimitHandler(keys, svc, &logs), registerAPIKey, tc.body, tc.encoding)
			var ce connectError
			if err := json.Unmarshal(rec.Body.Bytes(), &ce); err != nil {
				t.Fatal(err)
			}
			if rec.Code != http.StatusTooManyRequests || ce.Code != "resource_exhausted" {
				t.Fatalf("status %d, error %+v", rec.Code, ce)
			}
			if read >= len(tc.body) && tc.encoding == "" {
				t.Fatalf("read the entire %d-byte body", read)
			}
			if keys.calls != 0 || svc.registrations != 0 {
				t.Fatalf("authentication calls %d, service calls %d", keys.calls, svc.registrations)
			}
			if !strings.Contains(logs.String(), `"management request body rejected"`) ||
				!strings.Contains(logs.String(), `"message_limit_bytes":262144`) ||
				!strings.Contains(logs.String(), `"procedure":"`+registerAPIKey+`"`) {
				t.Fatalf("missing bounded rejection log: %s", logs.String())
			}
		})
	}
}

func TestManagementWireBodyLimitPrecedesAuthentication(t *testing.T) {
	// Empty gzip flush blocks use more than 1 MiB on the wire while decoding
	// to no bytes. This isolates the wire cap from the decoded-message cap.
	var out bytes.Buffer
	zw := gzip.NewWriter(&out)
	for out.Len() <= 1<<20 {
		if err := zw.Flush(); err != nil {
			t.Fatal(err)
		}
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	var logs bytes.Buffer
	keys := &countingKeys{}
	svc := &bodyLimitService{}
	rec, read := postCounted(newBodyLimitHandler(keys, svc, &logs), registerAPIKey, out.Bytes(), "gzip")
	var ce connectError
	if err := json.Unmarshal(rec.Body.Bytes(), &ce); err != nil {
		t.Fatal(err)
	}
	if rec.Code != http.StatusTooManyRequests || ce.Code != "resource_exhausted" {
		t.Fatalf("status %d, error %+v", rec.Code, ce)
	}
	if read >= len(out.Bytes()) || read > (1<<20)+1 {
		t.Fatalf("read %d of %d wire bytes", read, out.Len())
	}
	if keys.calls != 0 || svc.registrations != 0 {
		t.Fatalf("authentication calls %d, service calls %d", keys.calls, svc.registrations)
	}

	// A large plain body stops at the tighter decoded-message limit without
	// reading its remaining wire bytes.
	plain := []byte(`{"padding":"` + strings.Repeat("a", 2<<20) + `"}`)
	rec, read = postCounted(newBodyLimitHandler(keys, svc, &logs), registerAPIKey, plain, "")
	if rec.Code != http.StatusTooManyRequests || read >= len(plain) || keys.calls != 0 {
		t.Fatalf("plain status %d, read %d of %d bytes, authentication calls %d", rec.Code, read, len(plain), keys.calls)
	}
}

func TestManagementLargestSupportedMessagesPassTransportLimits(t *testing.T) {
	var logs bytes.Buffer
	keys := &countingKeys{}
	svc := &bodyLimitService{}
	h := newBodyLimitHandler(keys, svc, &logs)
	request := &mcpav1.RegisterApiKeyCredentialRequest{ApiKey: strings.Repeat("k", 8192)}
	credentialBody, err := protojson.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	rec, _ := postCounted(h, registerAPIKey, credentialBody, "")
	if rec.Code != http.StatusOK || svc.registrations != 1 {
		t.Fatalf("credential status %d, calls %d: %s", rec.Code, svc.registrations, rec.Body.String())
	}
	models := make([]string, 200)
	for i := range models {
		models[i] = strings.Repeat("m", 200)
	}
	routingBody, err := protojson.Marshal(&mcpav1.UpdateRoutingRequest{Routing: &mcpav1.RoutingSettings{ModelAllowlist: models}})
	if err != nil {
		t.Fatal(err)
	}
	rec, _ = postCounted(h, "/"+mcpav1connect.McpaServiceName+"/UpdateRouting", routingBody, "")
	if rec.Code != http.StatusOK || svc.routings != 1 || keys.calls != 2 {
		t.Fatalf("routing status %d, calls %d, authentication calls %d: %s", rec.Code, svc.routings, keys.calls, rec.Body.String())
	}
	if len(routingBody) <= 39<<10 {
		t.Fatalf("routing body is only %d bytes", len(routingBody))
	}
}
