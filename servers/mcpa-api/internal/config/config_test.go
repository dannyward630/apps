// SPDX-License-Identifier: FSL-1.1-ALv2

package config_test

import (
	"encoding/base64"
	"strings"
	"testing"

	"github.com/delinoio/apps/servers/mcpa-api/internal/config"
)

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
