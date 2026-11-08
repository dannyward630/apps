// SPDX-License-Identifier: FSL-1.1-ALv2

package config_test

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"log/slog"
	"strings"
	"testing"

	"github.com/delinoio/apps/servers/mcpa-api/internal/config"
)

func TestMasterKeyErrorsDoNotExposeSwappedKey(t *testing.T) {
	key := base64.RawURLEncoding.EncodeToString(bytes.Repeat([]byte{0x8b}, 32))
	vars := serveSettings()
	vars["MCPA_MASTER_KEYS"] = fmt.Sprintf(`{%q:"k1"}`, key)
	_, err := config.LoadServe(lookup(vars))
	if err == nil || !strings.Contains(err.Error(), "MCPA_MASTER_KEYS entry 1") {
		t.Fatalf("error = %v, want numbered MCPA_MASTER_KEYS entry", err)
	}
	for i := 0; i+8 <= len(key); i++ {
		if strings.Contains(err.Error(), key[i:i+8]) {
			t.Fatal("configuration error contains master-key material")
		}
	}

	for name, raw := range map[string]string{
		"short key":       `{"k1":"not-base64"}`,
		"invalid ID":      `{"bad!id":"not-base64"}`,
		"empty key value": `{"k1":""}`,
	} {
		t.Run(name, func(t *testing.T) {
			vars["MCPA_MASTER_KEYS"] = raw
			_, err := config.LoadServe(lookup(vars))
			if err == nil || !strings.Contains(err.Error(), "MCPA_MASTER_KEYS entry 1") {
				t.Fatalf("error = %v, want numbered MCPA_MASTER_KEYS entry", err)
			}
		})
	}
	vars["MCPA_MASTER_KEYS"] = fmt.Sprintf(`{"k1":%q}`, key)
	if _, err := config.LoadServe(lookup(vars)); err != nil {
		t.Fatalf("valid master key rejected: %v", err)
	}
}

func TestConfigFormattingRedactsSecrets(t *testing.T) {
	vars := serveSettings()
	vars["DATABASE_URL"] = "postgres://distinct-db-secret@localhost/mcpa"
	vars["MCPA_MASTER_KEYS"] = `{"visible-key-id":"` + base64.RawURLEncoding.EncodeToString([]byte("distinct-master-key-bytes-32!!xx")) + `"}`
	vars["MCPA_ACTIVE_KEY_ID"] = "visible-key-id"
	vars["CLIENT_SECRET"] = "distinct-machine-secret"
	vars["MCPA_LOGTO_CLIENT_SECRET"] = "distinct-console-secret"
	vars["MCPA_GEMINI_CLI_OAUTH_CLIENT_ID"] = "gemini-id"
	vars["MCPA_GEMINI_CLI_OAUTH_CLIENT_SECRET"] = "distinct-gemini-secret"
	vars["MCPA_ANTIGRAVITY_OAUTH_CLIENT_ID"] = "antigravity-id"
	vars["MCPA_ANTIGRAVITY_OAUTH_CLIENT_SECRET"] = "distinct-antigravity-secret"
	vars["MCPA_METRICS_TOKEN"] = "distinct-metrics-secret"
	cfg, err := config.LoadServe(lookup(vars))
	if err != nil {
		t.Fatal(err)
	}

	var outputs []string
	for _, value := range []any{cfg, *cfg, cfg.Database, cfg.Keys} {
		outputs = append(outputs, fmt.Sprintf("%v", value), fmt.Sprintf("%+v", value), fmt.Sprintf("%#v", value))
		encoded, err := json.Marshal(value)
		if err != nil {
			t.Fatal(err)
		}
		outputs = append(outputs, string(encoded))
		for _, makeHandler := range []func(*bytes.Buffer) slog.Handler{
			func(out *bytes.Buffer) slog.Handler { return slog.NewTextHandler(out, nil) },
			func(out *bytes.Buffer) slog.Handler { return slog.NewJSONHandler(out, nil) },
		} {
			var out bytes.Buffer
			slog.New(makeHandler(&out)).Info("config", slog.Any("value", value))
			outputs = append(outputs, out.String())
		}
	}
	for _, output := range outputs {
		for _, secret := range []string{
			"distinct-db-secret", "distinct-master-key-bytes-32!!xx",
			"distinct-machine-secret", "distinct-console-secret", "distinct-gemini-secret",
			"distinct-antigravity-secret", "distinct-metrics-secret",
		} {
			if strings.Contains(output, secret) {
				t.Fatalf("formatted config exposes %q: %s", secret, output)
			}
		}
		if !strings.Contains(output, "[REDACTED]") {
			t.Fatalf("formatted config lacks redaction: %s", output)
		}
	}
	for _, output := range outputs[:3] {
		if !strings.Contains(output, "visible-key-id") || !strings.Contains(output, "gemini-id") {
			t.Fatalf("formatted Serve loses nonsecret fields: %s", output)
		}
	}
	if got := string(cfg.Database.URL); got != vars["DATABASE_URL"] {
		t.Fatal("database URL changed during parsing")
	}
	if got := string(cfg.ClientSecret); got != vars["CLIENT_SECRET"] {
		t.Fatal("client secret changed during parsing")
	}
	if got := string(cfg.MasterKeys["visible-key-id"]); got != "distinct-master-key-bytes-32!!xx" {
		t.Fatal("master key changed during parsing")
	}
}

