// SPDX-License-Identifier: FSL-1.1-ALv2

package delcore

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"connectrpc.com/connect"
	"github.com/prometheus/client_golang/prometheus/testutil"
	"golang.org/x/oauth2"
	"google.golang.org/protobuf/types/known/timestamppb"

	delcorev1 "github.com/delinoio/apps/protos/delcore/v1"
	"github.com/delinoio/apps/protos/delcore/v1/delcorev1connect"
	"github.com/delinoio/apps/servers/mcpa-api/internal/obs"
)

// fakeDelcore is a scriptable Delcore server.
type fakeDelcore struct {
	delcorev1connect.UnimplementedDelcoreServiceHandler
	mu             sync.Mutex
	allowed        bool
	down           bool
	authError      connect.Code
	keyErrorReason delcorev1.ErrorReason
	authBlock      bool
	authEntered    chan struct{}
	state          delcorev1.TenantState
	calls          map[string]int
	authHeaders    []string
}

func (f *fakeDelcore) record(name string, h http.Header) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.calls == nil {
		f.calls = map[string]int{}
	}
	f.calls[name]++
	f.authHeaders = append(f.authHeaders, h.Get("Authorization"))
	if f.down {
		return connect.NewError(connect.CodeUnavailable, errors.New("down"))
	}
	return nil
}

func (f *fakeDelcore) count(name string) int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.calls[name]
}

func (f *fakeDelcore) set(fn func(*fakeDelcore)) {
	f.mu.Lock()
	defer f.mu.Unlock()
	fn(f)
}

func (f *fakeDelcore) Authorize(ctx context.Context, req *connect.Request[delcorev1.AuthorizeRequest]) (*connect.Response[delcorev1.AuthorizeResponse], error) {
	if err := f.record("Authorize", req.Header()); err != nil {
		return nil, err
	}
	f.mu.Lock()
	code, block, entered, allowed := f.authError, f.authBlock, f.authEntered, f.allowed
	f.mu.Unlock()
	if code != 0 {
		return nil, connect.NewError(code, errors.New("authorization failed"))
	}
	if block {
		if entered != nil {
			entered <- struct{}{}
		}
		<-ctx.Done()
		return nil, ctx.Err()
	}
	resp := &delcorev1.AuthorizeResponse{Allowed: allowed}
	if !allowed {
		resp.Reasons = []delcorev1.BlockReason{delcorev1.BlockReason_BLOCK_REASON_PERMISSION}
	}
	return connect.NewResponse(resp), nil
}

