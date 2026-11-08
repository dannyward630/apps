// SPDX-License-Identifier: FSL-1.1-ALv2

package main

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"go.opentelemetry.io/otel"
	collecttrace "go.opentelemetry.io/proto/otlp/collector/trace/v1"
	"google.golang.org/protobuf/proto"

	"github.com/delinoio/apps/servers/mcpa-api/internal/config"
	"github.com/delinoio/apps/servers/mcpa-api/internal/obs"
)

type collectedTrace struct {
	header string
	trace  *collecttrace.ExportTraceServiceRequest
}

func newTestCollector(t *testing.T) (*httptest.Server, <-chan collectedTrace) {
	t.Helper()
	requests := make(chan collectedTrace, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Errorf("read trace request: %v", err)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		var trace collecttrace.ExportTraceServiceRequest
		if err := proto.Unmarshal(body, &trace); err != nil {
			t.Errorf("decode trace request: %v", err)
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		requests <- collectedTrace{header: r.Header.Get("X-Test-Otel"), trace: &trace}
		w.WriteHeader(http.StatusOK)
	}))
	t.Cleanup(server.Close)
	return server, requests
}

func unsetTestEnv(t *testing.T, name string) {
	t.Helper()
	t.Setenv(name, "")
	if err := os.Unsetenv(name); err != nil {
		t.Fatal(err)
	}
}

func TestOTELDevEnvExportsAndUsesResourceSettings(t *testing.T) {
	for _, name := range []string{
		"OTEL_EXPORTER_OTLP_ENDPOINT",
		"OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
		"OTEL_EXPORTER_OTLP_HEADERS",
		"OTEL_SERVICE_NAME",
		"OTEL_RESOURCE_ATTRIBUTES",
	} {
		unsetTestEnv(t, name)
	}
	collector, requests := newTestCollector(t)
	path := filepath.Join(t.TempDir(), ".env")
	contents := "OTEL_EXPORTER_OTLP_ENDPOINT=" + collector.URL + "\n" +
		"OTEL_EXPORTER_OTLP_HEADERS=x-test-otel=from-dev-env\n" +
		"OTEL_SERVICE_NAME=mcpa-dev-test\n" +
		"OTEL_RESOURCE_ATTRIBUTES=deployment.environment=development\n"
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatal(err)
	}
	values, err := config.LoadDotEnv(path)
	if err != nil {
		t.Fatal(err)
	}
	restore, err := installOTELFallback(values)
	if err != nil {
		t.Fatal(err)
	}
	defer restore()

	previous := otel.GetTracerProvider()
	defer otel.SetTracerProvider(previous)
	shutdown, err := obs.SetupTracing(context.Background(), true)
	if err != nil {
		t.Fatal(err)
	}
	_, span := obs.Tracer().Start(context.Background(), "dev-env-export")
	span.End()
	if err := shutdown(context.Background()); err != nil {
		t.Fatal(err)
	}
	select {
	case received := <-requests:
		if received.header != "from-dev-env" {
			t.Errorf("header = %q, want from-dev-env", received.header)
		}
		resources := received.trace.GetResourceSpans()
		if len(resources) != 1 {
			t.Fatalf("resource spans = %d, want 1", len(resources))
		}
		attributes := map[string]string{}
		for _, a := range resources[0].GetResource().GetAttributes() {
			attributes[a.GetKey()] = a.GetValue().GetStringValue()
		}
		if attributes["service.name"] != "mcpa-dev-test" {
			t.Errorf("service.name = %q, want mcpa-dev-test", attributes["service.name"])
		}
		if attributes["deployment.environment"] != "development" {
			t.Errorf("deployment.environment = %q, want development", attributes["deployment.environment"])
		}
	default:
		t.Fatal("collector received no trace request")
	}
}

func TestOTELDevEnvKeepsProcessOverrides(t *testing.T) {
	processCollector, processRequests := newTestCollector(t)
	devCollector, devRequests := newTestCollector(t)
	t.Setenv("OTEL_EXPORTER_OTLP_ENDPOINT", processCollector.URL)
	unsetTestEnv(t, "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
	t.Setenv("OTEL_EXPORTER_OTLP_HEADERS", "x-test-otel=from-process")
	restore, err := installOTELFallback(map[string]string{
		"OTEL_EXPORTER_OTLP_ENDPOINT": devCollector.URL,
		"OTEL_EXPORTER_OTLP_HEADERS":  "x-test-otel=from-dev-env",
	})
	if err != nil {
		t.Fatal(err)
	}
	defer restore()
	previous := otel.GetTracerProvider()
	defer otel.SetTracerProvider(previous)
	shutdown, err := obs.SetupTracing(context.Background(), true)
	if err != nil {
		t.Fatal(err)
	}
	_, span := obs.Tracer().Start(context.Background(), "process-override")
	span.End()
	if err := shutdown(context.Background()); err != nil {
		t.Fatal(err)
	}
	select {
	case received := <-processRequests:
		if received.header != "from-process" {
			t.Errorf("header = %q, want from-process", received.header)
		}
	default:
		t.Fatal("process collector received no trace request")
	}
	select {
	case <-devRequests:
		t.Fatal("dev collector received a trace request despite process override")
	default:
	}
}

func TestOTELDevEnvKeepsEmptyProcessOverride(t *testing.T) {
	t.Setenv("OTEL_EXPORTER_OTLP_ENDPOINT", "")
	restore, err := installOTELFallback(map[string]string{
		"OTEL_EXPORTER_OTLP_ENDPOINT": "http://127.0.0.1:4318",
	})
	if err != nil {
		t.Fatal(err)
	}
	defer restore()
	if value, present := os.LookupEnv("OTEL_EXPORTER_OTLP_ENDPOINT"); !present || value != "" {
		t.Fatalf("endpoint = %q, present = %t; want present and empty", value, present)
	}
	lookup := config.WithFallback(os.LookupEnv, map[string]string{
		"OTEL_EXPORTER_OTLP_ENDPOINT": "http://127.0.0.1:4318",
	})
	if value, present := lookup("OTEL_EXPORTER_OTLP_ENDPOINT"); !present || value != "" {
		t.Fatalf("lookup endpoint = %q, present = %t; want present and empty", value, present)
	}
}
