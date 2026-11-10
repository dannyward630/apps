// SPDX-License-Identifier: FSL-1.1-ALv2

package store

import (
	"crypto/sha256"
	"fmt"
	"strings"
	"testing"
	"testing/fstest"
)

func migrationFS(names ...string) fstest.MapFS {
	fsys := fstest.MapFS{}
	for _, n := range names {
		fsys[n] = &fstest.MapFile{Data: []byte("SELECT 1;")}
	}
	return fsys
}

func TestLoadMigrationsRequiresContiguousNumbers(t *testing.T) {
	ms, err := loadMigrations(migrationFS("0001_core.sql", "0002_data_plane.sql", "0003_mcp_oauth.sql"))
	if err != nil || len(ms) != 3 || ms[1].Version != 2 || ms[2].Name != "mcp_oauth" {
		t.Fatalf("migrations = %+v, %v", ms, err)
	}
	if _, err := loadMigrations(migrationFS("0001_core.sql", "0003_mcp_oauth.sql")); err == nil || !strings.Contains(err.Error(), "0002 is missing") {
		t.Fatalf("gap: %v", err)
	}
	if _, err := loadMigrations(migrationFS("0002_x.sql")); err == nil {
		t.Fatal("missing 0001 accepted")
	}
}

func TestEmbeddedMigrations(t *testing.T) {
	ms, err := Migrations()
	if err != nil {
		t.Fatal(err)
	}
	if len(ms) == 0 {
		t.Fatal("no migrations embedded")
	}
	v, err := ExpectedVersion()
	if err != nil {
		t.Fatal(err)
	}
	if v != len(ms) || v != ms[len(ms)-1].Version {
		t.Fatalf("expected version %d with %d migrations, last %+v", v, len(ms), ms[len(ms)-1])
	}
	knownDigests := map[int]string{
		1: "8cbf670ca0f4d94b0968b9a6bf901e36326a73c917710ca838d2080523bfc53c",
		2: "0ce1fec9e9599158268ee9248edb75d068b8e8093290099177c6d85734ffceaa",
		3: "b31e5a647cf1f5ee9432fa72069616888ef4412b0b49f142a8677af9ca1bcd5f",
		4: "14f3092b749d8882ee954621708ac3a698b414fa7a3265fb080ee9bd149ae867",
	}
	if len(ms) < len(knownDigests) {
		t.Fatalf("expected at least %d applied migrations, got %d", len(knownDigests), len(ms))
	}
	for _, m := range ms {
		if want, ok := knownDigests[m.Version]; ok {
			if got := fmt.Sprintf("%x", sha256.Sum256([]byte(m.SQL))); got != want {
				t.Errorf("migration %04d digest = %s, want %s", m.Version, got, want)
			}
		}
	}
}
