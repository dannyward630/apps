// SPDX-License-Identifier: FSL-1.1-ALv2

//go:build integration

package store_test

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/google/uuid"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
)

func TestIntegrationListCallRecordsManyIdleResources(t *testing.T) {
	s := openTestStore(t)
	ctx := context.Background()
	resources := make([]uuid.UUID, 2000)
	for i := range resources {
		resources[i] = store.NewID()
	}
	requested := append([]uuid.UUID(nil), resources[:1500]...)
	requested = append(requested, resources[0], resources[1]) // Repeated authorized IDs must not repeat records.
	key := store.NewID()
	_, err := s.Pool.Exec(ctx, `INSERT INTO call_records
		(id, tenant_id, resource_id, call_key_id, provider, model, format, status, checkpoint_seq,
		 final, start_time, expire_time)
		SELECT gen_random_uuid(), 'query-plan-tenant',
			CASE WHEN n <= 450 THEN ($1::uuid[])[((n - 1) % 5) + 1]
				 ELSE ($2::uuid[])[((n - 451) % 500) + 1] END,
			$3, CASE WHEN n % 2 = 0 THEN 1 ELSE 2 END,
			CASE WHEN n % 2 = 0 THEN 'model-a' ELSE 'model-b' END,
			1, CASE WHEN n % 3 = 0 THEN 2 ELSE 1 END, 1, true,
			now() - interval '1 day', now() + interval '29 days'
		FROM generate_series(1, 200000) AS n`, resources[:5], resources[1500:], key)
	if err != nil {
		t.Fatal(err)
	}
	_, err = s.Pool.Exec(ctx, `INSERT INTO call_records
		(id, tenant_id, resource_id, call_key_id, provider, model, format, status, checkpoint_seq,
		 final, start_time, expire_time)
		VALUES (gen_random_uuid(), 'other-tenant', $1, $2, 1, 'model-a', 1, 1, 1,
			true, now() - interval '1 day', now() + interval '29 days')`, resources[0], key)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := s.Pool.Exec(ctx, `ANALYZE call_records`); err != nil {
		t.Fatal(err)
	}
	filters := []struct {
		name string
		f    store.UsageFilter
	}{
		{"all", store.UsageFilter{}},
		{"model", store.UsageFilter{Models: []string{"model-a"}}},
		{"provider", store.UsageFilter{Providers: []mcpav1.Provider{mcpav1.Provider(2)}}},
		{"status", store.UsageFilter{Statuses: []mcpav1.CallStatus{mcpav1.CallStatus(2)}}},
		{"call key", store.UsageFilter{CallKeyIDs: []uuid.UUID{key}}},
		{"time", store.UsageFilter{Start: timePointer(time.Now().Add(-48 * time.Hour)), End: timePointer(time.Now())}},
	}
	for _, tc := range filters {
		t.Run(tc.name, func(t *testing.T) {
			f := tc.f
			f.TenantID, f.ResourceIDs = "query-plan-tenant", requested
			var before uuid.UUID
			seen := make(map[uuid.UUID]bool)
			for {
				got, err := store.ListCallRecords(ctx, s.Pool, f, before, 100)
				if err != nil {
					t.Fatal(err)
				}
				want := referenceCallRecordIDs(t, ctx, s, f, before, 100)
				if len(got) != len(want) {
					t.Fatalf("page after %s: got %d rows, want %d", before, len(got), len(want))
				}
				for i, row := range got {
					if row.ID != want[i] || seen[row.ID] {
						t.Fatalf("page after %s: row %d = %s, want %s; duplicate=%t", before, i, row.ID, want[i], seen[row.ID])
					}
					seen[row.ID] = true
				}
				if len(got) < 100 {
					break
				}
				before = got[len(got)-1].ID
			}
			if tc.name == "all" && len(seen) != 450 {
				t.Fatalf("listed %d rows, want 450", len(seen))
			}
		})
	}
	wrongTenant := store.UsageFilter{TenantID: "other-tenant", ResourceIDs: requested}
	if rows, err := store.ListCallRecords(ctx, s.Pool, wrongTenant, uuid.Nil, 100); err != nil || len(rows) != 1 || rows[0].TenantID != "other-tenant" {
		t.Fatalf("other tenant returned %d rows: %v", len(rows), err)
	}
	if rows, err := store.ListCallRecords(ctx, s.Pool, store.UsageFilter{
		TenantID: "query-plan-tenant", ResourceIDs: resources[1500:],
	}, uuid.Nil, 100); err != nil || len(rows) != 100 {
		t.Fatalf("active authorized resources returned %d rows: %v", len(rows), err)
	}
	// EXPLAIN the resource-driven shape under the same 1,500-ID distribution.
	plan, err := s.Pool.Query(ctx, `EXPLAIN SELECT page.id FROM
		(SELECT DISTINCT id FROM unnest($1::uuid[]) AS r(id)) AS resources
		CROSS JOIN LATERAL (SELECT c.id FROM call_records AS c
			WHERE c.tenant_id = 'query-plan-tenant' AND c.resource_id = resources.id
			ORDER BY c.id DESC LIMIT 100) AS page
		ORDER BY page.id DESC LIMIT 100`, requested)
	if err != nil {
		t.Fatal(err)
	}
	defer plan.Close()
	var lines []string
	for plan.Next() {
		var line string
		if err := plan.Scan(&line); err != nil {
			t.Fatal(err)
		}
		lines = append(lines, line)
	}
	if err := plan.Err(); err != nil {
		t.Fatal(err)
	}
	text := strings.Join(lines, "\n")
	if !strings.Contains(text, "call_records_resource_idx") || strings.Contains(text, "call_records_pkey") {
		t.Fatalf("unexpected call-record plan:\n%s", text)
	}
}

