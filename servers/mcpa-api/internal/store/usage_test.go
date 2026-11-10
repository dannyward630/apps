// SPDX-License-Identifier: FSL-1.1-ALv2

package store

import (
	"fmt"
	"strings"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/google/uuid"
)

func TestTruncateUTF8(t *testing.T) {
	cases := []struct {
		in   string
		n    int
		want string
	}{
		{"gpt-4o", 200, "gpt-4o"},
		{"abcdef", 3, "abc"},
		{"ab\x00c", 10, "abc"},
		{"a\xffb", 10, "ab"},
		{"한국어", 4, "한"},
		{"한국어", 6, "한국"},
	}
	for _, tc := range cases {
		if got := TruncateUTF8(tc.in, tc.n); got != tc.want {
			t.Fatalf("TruncateUTF8(%q, %d) = %q, want %q", tc.in, tc.n, got, tc.want)
		}
	}
	long := strings.Repeat("é", 300)
	if got := TruncateUTF8(long, MaxModelBytes); len(got) > MaxModelBytes || !utf8.ValidString(got) {
		t.Fatalf("long model truncated to %d bytes", len(got))
	}
}

func TestEventKey(t *testing.T) {
	id := uuid.MustParse("019a0000-0000-7000-8000-000000000042")
	if got := EventKey(id, 3); got != "mcpa:019a0000-0000-7000-8000-000000000042:3" {
		t.Fatalf("event key %q", got)
	}
}

func TestSpendRetentionOutlivesBillingPeriods(t *testing.T) {
	// Delcore billing periods last at most one month (31 days); the partial
	// first day of a period must stay measurable until the period ends.
	if SpendRetention <= 31*24*time.Hour || SpendRetention != 35*24*time.Hour {
		t.Fatalf("spend retention %s", SpendRetention)
	}
}

func TestMigrationsAreContiguous(t *testing.T) {
	ms, err := Migrations()
	if err != nil {
		t.Fatal(err)
	}
	if len(ms) == 0 {
		t.Fatal("no migrations embedded")
	}
	version, err := ExpectedVersion()
	if err != nil || version != len(ms) || version != ms[len(ms)-1].Version {
		t.Fatalf("expected version %d with %d migrations: %v", version, len(ms), err)
	}
	for i, m := range ms {
		if m.Version != i+1 {
			t.Errorf("migration %d has version %d", i+1, m.Version)
		}
		if name := fmt.Sprintf("%04d_%s.sql", m.Version, m.Name); !migrationName.MatchString(name) {
			t.Errorf("migration %d has invalid filename %q", m.Version, name)
		}
	}
}
