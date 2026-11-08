// SPDX-License-Identifier: FSL-1.1-ALv2

package config

import "testing"

func TestLoggingLevel(t *testing.T) {
	for _, tc := range []struct {
		name  string
		value string
		want  LogLevel
		err   bool
	}{
		{name: "default", want: LogLevelInfo},
		{name: "info", value: "info", want: LogLevelInfo},
		{name: "debug", value: "DEBUG", want: LogLevelDebug},
		{name: "invalid", value: "trace", want: LogLevelInfo, err: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			p := &parser{lookup: func(name string) (string, bool) {
				if name == "LOG_LEVEL" && tc.value != "" {
					return tc.value, true
				}
				return "", false
			}}
			got := p.logging()
			if got.Level != tc.want || (len(p.errs) > 0) != tc.err {
				t.Fatalf("level = %v, errors = %v; want level %v, error %v", got.Level, p.errs, tc.want, tc.err)
			}
		})
	}
}