func (f *fakeDelcore) AuthorizeApiKey(_ context.Context, req *connect.Request[delcorev1.AuthorizeApiKeyRequest]) (*connect.Response[delcorev1.AuthorizeApiKeyResponse], error) {
	if err := f.record("AuthorizeApiKey", req.Header()); err != nil {
		return nil, err
	}
	if req.Msg.GetSecret() != "dlk_good" {
		reason := f.keyErrorReason
		if reason == delcorev1.ErrorReason_ERROR_REASON_UNSPECIFIED {
			reason = delcorev1.ErrorReason_ERROR_REASON_UNAUTHENTICATED
		}
		ce := connect.NewError(connect.CodeUnauthenticated, errors.New("authentication failed"))
		if detail, err := connect.NewErrorDetail(&delcorev1.ErrorDetail{Reason: reason}); err == nil {
			ce.AddDetail(detail)
		}
		return nil, ce
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	return connect.NewResponse(&delcorev1.AuthorizeApiKeyResponse{
		Allowed: f.allowed, KeyId: "k", TenantId: "t", UserId: "u", OwnerKind: delcorev1.ApiKeyOwnerKind_API_KEY_OWNER_KIND_PERSONAL,
	}), nil
}

func (f *fakeDelcore) GetTenantStates(_ context.Context, req *connect.Request[delcorev1.GetTenantStatesRequest]) (*connect.Response[delcorev1.GetTenantStatesResponse], error) {
	if err := f.record("GetTenantStates", req.Header()); err != nil {
		return nil, err
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	var states []*delcorev1.TenantStateEntry
	for _, id := range req.Msg.GetTenantIds() {
		states = append(states, &delcorev1.TenantStateEntry{TenantId: id, State: f.state})
	}
	return connect.NewResponse(&delcorev1.GetTenantStatesResponse{States: states}), nil
}

func (f *fakeDelcore) GetCurrentUser(_ context.Context, req *connect.Request[delcorev1.GetCurrentUserRequest]) (*connect.Response[delcorev1.GetCurrentUserResponse], error) {
	if err := f.record("GetCurrentUser", req.Header()); err != nil {
		return nil, err
	}
	if req.Msg.GetPage().GetAfterId() == "" {
		return connect.NewResponse(&delcorev1.GetCurrentUserResponse{UserId: "user-1", Tenants: []*delcorev1.Tenant{{Id: "t1"}}, NextAfterId: "t1"}), nil
	}
	return connect.NewResponse(&delcorev1.GetCurrentUserResponse{UserId: "user-1", Tenants: []*delcorev1.Tenant{{Id: "t2"}}}), nil
}

func (f *fakeDelcore) ListGroups(_ context.Context, req *connect.Request[delcorev1.ListGroupsRequest]) (*connect.Response[delcorev1.ListGroupsResponse], error) {
	if err := f.record("ListGroups", req.Header()); err != nil {
		return nil, err
	}
	if req.Msg.GetPage().GetAfterId() == "" {
		return connect.NewResponse(&delcorev1.ListGroupsResponse{Groups: []*delcorev1.Group{{Id: "g1", UserIds: []string{"u"}}}, NextAfterId: "g1"}), nil
	}
	return connect.NewResponse(&delcorev1.ListGroupsResponse{Groups: []*delcorev1.Group{{Id: "g2"}}}), nil
}

func (f *fakeDelcore) ListMeterAggregates(_ context.Context, req *connect.Request[delcorev1.ListMeterAggregatesRequest]) (*connect.Response[delcorev1.ListMeterAggregatesResponse], error) {
	if err := f.record("ListMeterAggregates", req.Header()); err != nil {
		return nil, err
	}
	if req.Msg.GetServiceId() != "svc" || req.Msg.GetMeterKey() != "mcpa-traffic-bytes" {
		return nil, connect.NewError(connect.CodeInvalidArgument, errors.New("bad meter"))
	}
	return connect.NewResponse(&delcorev1.ListMeterAggregatesResponse{
		PeriodStart: timestamppb.New(time.Date(2026, 11, 1, 0, 0, 0, 0, time.UTC)),
		PeriodEnd:   timestamppb.New(time.Date(2026, 12, 1, 0, 0, 0, 0, time.UTC)),
	}), nil
}

type clock struct{ t atomic.Int64 }

func (c *clock) now() time.Time          { return time.Unix(0, c.t.Load()) }
func (c *clock) advance(d time.Duration) { c.t.Add(int64(d)) }

func newTestClient(t *testing.T, f *fakeDelcore) (*Client, *clock) {
	t.Helper()
	_, handler := delcorev1connect.NewDelcoreServiceHandler(f)
	srv := httptest.NewServer(handler)
	t.Cleanup(srv.Close)
	clk := &clock{}
	clk.t.Store(time.Date(2026, 11, 1, 12, 0, 0, 0, time.UTC).UnixNano())
	c, err := New(Config{
		BaseURL:     srv.URL,
		ServiceID:   "svc",
		Timeout:     2 * time.Second,
		CacheTTL:    30 * time.Second,
		HTTPClient:  srv.Client(),
		Now:         clk.now,
		TokenSource: oauth2.StaticTokenSource(&oauth2.Token{AccessToken: "m2m-token"}),
	})
	if err != nil {
		t.Fatal(err)
	}
	return c, clk
}

func TestAuthorizeCachesOnlyAllowed(t *testing.T) {
	f := &fakeDelcore{allowed: false}
	c, clk := newTestClient(t, f)
	ctx := context.Background()

	for range 2 {
		d, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, false)
		if err != nil || d.Allowed {
			t.Fatalf("denial: %+v %v", d, err)
		}
	}
	if n := f.count("Authorize"); n != 2 {
		t.Fatalf("denials must not be cached: %d calls", n)
	}

	f.set(func(f *fakeDelcore) { f.allowed = true })
	for range 3 {
		if d, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, false); err != nil || !d.Allowed {
			t.Fatalf("allowed: %+v %v", d, err)
		}
	}
	if n := f.count("Authorize"); n != 3 {
		t.Fatalf("allowed result must be cached: %d calls", n)
	}
	// Another capability or paid flag is a different cache entry.
	if _, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, true); err != nil {
		t.Fatal(err)
	}
	if n := f.count("Authorize"); n != 4 {
		t.Fatalf("paid must not share the cache entry: %d calls", n)
	}

	// After expiry with Delcore down, the call fails closed.
	f.set(func(f *fakeDelcore) { f.down = true })
	if d, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, false); err != nil || !d.Allowed {
		t.Fatalf("within TTL the cached result applies: %+v %v", d, err)
	}
	clk.advance(31 * time.Second)
	d, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, false)
	if err == nil || d.Allowed {
		t.Fatalf("expired entry with Delcore down must fail closed: %+v %v", d, err)
	}
	var de *Error
	if !errors.As(err, &de) || de.Kind != KindUnavailable {
		t.Fatalf("error kind: %v", err)
	}
	// Errors are not cached either.
	f.set(func(f *fakeDelcore) { f.down = false })
	if d, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, false); err != nil || !d.Allowed {
		t.Fatalf("recovery: %+v %v", d, err)
	}
}

