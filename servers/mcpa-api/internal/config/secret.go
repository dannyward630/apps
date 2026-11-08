// SPDX-License-Identifier: FSL-1.1-ALv2

package config

import (
	"encoding/json"
	"log/slog"
)

const redactedValue = "[REDACTED]"

// Redacted is a configuration secret that must be unwrapped at its consumer.
type Redacted string

func (v Redacted) String() string {
	if v == "" {
		return ""
	}
	return redactedValue
}

func (v Redacted) GoString() string { return v.String() }

func (v Redacted) LogValue() slog.Value { return slog.StringValue(v.String()) }

func (v Redacted) MarshalJSON() ([]byte, error) { return json.Marshal(v.String()) }

// Secret holds a master key without exposing its bytes through formatting.
type Secret []byte

func (v Secret) String() string {
	if len(v) == 0 {
		return ""
	}
	return redactedValue
}

func (v Secret) GoString() string { return v.String() }

func (v Secret) LogValue() slog.Value { return slog.StringValue(v.String()) }

func (v Secret) MarshalJSON() ([]byte, error) { return json.Marshal(v.String()) }
