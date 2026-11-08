// SPDX-License-Identifier: FSL-1.1-ALv2

package delcore

import (
	"bytes"
	"encoding/json"
	"fmt"
	"log/slog"
	"strings"
	"testing"

	"github.com/delinoio/apps/servers/mcpa-api/internal/config"
)

func TestConfigFormattingRedactsClientSecret(t *testing.T) {
	cfg := Config{BaseURL: "https://delcore.example", ClientID: "visible-client", ClientSecret: config.Redacted("distinct-delcore-secret")}
	outputs := []string{fmt.Sprintf("%v", cfg), fmt.Sprintf("%+v", cfg), fmt.Sprintf("%#v", cfg)}
	encoded, err := json.Marshal(cfg)
	if err != nil {
		t.Fatal(err)
	}
	outputs = append(outputs, string(encoded))
	for _, makeHandler := range []func(*bytes.Buffer) slog.Handler{
		func(out *bytes.Buffer) slog.Handler { return slog.NewTextHandler(out, nil) },
		func(out *bytes.Buffer) slog.Handler { return slog.NewJSONHandler(out, nil) },
	} {
		var out bytes.Buffer
		slog.New(makeHandler(&out)).Info("config", slog.Any("value", cfg))
		outputs = append(outputs, out.String())
	}
	for _, output := range outputs {
		if strings.Contains(output, "distinct-delcore-secret") || !strings.Contains(output, "[REDACTED]") {
			t.Fatalf("client config is not redacted: %s", output)
		}
		if !strings.Contains(output, "visible-client") {
			t.Fatalf("client config loses nonsecret ID: %s", output)
		}
	}
}