func TestNormalizeOrigin(t *testing.T) {
	tests := []struct {
		name  string
		input string
		want  string
	}{
		{"HTTPS default port", "https://x.example:443", "https://x.example"},
		{"HTTPS numeric default port", "https://x.example:0443", "https://x.example"},
		{"HTTP default port", "http://x.example:80", "http://x.example"},
		{"HTTPS custom port", "https://x.example:8443", "https://x.example:8443"},
		{"uppercase scheme and host", "HTTP://LOCALHOST:46328", "http://localhost:46328"},
		{"IPv6 loopback", "http://[::1]:80", "http://[::1]"},
		{"trailing slash", "https://x.example:443/", "https://x.example"},
		{"development origin", "http://127.0.0.1:46328", "http://127.0.0.1:46328"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := config.NormalizeOrigin(tt.input)
			if err != nil || got != tt.want {
				t.Fatalf("NormalizeOrigin(%q) = %q, %v; want %q", tt.input, got, err, tt.want)
			}
		})
	}
}

func TestLoadServeCanonicalizesWebOrigin(t *testing.T) {
	vars := serveSettings()
	vars["WEB_ORIGIN"] = "https://x.example:443"
	vars["MCPA_WEB_RETURN_URLS"] = `["https://x.example/"]`
	cfg, err := config.LoadServe(lookup(vars))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.WebOrigin != "https://x.example" {
		t.Fatalf("WebOrigin = %q, want canonical browser origin", cfg.WebOrigin)
	}
	if len(cfg.WebReturnURLs) != 1 || cfg.WebReturnURLs[0] != "https://x.example/" {
		t.Fatalf("WebReturnURLs = %q, want unchanged return URL", cfg.WebReturnURLs)
	}
}

func TestLoadServeRejectsNonASCIIWebOrigin(t *testing.T) {
	vars := serveSettings()
	vars["WEB_ORIGIN"] = "https://é.example"
	_, err := config.LoadServe(lookup(vars))
	if err == nil || !strings.Contains(err.Error(), "WEB_ORIGIN must use an ASCII (punycode) host") {
		t.Fatalf("error = %v, want WEB_ORIGIN ASCII host error", err)
	}
}

func TestLoadServeRejectsNonLoopbackHTTPWebOrigin(t *testing.T) {
	vars := serveSettings()
	vars["WEB_ORIGIN"] = "http://x.example:80"
	_, err := config.LoadServe(lookup(vars))
	if err == nil || !strings.Contains(err.Error(), "WEB_ORIGIN must use https unless it targets a loopback host") {
		t.Fatalf("error = %v, want WEB_ORIGIN HTTPS error", err)
	}
}

func lookup(vars map[string]string) config.Lookup {
	return func(name string) (string, bool) {
		value, ok := vars[name]
		return value, ok
	}
}

func serveSettings() map[string]string {
	return map[string]string{
		"DATABASE_URL":             "postgres://mcpa:local-mcpa@127.0.0.1:55438/mcpa?sslmode=disable",
		"MCPA_PUBLIC_URL":          "http://127.0.0.1:46329",
		"WEB_ORIGIN":               "http://127.0.0.1:46328",
		"MCPA_WEB_RETURN_URLS":     `["http://127.0.0.1:46328/"]`,
		"MCPA_MASTER_KEYS":         `{"k1":"` + base64.RawURLEncoding.EncodeToString(make([]byte, 32)) + `"}`,
		"MCPA_ACTIVE_KEY_ID":       "k1",
		"MCPA_SERVICE_ID":          "019a0000-0000-7000-8000-000000000001",
		"DELCORE_URL":              "https://delcore.example",
		"DELCORE_RESOURCE":         "https://delcore.example",
		"LOGTO_TOKEN_URL":          "https://logto.example/token",
		"CLIENT_ID":                "mcpa-m2m",
		"CLIENT_SECRET":            "m2m-secret",
		"LOGTO_ISSUER":             "https://logto.example",
		"MCPA_LOGTO_CLIENT_ID":     "mcpa-web",
		"MCPA_LOGTO_CLIENT_SECRET": "web-secret",
		"MCPA_API_RESOURCE":        "https://mcpa.example",
		"MCPA_MCP_RESOURCE":        "https://mcpa.example/mcp",
		"MCPA_MFA_REAUTH_URL":      "https://logto.example/mfa",
		"MCPA_OAUTH_CLIENTS":       "{}",
		"MCPA_AUTHORIZE_CACHE_TTL": "30s",
		"MCPA_CHECKPOINT_INTERVAL": "5s",
		"MCPA_OUTBOX_INTERVAL":     "10s",
	}
}
