// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"strings"
	"testing"
)

func TestSDKStdoutRelayBoundsAndRedactsLines(t *testing.T) {
	var output bytes.Buffer
	logger := slog.New(slog.NewJSONHandler(&output, &slog.HandlerOptions{Level: slog.LevelDebug})).With("component", "cliproxy-sdk")
	input := strings.Join([]string{
		"API server started successfully on: 127.0.0.1:-1",
		"server clients and configuration updated: 1 clients (1 auth entries + 0 Gemini API keys + 0 Claude API keys + 0 Codex keys + 0 Vertex-compat + 0 OpenAI-compat)",
		"model body: sk-secret",
		strings.Repeat("x", maxSDKStdoutLineBytes+100),
	}, "\n")
	relaySDKStdout(strings.NewReader(input), logger)

	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 4 {
		t.Fatalf("got %d records: %s", len(lines), output.String())
	}
	for i, line := range lines {
		var record map[string]any
		if err := json.Unmarshal([]byte(line), &record); err != nil {
			t.Fatalf("record %d: %v", i, err)
		}
		if record["level"] != "DEBUG" || record["component"] != "cliproxy-sdk" {
			t.Errorf("record %d = %v", i, record)
		}
		if i >= 2 && record["msg"] != "SDK stdout line omitted" {
			t.Errorf("unsafe record %d = %v", i, record)
		}
	}
	if strings.Contains(output.String(), "sk-secret") || strings.Contains(output.String(), strings.Repeat("x", 100)) {
		t.Fatalf("unsafe content in records: %s", output.String())
	}
}
