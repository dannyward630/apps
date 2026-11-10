// SPDX-License-Identifier: FSL-1.1-ALv2

package service

import (
	"encoding/base64"
	"strings"
	"time"

	"github.com/google/uuid"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/apperr"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
)

const usageCursorPrefix = "u1|"

// ParseUsagePage keeps GetUsage's position cursor separate from the UUID
// cursors used by the other list RPCs. Canonical UUIDs are accepted for one
// release so callers holding a cursor issued before this change can finish.
func ParseUsagePage(p *mcpav1.Page) (Page, *store.UsageCursor, error) {
	page, err := ParsePage(&mcpav1.Page{Size: p.GetSize()})
	if err != nil {
		return Page{}, nil, err
	}
	raw := p.GetAfterId()
	if raw == "" {
		return page, nil, nil
	}
	if id, err := uuid.Parse(raw); err == nil && id.String() == raw {
		page.After = id
		return page, nil, nil
	}
	decoded, err := base64.RawURLEncoding.Strict().DecodeString(raw)
	if err != nil || len(decoded) != len(usageCursorPrefix)+10+1+36 || !strings.HasPrefix(string(decoded), usageCursorPrefix) || decoded[len(usageCursorPrefix)+10] != '|' {
		return Page{}, nil, apperr.InvalidInput("page.after_id is not a valid usage cursor")
	}
	if base64.RawURLEncoding.EncodeToString(decoded) != raw {
		return Page{}, nil, apperr.InvalidInput("page.after_id is not a canonical usage cursor")
	}
	dayText := string(decoded[len(usageCursorPrefix) : len(usageCursorPrefix)+10])
	day, err := time.Parse("2006-01-02", dayText)
	if err != nil || day.Format("2006-01-02") != dayText {
		return Page{}, nil, apperr.InvalidInput("page.after_id has an invalid day")
	}
	idText := string(decoded[len(usageCursorPrefix)+11:])
	id, err := uuid.Parse(idText)
	if err != nil || id.String() != idText {
		return Page{}, nil, apperr.InvalidInput("page.after_id has an invalid row ID")
	}
	return page, &store.UsageCursor{Day: day, ID: id}, nil
}

// EncodeUsageCursor preserves a row's logical position after its aggregate
// bucket is deleted. Page.after_id is opaque to clients.
func EncodeUsageCursor(day time.Time, id uuid.UUID) string {
	value := usageCursorPrefix + day.UTC().Format("2006-01-02") + "|" + id.String()
	return base64.RawURLEncoding.EncodeToString([]byte(value))
}
