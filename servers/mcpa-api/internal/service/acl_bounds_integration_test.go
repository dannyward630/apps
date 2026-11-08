// SPDX-License-Identifier: FSL-1.1-ALv2

//go:build integration

package service_test

import (
	"sync"
	"testing"

	"connectrpc.com/connect"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
)

func TestPutAclEntryCapsDistinctPrincipalsUnderResourceLock(t *testing.T) {
	e := newEnv(t)
	owner := newUser()
	resource := e.createResource(owner, store.NewID().String())
	resourceID := resource.GetId()
	user := mcpav1.PrincipalKind_PRINCIPAL_KIND_USER
	invoke := mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE
	manage := mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE
	// The creator has one entry. Bulk seed 998 others to reach 999 before
	// two concurrent management requests compete for the final slot.
	_, err := e.db.Pool.Exec(e.ctx, `INSERT INTO resource_acl
		(resource_id, principal_kind, principal_id, role, created_by_user_id)
		SELECT $1, $2, 'seed-' || n::text, $3, $4 FROM generate_series(1, 998) AS n`,
		resourceID, int16(user), int16(invoke), owner)
	if err != nil {
		t.Fatal(err)
	}
	first, second := newUser(), newUser()
	var wg sync.WaitGroup
	results := make(chan error, 2)
	for _, principalID := range []string{first, second} {
		wg.Add(1)
		go func() {
			defer wg.Done()
			results <- e.putACL(owner, resourceID, principalID, invoke)
		}()
	}
	wg.Wait()
	close(results)
	succeeded, rejected := 0, 0
	for err := range results {
		if err == nil {
			succeeded++
		} else if reason(err) == mcpav1.ErrorReason_ERROR_REASON_FAILED_PRECONDITION {
			rejected++
		} else {
			t.Fatalf("unexpected ACL error: %v", err)
		}
	}
	if succeeded != 1 || rejected != 1 {
		t.Fatalf("at the cap: %d succeeded, %d rejected; want one each", succeeded, rejected)
	}
	var count int
	if err := e.db.Pool.QueryRow(e.ctx, `SELECT count(*) FROM resource_acl WHERE resource_id = $1`, resourceID).Scan(&count); err != nil {
		t.Fatal(err)
	}
	if count != 1000 {
		t.Fatalf("ACL count = %d, want 1000", count)
	}
	winner := first
	if e.userRole(resourceID, winner) == mcpav1.ResourceRole_RESOURCE_ROLE_UNSPECIFIED {
		winner = second
	}
	if err := e.putACL(owner, resourceID, winner, manage); err != nil {
		t.Fatalf("updating an entry at the cap: %v", err)
	}
	if got := e.userRole(resourceID, winner); got != manage {
		t.Fatalf("updated role = %s, want MANAGE", got)
	}

	// A pre-existing over-limit resource remains mutable for existing entries
	// and removals, but new distinct principals remain blocked.
	_, err = e.db.Pool.Exec(e.ctx, `INSERT INTO resource_acl
		(resource_id, principal_kind, principal_id, role, created_by_user_id)
		VALUES ($1, $2, $3, $4, $5)`, resourceID, int16(user), "legacy-over-cap", int16(invoke), owner)
	if err != nil {
		t.Fatal(err)
	}
	if err := e.putACL(owner, resourceID, winner, invoke); err != nil {
		t.Fatalf("updating an over-limit resource: %v", err)
	}
	if err := e.putACL(owner, resourceID, newUser(), invoke); reason(err) != mcpav1.ErrorReason_ERROR_REASON_FAILED_PRECONDITION {
		t.Fatalf("new principal over cap: %v", err)
	}
	_, err = e.svc.RemoveAclEntry(e.as(owner), connect.NewRequest(&mcpav1.RemoveAclEntryRequest{
		ResourceId: resourceID, PrincipalKind: user, PrincipalId: winner,
	}))
	if err != nil {
		t.Fatalf("remove entry from an over-limit resource: %v", err)
	}
}
