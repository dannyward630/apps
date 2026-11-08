// SPDX-License-Identifier: FSL-1.1-ALv2

//go:build integration

package provideroauth

import (
	"context"
	"net/http"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/jackc/pgx/v5"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
	"github.com/delinoio/apps/servers/mcpa-api/internal/auth"
	"github.com/delinoio/apps/servers/mcpa-api/internal/crypto"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store"
	"github.com/delinoio/apps/servers/mcpa-api/internal/store/storetest"
)

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