func TestM2MTokenOnServiceCallsOnly(t *testing.T) {
	f := &fakeDelcore{allowed: true}
	c, _ := newTestClient(t, f)
	ctx := context.Background()
	if _, err := c.Authorize(ctx, "t", "u", CapabilityCreateResources, false); err != nil {
		t.Fatal(err)
	}
	user, err := c.GetCurrentUser(ctx, "user-token")
	if err != nil {
		t.Fatal(err)
	}
	if user.UserID != "user-1" || len(user.Tenants) != 2 {
		t.Fatalf("all pages: %+v", user)
	}
	if _, err := c.GetCurrentUser(ctx, ""); err == nil {
		t.Fatal("empty user token must not fall back to the service identity")
	}
	groups, err := c.ListGroups(ctx, "user-token", "t")
	if err != nil || len(groups) != 2 {
		t.Fatalf("groups: %v %v", groups, err)
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	want := []string{"Bearer m2m-token", "Bearer user-token", "Bearer user-token", "Bearer user-token", "Bearer user-token"}
	if len(f.authHeaders) != len(want) {
		t.Fatalf("headers: %v", f.authHeaders)
	}
	for i := range want {
		if f.authHeaders[i] != want[i] {
			t.Fatalf("header %d = %q, want %q", i, f.authHeaders[i], want[i])
		}
	}
}

func TestTenantStateCachesOnlyActive(t *testing.T) {
	f := &fakeDelcore{state: delcorev1.TenantState_TENANT_STATE_BLOCKED}
	c, clk := newTestClient(t, f)
	ctx := context.Background()
	for range 2 {
		s, err := c.TenantState(ctx, "t")
		if err != nil || s != delcorev1.TenantState_TENANT_STATE_BLOCKED {
			t.Fatalf("state %v %v", s, err)
		}
	}
	if n := f.count("GetTenantStates"); n != 2 {
		t.Fatalf("non-active states must not be cached: %d", n)
	}
	f.set(func(f *fakeDelcore) { f.state = delcorev1.TenantState_TENANT_STATE_ACTIVE })
	for range 3 {
		if s, err := c.TenantState(ctx, "t"); err != nil || s != delcorev1.TenantState_TENANT_STATE_ACTIVE {
			t.Fatalf("state %v %v", s, err)
		}
	}
	if n := f.count("GetTenantStates"); n != 3 {
		t.Fatalf("active state must be cached: %d", n)
	}
	f.set(func(f *fakeDelcore) { f.down = true })
	clk.advance(30 * time.Second)
	if _, err := c.TenantState(ctx, "t"); err == nil {
		t.Fatal("expired state with Delcore down must fail closed")
	}
}

func TestAuthorizeAPIKey(t *testing.T) {
	f := &fakeDelcore{allowed: true}
	c, _ := newTestClient(t, f)
	ctx := context.Background()
	if _, err := c.AuthorizeAPIKey(ctx, "dlk_bad", CapabilityAccessResources, false); !IsUnauthenticated(err) {
		t.Fatalf("invalid key: %v", err)
	}
	d, err := c.AuthorizeAPIKey(ctx, "dlk_good", CapabilityAccessResources, false)
	if err != nil || !d.Allowed || d.UserID != "u" || d.OwnerKind != delcorev1.ApiKeyOwnerKind_API_KEY_OWNER_KIND_PERSONAL {
		t.Fatalf("decision %+v %v", d, err)
	}
	if _, err := c.AuthorizeAPIKey(ctx, "dlk_good", CapabilityAccessResources, false); err != nil {
		t.Fatal(err)
	}
	if n := f.count("AuthorizeApiKey"); n != 2 {
		t.Fatalf("allowed key result must be cached: %d", n)
	}
}

func TestDelcoreFailureMetrics(t *testing.T) {
	counter := func(m *obs.Metrics, rpc string, kind obs.DelcoreFailureKind) float64 {
		return testutil.ToFloat64(m.DelcoreFailures.WithLabelValues(rpc, string(kind)))
	}
	newClient := func(t *testing.T, f *fakeDelcore) (*Client, *obs.Metrics) {
		t.Helper()
		c, _ := newTestClient(t, f)
		m := obs.NewMetrics()
		c.metrics = m
		return c, m
	}

	t.Run("invalid API keys are denied", func(t *testing.T) {
		c, m := newClient(t, &fakeDelcore{})
		for range 6 {
			if _, err := c.AuthorizeAPIKey(context.Background(), "dlk_bad", CapabilityAccessResources, false); !IsUnauthenticated(err) {
				t.Fatalf("invalid key error: %v", err)
			}
		}
		if got := counter(m, "AuthorizeApiKey", obs.DelcoreDenied); got != 6 {
			t.Fatalf("denied = %v, want 6", got)
		}
		if got := counter(m, "AuthorizeApiKey", obs.DelcoreError) + counter(m, "AuthorizeApiKey", obs.DelcoreUnavailable); got != 0 {
			t.Fatalf("error or unavailable = %v, want 0", got)
		}
	})

	t.Run("service authentication failure is an error", func(t *testing.T) {
		c, m := newClient(t, &fakeDelcore{keyErrorReason: delcorev1.ErrorReason_ERROR_REASON_SERVICE_AUTHENTICATION_FAILED})
		_, err := c.AuthorizeAPIKey(context.Background(), "dlk_bad", CapabilityAccessResources, false)
		if !IsUnauthenticated(err) || ReasonOf(err) != delcorev1.ErrorReason_ERROR_REASON_SERVICE_AUTHENTICATION_FAILED {
			t.Fatalf("service authentication error: %v, reason %s", err, ReasonOf(err))
		}
		if got := counter(m, "AuthorizeApiKey", obs.DelcoreError); got != 1 {
			t.Fatalf("error = %v, want 1", got)
		}
		if got := counter(m, "AuthorizeApiKey", obs.DelcoreDenied) + counter(m, "AuthorizeApiKey", obs.DelcoreUnavailable); got != 0 {
			t.Fatalf("denied or unavailable = %v, want 0", got)
		}
	})

	t.Run("invalid tenant is denied", func(t *testing.T) {
		c, m := newClient(t, &fakeDelcore{authError: connect.CodeInvalidArgument})
		_, err := c.Authorize(context.Background(), "bad-tenant", "u", CapabilityAccessResources, false)
		var de *Error
		if !errors.As(err, &de) || de.Kind != KindRejected || de.Code != connect.CodeInvalidArgument {
			t.Fatalf("invalid tenant error: %v", err)
		}
		if got := counter(m, "Authorize", obs.DelcoreDenied); got != 1 {
			t.Fatalf("denied = %v, want 1", got)
		}
		if got := counter(m, "Authorize", obs.DelcoreError); got != 0 {
			t.Fatalf("error = %v, want 0", got)
		}
	})

	t.Run("non-key unauthenticated remains an error", func(t *testing.T) {
		c, m := newClient(t, &fakeDelcore{authError: connect.CodeUnauthenticated})
		_, err := c.Authorize(context.Background(), "t", "u", CapabilityAccessResources, false)
		if !IsUnauthenticated(err) {
			t.Fatalf("unauthenticated error: %v", err)
		}
		if got := counter(m, "Authorize", obs.DelcoreError); got != 1 {
			t.Fatalf("error = %v, want 1", got)
		}
	})

	t.Run("authorization denial remains denied", func(t *testing.T) {
		c, m := newClient(t, &fakeDelcore{allowed: false})
		decision, err := c.Authorize(context.Background(), "t", "u", CapabilityAccessResources, false)
		if err != nil || decision.Allowed {
			t.Fatalf("denied decision: %+v %v", decision, err)
		}
		if got := counter(m, "Authorize", obs.DelcoreDenied); got != 1 {
			t.Fatalf("denied = %v, want 1", got)
		}
	})

	t.Run("caller cancellation is not a failure", func(t *testing.T) {
		entered := make(chan struct{}, 1)
		c, m := newClient(t, &fakeDelcore{authBlock: true, authEntered: entered})
		ctx, cancel := context.WithCancel(context.Background())
		result := make(chan error, 1)
		go func() {
			_, err := c.Authorize(ctx, "t", "u", CapabilityAccessResources, false)
			result <- err
		}()
		<-entered
		cancel()
		var de *Error
		if err := <-result; !errors.As(err, &de) || de.Kind != KindUnavailable {
			t.Fatalf("canceled call error: %v", err)
		}
		for _, kind := range []obs.DelcoreFailureKind{obs.DelcoreDenied, obs.DelcoreError, obs.DelcoreUnavailable} {
			if got := counter(m, "Authorize", kind); got != 0 {
				t.Fatalf("%s = %v, want 0", kind, got)
			}
		}
	})

	for _, code := range []connect.Code{connect.CodeUnavailable, connect.CodeInternal} {
		t.Run(code.String(), func(t *testing.T) {
			c, m := newClient(t, &fakeDelcore{authError: code})
			_, err := c.Authorize(context.Background(), "t", "u", CapabilityAccessResources, false)
			var de *Error
			if !errors.As(err, &de) || de.Kind != KindUnavailable {
				t.Fatalf("service fault error: %v", err)
			}
			if got := counter(m, "Authorize", obs.DelcoreUnavailable); got != 1 {
				t.Fatalf("unavailable = %v, want 1", got)
			}
		})
	}

	t.Run("client timeout remains unavailable", func(t *testing.T) {
		c, m := newClient(t, &fakeDelcore{authBlock: true})
		c.timeout = 20 * time.Millisecond
		_, err := c.Authorize(context.Background(), "t", "u", CapabilityAccessResources, false)
		var de *Error
		if !errors.As(err, &de) || de.Kind != KindUnavailable {
			t.Fatalf("timeout error: %v", err)
		}
		if got := counter(m, "Authorize", obs.DelcoreUnavailable); got != 1 {
			t.Fatalf("unavailable = %v, want 1", got)
		}
	})
}

func TestBillingPeriod(t *testing.T) {
	f := &fakeDelcore{}
	c, _ := newTestClient(t, f)
	p, err := c.BillingPeriod(context.Background(), "t")
	if err != nil {
		t.Fatal(err)
	}
	if p.Start.Month() != time.November || p.End.Month() != time.December {
		t.Fatalf("period %+v", p)
	}
	if _, err := c.BillingPeriod(context.Background(), "t"); err != nil {
		t.Fatal(err)
	}
	if n := f.count("ListMeterAggregates"); n != 1 {
		t.Fatalf("period must be cached: %d", n)
	}
}