func TestIntegrationListUsageRowsManyIdleResources(t *testing.T) {
	s := openTestStore(t)
	ctx := context.Background()
	resources := make([]uuid.UUID, 1500)
	for i := range resources {
		resources[i] = store.NewID()
	}
	key := store.NewID()
	_, err := s.Pool.Exec(ctx, `INSERT INTO usage_daily
		(id, day, tenant_id, resource_id, model, provider, call_key_id, status,
		 call_count, request_bytes, response_bytes)
		SELECT gen_random_uuid(), current_date - ((n - 1) / 5), 'usage-query-tenant',
			($1::uuid[])[((n - 1) % 5) + 1], 'model-a', 1, $2, 1, 1, 10, 20
		FROM generate_series(1, 450) AS n`, resources[:5], key)
	if err != nil {
		t.Fatal(err)
	}
	requested := append(resources, resources[0])
	f := store.UsageFilter{TenantID: "usage-query-tenant", ResourceIDs: requested}
	var after *store.UsageCursor
	seen := make(map[uuid.UUID]bool)
	for {
		got, err := store.ListUsageRows(ctx, s.Pool, f, after, 100)
		if err != nil {
			t.Fatal(err)
		}
		var day any
		id := uuid.Nil
		if after != nil {
			day, id = after.Day, after.ID
		}
		rows, err := s.Pool.Query(ctx, `SELECT id FROM usage_daily
			WHERE tenant_id = $1 AND resource_id = ANY($2)
				AND ($3::date IS NULL OR (day, id) < ($3::date, $4::uuid))
			ORDER BY day DESC, id DESC LIMIT 100`, f.TenantID, f.ResourceIDs, day, id)
		if err != nil {
			t.Fatal(err)
		}
		var want []uuid.UUID
		for rows.Next() {
			var id uuid.UUID
			if err := rows.Scan(&id); err != nil {
				rows.Close()
				t.Fatal(err)
			}
			want = append(want, id)
		}
		if err := rows.Err(); err != nil {
			rows.Close()
			t.Fatal(err)
		}
		rows.Close()
		if len(got) != len(want) {
			t.Fatalf("page after %+v: got %d rows, want %d", after, len(got), len(want))
		}
		for i, row := range got {
			if row.ID != want[i] || seen[row.ID] {
				t.Fatalf("page after %+v: row %d = %s, want %s; duplicate=%t", after, i, row.ID, want[i], seen[row.ID])
			}
			seen[row.ID] = true
		}
		if len(got) < 100 {
			break
		}
		last := got[len(got)-1]
		after = &store.UsageCursor{Day: last.Day, ID: last.ID}
	}
	if len(seen) != 450 {
		t.Fatalf("listed %d usage rows, want 450", len(seen))
	}
}

func timePointer(v time.Time) *time.Time { return &v }

func referenceCallRecordIDs(t *testing.T, ctx context.Context, s *store.Store, f store.UsageFilter, before uuid.UUID, limit int) []uuid.UUID {
	t.Helper()
	models, providers, statuses := f.Models, make([]int16, len(f.Providers)), make([]int16, len(f.Statuses))
	if models == nil {
		models = []string{}
	}
	for i, p := range f.Providers {
		providers[i] = int16(p)
	}
	for i, status := range f.Statuses {
		statuses[i] = int16(status)
	}
	keys := f.CallKeyIDs
	if keys == nil {
		keys = []uuid.UUID{}
	}
	rows, err := s.Pool.Query(ctx, `SELECT id FROM call_records
		WHERE tenant_id = $1 AND resource_id = ANY($2)
			AND (cardinality($3::text[]) = 0 OR model = ANY($3))
			AND (cardinality($4::smallint[]) = 0 OR provider = ANY($4))
			AND (cardinality($5::uuid[]) = 0 OR call_key_id = ANY($5))
			AND (cardinality($6::smallint[]) = 0 OR status = ANY($6))
			AND ($7::timestamptz IS NULL OR start_time >= $7)
			AND ($8::timestamptz IS NULL OR start_time < $8)
			AND ($9 = '00000000-0000-0000-0000-000000000000'::uuid OR id < $9)
		ORDER BY id DESC LIMIT $10`, f.TenantID, f.ResourceIDs, models, providers, keys, statuses, f.Start, f.End, before, limit)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var ids []uuid.UUID
	for rows.Next() {
		var id uuid.UUID
		if err := rows.Scan(&id); err != nil {
			t.Fatal(err)
		}
		ids = append(ids, id)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(fmt.Errorf("reference query: %w", err))
	}
	return ids
}
