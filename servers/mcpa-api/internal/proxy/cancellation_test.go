// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/testutil"
)

type cancellationAuthorizer struct {
	entered chan struct{}
}

func (a *cancellationAuthorizer) AuthorizeCall(ctx context.Context, _ CallPrincipal) error {
	close(a.entered)
	<-ctx.Done()
	return ctx.Err()
}

func (a *cancellationAuthorizer) CheckBudgets(context.Context, CallPrincipal) error { return nil }

func nextFinal(t *testing.T, sink *recordingSink) CallRecordUpdate {
	t.Helper()
	select {
	case id := <-sink.finals:
		return sink.waitFinal(t, id)
	case <-time.After(10 * time.Second):
		t.Fatal("no final call record")
		return CallRecordUpdate{}
	}
}

func waitCallMetric(t *testing.T, metrics *Metrics, format, provider, status string) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if testutil.ToFloat64(metrics.calls.WithLabelValues(format, provider, status)) == 1 {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatalf("no %s call metric for %s/%s", status, format, provider)
}

func TestClientCancellationDuringUpload(t *testing.T) {
	metrics, err := NewMetrics(prometheus.NewRegistry(), nil)
	if err != nil {
		t.Fatal(err)
	}
	h := newHarness(t, func(o *Options) { o.Metrics = metrics })
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	up := openAIUpstream(t, "unexpected")
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	conn, err := net.Dial("tcp", strings.TrimPrefix(h.srv.URL, "http://"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = fmt.Fprintf(conn, "POST /r/%s/v1/chat/completions HTTP/1.1\r\nHost: local.test\r\nAuthorization: Bearer %s\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n%s", resourceID, key, chatBody)
	if err != nil {
		t.Fatal(err)
	}
	_ = conn.Close()
	record := nextFinal(t, h.sink)
	if record.Status != mcpav1.CallStatus_CALL_STATUS_CLIENT_CANCELLED || record.ErrorClass != ErrorClassClientCancelled {
		t.Fatalf("record status %s class %s", record.Status, record.ErrorClass)
	}
	if up.hits.Load() != 0 {
		t.Fatalf("provider hits %d", up.hits.Load())
	}
	waitCallMetric(t, metrics, "openai", "unspecified", "client_cancelled")
	if got := testutil.ToFloat64(metrics.calls.WithLabelValues("openai", "unspecified", "client_cancelled")); got != 1 {
		t.Fatalf("client_cancelled calls %v", got)
	}
	for _, status := range []string{"internal_error", "rejected_dependency"} {
		if got := testutil.ToFloat64(metrics.calls.WithLabelValues("openai", "unspecified", status)); got != 0 {
			t.Fatalf("%s calls %v", status, got)
		}
	}
}

func TestClientCancellationDuringAuthorization(t *testing.T) {
	authz := &cancellationAuthorizer{entered: make(chan struct{})}
	metrics, err := NewMetrics(prometheus.NewRegistry(), nil)
	if err != nil {
		t.Fatal(err)
	}
	h := newHarness(t, func(o *Options) { o.Authorizer = authz; o.Metrics = metrics })
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	up := openAIUpstream(t, "unexpected")
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	srv := httptest.NewUnstartedServer(h.engine)
	srv.EnableHTTP2 = true
	srv.StartTLS()
	defer srv.Close()
	ctx, cancel := context.WithCancel(context.Background())
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, srv.URL+"/r/"+resourceID+"/v1/chat/completions", strings.NewReader(chatBody))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+key)
	finished := make(chan struct{})
	go func() {
		resp, err := srv.Client().Do(req)
		if err == nil {
			_, _ = io.Copy(io.Discard, resp.Body)
			_ = resp.Body.Close()
		}
		close(finished)
	}()
	select {
	case <-authz.entered:
	case <-time.After(10 * time.Second):
		t.Fatal("authorization did not start")
	}
	cancel()
	<-finished
	record := nextFinal(t, h.sink)
	if record.Status != mcpav1.CallStatus_CALL_STATUS_CLIENT_CANCELLED || record.ErrorClass != ErrorClassClientCancelled {
		t.Fatalf("record status %s class %s", record.Status, record.ErrorClass)
	}
	if up.hits.Load() != 0 {
		t.Fatalf("provider hits %d", up.hits.Load())
	}
	waitCallMetric(t, metrics, "openai", "unspecified", "client_cancelled")
	if got := testutil.ToFloat64(metrics.calls.WithLabelValues("openai", "unspecified", "client_cancelled")); got != 1 {
		t.Fatalf("client_cancelled calls %v", got)
	}
}

