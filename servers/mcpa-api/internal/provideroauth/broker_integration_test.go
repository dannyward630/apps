// SPDX-License-Identifier: FSL-1.1-ALv2

//go:build integration

package provideroauth

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/jackc/pgx/v5"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/apperr"
	"github.com/delinoio/apps/servers/mcpa-api/internal/auth"
	"github.com/delinoio/apps/servers/mcpa-api/internal/crypto"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store/storetest"
)

func TestStartProviderOAuthRechecksManage(t *testing.T) {
	ctx := context.Background()
	db := storetest.Open(t)
	keys, err := crypto.NewKeyring(map[string][]byte{"test": make([]byte, 32)}, "test")
	if err != nil {
		t.Fatal(err)
	}
	fake := newFakeProvider(t)
	b := testBroker(t, fake)
	b.db, b.keys = db, keys
	p := auth.NewUserPrincipal(mcpav1.ActorKind_ACTOR_KIND_CONSOLE, "caller", "caller", nil)

	newResource := func(name string) store.Resource {
		t.Helper()
		res := store.Resource{ID: store.NewID(), TenantID: "oauth-role-lock-test", Name: name, CreatedByUserID: "owner",
			LoadBalancing: mcpav1.LoadBalancing_LOAD_BALANCING_ROUND_ROBIN}
		if err := db.InTx(ctx, func(tx pgx.Tx) error {
			var err error
			res, err = store.InsertResource(ctx, tx, res)
			if err != nil {
				return err
			}
			for _, user := range []string{"owner", "caller"} {
				_, err = store.PutACL(ctx, tx, store.ACLEntry{ResourceID: res.ID,
					PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_USER, PrincipalID: user,
					Role: mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE, CreatedByUserID: "owner"})
				if err != nil {
					return err
				}
			}
			return nil
		}); err != nil {
			t.Fatal(err)
		}
		return res
	}
	changeRole := func(res store.Resource, role mcpav1.ResourceRole) {
		t.Helper()
		if err := db.InTx(ctx, func(tx pgx.Tx) error {
			if _, err := store.LockResource(ctx, tx, res.ID); err != nil {
				return err
			}
			if role == mcpav1.ResourceRole_RESOURCE_ROLE_UNSPECIFIED {
				_, err := store.RemoveACL(ctx, tx, res.ID, mcpav1.PrincipalKind_PRINCIPAL_KIND_USER, "caller")
				return err
			}
			_, err := store.PutACL(ctx, tx, store.ACLEntry{ResourceID: res.ID,
				PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_USER, PrincipalID: "caller",
				Role: role, CreatedByUserID: "owner"})
			return err
		}); err != nil {
			t.Fatal(err)
		}
	}
	assertState := func(res store.Resource, wantFlows int) {
		t.Helper()
		var flows, credentials int
		if err := db.Pool.QueryRow(ctx, `SELECT count(*) FROM provider_oauth_flows WHERE resource_id = $1`, res.ID).Scan(&flows); err != nil {
			t.Fatal(err)
		}
		if err := db.Pool.QueryRow(ctx, `SELECT count(*) FROM credentials WHERE resource_id = $1`, res.ID).Scan(&credentials); err != nil {
			t.Fatal(err)
		}
		if flows != wantFlows || credentials != 0 {
			t.Fatalf("flows = %d, credentials = %d; want %d flows and no credential", flows, credentials, wantFlows)
		}
	}
	start := func(res store.Resource, provider mcpav1.Provider) (*mcpav1.ProviderOAuthFlow, error) {
		return b.StartProviderOAuth(ctx, p, res, &mcpav1.StartProviderOAuthRequest{Provider: provider, Name: "test"})
	}
	assertReason := func(err error, want mcpav1.ErrorReason) {
		t.Helper()
		var appErr *apperr.Error
		if !errors.As(err, &appErr) || appErr.Reason != want {
			t.Fatalf("error = %v, want %s", err, want)
		}
	}

	for _, tc := range []struct {
		name string
		role mcpav1.ResourceRole
		want mcpav1.ErrorReason
	}{
		{"revoked", mcpav1.ResourceRole_RESOURCE_ROLE_UNSPECIFIED, mcpav1.ErrorReason_ERROR_REASON_NOT_FOUND},
		{"downgraded", mcpav1.ResourceRole_RESOURCE_ROLE_INVOKE, mcpav1.ErrorReason_ERROR_REASON_PERMISSION_DENIED},
	} {
		t.Run(tc.name, func(t *testing.T) {
			res := newResource(tc.name)
			changeRole(res, tc.role)
			_, err := start(res, mcpav1.Provider_PROVIDER_CLAUDE)
			assertReason(err, tc.want)
			assertState(res, 0)
		})
	}

	res := newResource("current-manager")
	flow, err := start(res, mcpav1.Provider_PROVIDER_CLAUDE)
	if err != nil || flow.GetKind() != mcpav1.OauthFlowKind_OAUTH_FLOW_KIND_AUTHORIZATION_CODE || flow.GetAuthorizationUrl() == "" {
		t.Fatalf("authorization-code flow = %v, %v", flow, err)
	}
	assertState(res, 1)

	fake.handle("/kimi/device", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{"device_code": "test-device", "user_code": "KIMI-1",
			"verification_uri": "https://www.kimi.com/device", "expires_in": 600, "interval": 5})
	})
	res = newResource("current-device-manager")
	flow, err = start(res, mcpav1.Provider_PROVIDER_KIMI)
	if err != nil || flow.GetKind() != mcpav1.OauthFlowKind_OAUTH_FLOW_KIND_DEVICE_CODE || flow.GetUserCode() == "" {
		t.Fatalf("device flow = %v, %v", flow, err)
	}
	assertState(res, 1)
	stored, err := store.GetOAuthFlow(ctx, db.Pool, res.ID, uuid.MustParse(flow.GetId()))
	if err != nil {
		t.Fatal(err)
	}
	changeRole(res, mcpav1.ResourceRole_RESOURCE_ROLE_UNSPECIFIED)
	_, err = b.register(ctx, p, stored, token{payload: map[string]any{
		"type": "kimi", "access_token": "test-access", "refresh_token": "test-refresh", "device_id": "test-device",
	}})
	assertReason(err, mcpav1.ErrorReason_ERROR_REASON_NOT_FOUND)
	stored, err = store.GetOAuthFlow(ctx, db.Pool, res.ID, stored.ID)
	if err != nil || stored.State != mcpav1.OauthFlowState_OAUTH_FLOW_STATE_FAILED {
		t.Fatalf("flow after revoked registration = %v, %v", stored.State, err)
	}
	assertState(res, 1)

	res = newResource("revoked-before-device")
	changeRole(res, mcpav1.ResourceRole_RESOURCE_ROLE_UNSPECIFIED)
	before := fake.count("/kimi/device")
	_, err = start(res, mcpav1.Provider_PROVIDER_KIMI)
	assertReason(err, mcpav1.ErrorReason_ERROR_REASON_NOT_FOUND)
	assertState(res, 0)
	if got := fake.count("/kimi/device"); got != before {
		t.Fatalf("provider calls = %d, want %d", got, before)
	}

	entered, release := make(chan struct{}), make(chan struct{})
	fake.handle("/kimi/device", func(w http.ResponseWriter, _ *http.Request) {
		close(entered)
		<-release
		writeJSON(w, http.StatusOK, map[string]any{"device_code": "test-device", "user_code": "KIMI-2",
			"verification_uri": "https://www.kimi.com/device", "expires_in": 600, "interval": 5})
	})
	res = newResource("revoked-during-device")
	type startResult struct {
		flow *mcpav1.ProviderOAuthFlow
		err  error
	}
	result := make(chan startResult, 1)
	go func() {
		flow, err := start(res, mcpav1.Provider_PROVIDER_KIMI)
		result <- startResult{flow, err}
	}()
	select {
	case <-entered:
	case <-time.After(5 * time.Second):
		t.Fatal("provider call did not start")
	}
	changeRole(res, mcpav1.ResourceRole_RESOURCE_ROLE_UNSPECIFIED)
	close(release)
	select {
	case got := <-result:
		assertReason(got.err, mcpav1.ErrorReason_ERROR_REASON_NOT_FOUND)
		if got.flow != nil {
			t.Fatalf("revoked caller received flow %v", got.flow)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("provider call did not finish")
	}
	assertState(res, 0)

	res = newResource("deleted")
	if err := db.InTx(ctx, func(tx pgx.Tx) error {
		if _, err := store.LockResource(ctx, tx, res.ID); err != nil {
			return err
		}
		_, err := store.DeleteResource(ctx, tx, res.ID)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	_, err = start(res, mcpav1.Provider_PROVIDER_CLAUDE)
	assertReason(err, mcpav1.ErrorReason_ERROR_REASON_FAILED_PRECONDITION)
	assertState(res, 0)
}

func TestKimiSlowDownPersistsNextPoll(t *testing.T) {
	ctx := context.Background()
	db := storetest.Open(t)
	res := store.Resource{
		ID: store.NewID(), TenantID: "kimi-poll-test", Name: "Kimi poll", CreatedByUserID: "owner",
		LoadBalancing: mcpav1.LoadBalancing_LOAD_BALANCING_ROUND_ROBIN,
	}
	if err := db.InTx(ctx, func(tx pgx.Tx) error {
		var err error
		res, err = store.InsertResource(ctx, tx, res)
		if err != nil {
			return err
		}
		_, err = store.PutACL(ctx, tx, store.ACLEntry{ResourceID: res.ID,
			PrincipalKind: mcpav1.PrincipalKind_PRINCIPAL_KIND_USER, PrincipalID: "owner",
			Role: mcpav1.ResourceRole_RESOURCE_ROLE_MANAGE, CreatedByUserID: "owner"})
		return err
	}); err != nil {
		t.Fatal(err)
	}
	keys, err := crypto.NewKeyring(map[string][]byte{"test": make([]byte, 32)}, "test")
	if err != nil {
		t.Fatal(err)
	}
	fake := newFakeProvider(t)
	fake.handle("/kimi/device", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{
			"device_code": "test-device", "user_code": "KIMI-1", "verification_uri": "https://www.kimi.com/device",
			"expires_in": 600, "interval": 5,
		})
	})
	answers := []string{"authorization_pending", "slow_down", "expired_token"}
	fake.handle("/kimi/token", func(w http.ResponseWriter, _ *http.Request) {
		index := fake.count("/kimi/token") - 1
		writeJSON(w, http.StatusBadRequest, map[string]string{"error": answers[index]})
	})
	now := fixedNow
	b := testBroker(t, fake)
	b.db = db
	b.keys = keys
	b.now = func() time.Time { return now }
	p := auth.NewUserPrincipal(mcpav1.ActorKind_ACTOR_KIND_CONSOLE, "owner", "owner", nil)
	flow, err := b.StartProviderOAuth(ctx, p, res, &mcpav1.StartProviderOAuthRequest{
		Provider: mcpav1.Provider_PROVIDER_KIMI, Name: "test",
	})
	if err != nil {
		t.Fatal(err)
	}
	flowID := uuid.MustParse(flow.GetId())
	get := func() *mcpav1.ProviderOAuthFlow {
		t.Helper()
		got, err := b.GetProviderOAuthFlow(ctx, p, res, &mcpav1.GetProviderOAuthFlowRequest{FlowId: flow.GetId()})
		if err != nil {
			t.Fatal(err)
		}
		return got
	}
	secret := func() flowSecret {
		t.Helper()
		var sec flowSecret
		err := db.InTx(ctx, func(tx pgx.Tx) error {
			stored, sealed, err := store.LockOAuthFlow(ctx, tx, res.ID, flowID)
			if err != nil {
				return err
			}
			sec, err = b.open(stored, sealed)
			return err
		})
		if err != nil {
			t.Fatal(err)
		}
		return sec
	}
	assertPending := func(wantCalls int) {
		t.Helper()
		if got := get(); got.GetState() != mcpav1.OauthFlowState_OAUTH_FLOW_STATE_PENDING {
			t.Fatalf("flow state = %v, want PENDING", got.GetState())
		}
		if got := fake.count("/kimi/token"); got != wantCalls {
			t.Fatalf("provider poll count = %d, want %d", got, wantCalls)
		}
	}

	now = now.Add(5 * time.Second)
	assertPending(1)
	if got := secret().NextPollUnixMilli; got != now.Add(5*time.Second).UnixMilli() {
		t.Fatalf("authorization_pending next poll = %d, want %d", got, now.Add(5*time.Second).UnixMilli())
	}
	now = now.Add(5 * time.Second)
	assertPending(2)
	if sec := secret(); sec.IntervalSeconds != 10 || sec.NextPollUnixMilli < now.Add(10*time.Second).UnixMilli() {
		t.Fatalf("slow_down interval = %d, next poll = %d, want at least %d", sec.IntervalSeconds, sec.NextPollUnixMilli, now.Add(10*time.Second).UnixMilli())
	}
	now = now.Add(5 * time.Second)
	assertPending(2)
	now = now.Add(5 * time.Second)
	if got := get(); got.GetState() != mcpav1.OauthFlowState_OAUTH_FLOW_STATE_EXPIRED {
		t.Fatalf("flow state = %v, want EXPIRED", got.GetState())
	}
	if got := get(); got.GetState() != mcpav1.OauthFlowState_OAUTH_FLOW_STATE_EXPIRED || fake.count("/kimi/token") != 3 {
		t.Fatalf("terminal flow state = %v, provider polls = %d", got.GetState(), fake.count("/kimi/token"))
	}
}
