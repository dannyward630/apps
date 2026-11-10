// SPDX-License-Identifier: FSL-1.1-ALv2

package service

import (
	"encoding/base64"
	"errors"
	"strings"
	"testing"
	"time"

	"connectrpc.com/connect"
	"github.com/google/uuid"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/apperr"
)

func TestUsageCursorRoundTripAndLegacy(t *testing.T) {
	day := time.Date(2026, 11, 10, 0, 0, 0, 0, time.UTC)
	id := uuid.MustParse("019a4b45-7051-7800-9000-000000000001")
	raw := EncodeUsageCursor(day, id)
	page, cursor, err := ParseUsagePage(&mcpav1.Page{Size: 2, AfterId: raw})
	if err != nil || page.Size != 2 || page.After != uuid.Nil || cursor == nil || !cursor.Day.Equal(day) || cursor.ID != id {
		t.Fatalf("decoded page=%+v cursor=%+v err=%v", page, cursor, err)
	}
	page, cursor, err = ParseUsagePage(&mcpav1.Page{AfterId: id.String()})
	if err != nil || page.After != id || cursor != nil {
		t.Fatalf("legacy page=%+v cursor=%+v err=%v", page, cursor, err)
	}
	if _, err := ParsePage(&mcpav1.Page{AfterId: raw}); err == nil {
		t.Fatal("the shared parser accepted a GetUsage-only cursor")
	}
}

func TestUsageCursorRejectsInvalid(t *testing.T) {
	id := "019a4b45-7051-7800-9000-000000000001"
	cases := []string{
		"not-a-cursor",
		"019A4B45-7051-7800-9000-000000000001",
		EncodeUsageCursor(time.Date(2026, 11, 10, 0, 0, 0, 0, time.UTC), uuid.MustParse(id))[:20],
		base64.RawURLEncoding.EncodeToString([]byte("u2|2026-11-10|" + id)),
		base64.RawURLEncoding.EncodeToString([]byte("u1|2026-02-30|" + id)),
		base64.RawURLEncoding.EncodeToString([]byte("u1|2026-11-10|" + strings.ToUpper(id))),
	}
	for _, raw := range cases {
		_, _, err := ParseUsagePage(&mcpav1.Page{AfterId: raw})
		var appErr *apperr.Error
		if !errors.As(err, &appErr) || appErr.Code != connect.CodeInvalidArgument || appErr.Reason != mcpav1.ErrorReason_ERROR_REASON_INVALID_INPUT {
			t.Fatalf("cursor %q: %v, want INVALID_INPUT", raw, err)
		}
	}
}