func TestClientCancellationBeforeProviderResponse(t *testing.T) {
	metrics, err := NewMetrics(prometheus.NewRegistry(), nil)
	if err != nil {
		t.Fatal(err)
	}
	h := newHarness(t, func(o *Options) { o.Metrics = metrics })
	entered := make(chan struct{})
	release := make(chan struct{})
	defer close(release)
	up := newUpstream(t, func(_ http.ResponseWriter, r *http.Request) {
		close(entered)
		select {
		case <-r.Context().Done():
		case <-release:
		}
	})
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	srv := httptest.NewUnstartedServer(h.engine)
	srv.EnableHTTP2 = true
	srv.StartTLS()
	defer srv.Close()
	ctx, cancel := context.WithCancel(context.Background())
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, srv.URL+"/r/"+resourceID+"/v1/chat/completions", strings.NewReader(chatBody))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+key)
	finished := make(chan struct{})
	go func() {
		resp, err := srv.Client().Do(req)
		if err == nil {
			_ = resp.Body.Close()
		}
		close(finished)
	}()
	select {
	case <-entered:
	case <-time.After(10 * time.Second):
		t.Fatal("provider was not contacted")
	}
	cancel()
	<-finished
	record := nextFinal(t, h.sink)
	if record.Status != mcpav1.CallStatus_CALL_STATUS_CLIENT_CANCELLED || record.ErrorClass != ErrorClassClientCancelled {
		t.Fatalf("record status %s class %s", record.Status, record.ErrorClass)
	}
	waitCallMetric(t, metrics, "openai", "openai_compatible", "client_cancelled")
	if got := testutil.ToFloat64(metrics.providerFailures.WithLabelValues("openai_compatible", "network")); got != 0 {
		t.Fatalf("network provider failures %v", got)
	}
	if got := testutil.ToFloat64(metrics.calls.WithLabelValues("openai", "openai_compatible", "client_cancelled")); got != 1 {
		t.Fatalf("client_cancelled calls %v", got)
	}
}

func TestProviderNetworkFailureWithLiveClient(t *testing.T) {
	metrics, err := NewMetrics(prometheus.NewRegistry(), nil)
	if err != nil {
		t.Fatal(err)
	}
	h := newHarness(t, func(o *Options) { o.Metrics = metrics })
	up := newUpstream(t, func(http.ResponseWriter, *http.Request) {})
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	up.srv.Close()
	res := h.call(http.MethodPost, "/r/"+resourceID+"/v1/chat/completions", key, chatBody, nil)
	record := h.sink.waitFinal(t, res.callID)
	if record.Status != mcpav1.CallStatus_CALL_STATUS_PROVIDER_ERROR || record.ErrorClass != ErrorClassNetwork {
		t.Fatalf("record status %s class %s", record.Status, record.ErrorClass)
	}
	waitCallMetric(t, metrics, "openai", "openai_compatible", "provider_error")
	if got := testutil.ToFloat64(metrics.providerFailures.WithLabelValues("openai_compatible", "network")); got != 1 {
		t.Fatalf("network provider failures %v", got)
	}
}

func TestOversizedRequestWithLiveClientKeepsRequestTooLarge(t *testing.T) {
	h := newHarness(t, func(o *Options) { o.MaxRequestBodyBytes = 16 })
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	up := openAIUpstream(t, "unexpected")
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	res := h.call(http.MethodPost, "/r/"+resourceID+"/v1/chat/completions", key, chatBody, nil)
	record := h.sink.waitFinal(t, res.callID)
	if record.Status != mcpav1.CallStatus_CALL_STATUS_INTERNAL_ERROR || record.ErrorClass != ErrorClassRequestTooLarge {
		t.Fatalf("record status %s class %s", record.Status, record.ErrorClass)
	}
	if up.hits.Load() != 0 {
		t.Fatalf("provider hits %d", up.hits.Load())
	}
}
