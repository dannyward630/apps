// SPDX-License-Identifier: FSL-1.1-ALv2

//go:build integration

package store_test

import (
	"context"
	"reflect"
	"testing"

	"github.com/jackc/pgx/v5"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store/storetest"
)

func TestListACLForPrincipals(t *testing.T) {
	ctx := context.Background()
	db := storetest.Open(t)
	resource := store.Resource{ID: store.NewID(), TenantID: "acl-principals-test", Name: "acl-principals",
		CreatedByUserID: "owner", LoadBalancing: mcpav1.LoadBalancing_LOAD_BALANCING_ROUND_ROBIN}
	if err := db.InTx(ctx, func(tx pgx.Tx) error {
		var err error
		resource, err = store.InsertResource(ctx, tx, resource)
		if err != nil {
			return err
		}
		for _, entry := range []store.ACLEntry{
			{PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_USER, PrincipalID: "issuer", Role: mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE},
			{PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_USER, PrincipalID: "other-user", Role: mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE},
			{PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_GROUP, PrincipalID: "group-a", Role: mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE},
			{PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_GROUP, PrincipalID: "group-b", Role: mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE},
		} {
			entry.ResourceID, entry.CreatedByUserID = resource.ID, "owner"
			if _, err := store.PutACL(ctx, tx, entry); err != nil {
				return err
			}
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}

	for _, tc := range []struct {
		name, user string
		groups     []string
		want       map[string]mcpav1.ResourceRole
	}{
		{"direct", "issuer", nil, map[string]mcpav1.ResourceRole{"issuer": mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE}},
		{"group", "missing", []string{"group-a"}, map[string]mcpav1.ResourceRole{"group-a": mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE}},
		{"both", "issuer", []string{"group-a", "group-b"}, map[string]mcpav1.ResourceRole{
			"issuer": mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE, "group-a": mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE,
			"group-b": mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE}},
		{"neither", "missing", []string{"other-group"}, map[string]mcpav1.ResourceRole{}},
		{"empty-groups", "issuer", []string{}, map[string]mcpav1.ResourceRole{"issuer": mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			entries, err := store.ListACLForPrincipals(ctx, db.Pool, resource.ID, tc.user, tc.groups)
			if err != nil {
				t.Fatal(err)
			}
			got := map[string]mcpav1.ResourceRole{}
			for _, entry := range entries {
				got[entry.PrincipalID] = entry.Role
			}
			if !reflect.DeepEqual(got, tc.want) {
				t.Fatalf("entries = %v, want %v", got, tc.want)
			}
		})
	}
}
