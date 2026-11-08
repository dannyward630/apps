// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"context"
	"net/http"
	"time"

	coreauth "github.com/router-for-me/CLIProxyAPI/v6/sdk/cliproxy/auth"
)

// modelSupportCooldownHook undoes CLIProxyAPI v6.10.9's unconditional 12-hour
// model suspension after a provider 400/422 model-support response. The SDK
// applies it despite DisableCooling. Remove this hook when the SDK honors
// DisableCooling for that branch. It runs after MarkResult releases the manager
// lock and suspends the registry model; the original provider response and
// single-attempt call record are already fixed before this callback.
type modelSupportCooldownHook struct {
	coreauth.NoopHook
	rt *resourceRuntime
}

func (h *modelSupportCooldownHook) OnResult(ctx context.Context, result coreauth.Result) {
	if result.Error == nil || result.Model == "" {
		return
	}
	status := result.Error.StatusCode()
	if status != http.StatusBadRequest && status != http.StatusUnprocessableEntity {
		return
	}

	// Serialize with credential replacement and removal. A concurrent result
	// can replace this model state before the callback obtains the lock.
	h.rt.mu.Lock()
	defer h.rt.mu.Unlock()
	auth, ok := h.rt.manager.GetByID(result.AuthID)
	if !ok || auth == nil || auth.Disabled || auth.Status == coreauth.StatusDisabled {
		return
	}
	state := auth.ModelStates[result.Model]
	if state == nil || state.LastError == nil || state.LastError.Message != result.Error.Message {
		return
	}
	// Only the SDK model-support branch assigns a 12-hour retry time to
	// these statuses. Ordinary 400/422 errors have no retry time.
	if !state.NextRetryAfter.After(time.Now().Add(11 * time.Hour)) {
		return
	}
	// MarkResult's success path resets the model and resumes the global
	// registry under the SDK's own locks. Manager.Update cannot be used here:
	// v6.10.9 passes its mutable auth object to the scheduler and races with
	// another MarkResult. This synthetic SDK success affects its internal
	// health counters, but does not make another provider attempt or alter
	// the HTTP response, call record, or mcpa metrics.
	h.rt.manager.MarkResult(ctx, coreauth.Result{AuthID: result.AuthID, Provider: result.Provider, Model: result.Model, Success: true})
}
