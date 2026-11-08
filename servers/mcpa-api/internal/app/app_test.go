// SPDX-License-Identifier: FSL-1.1-ALv2

package app

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"testing"
	"time"
)

func listen(t *testing.T) net.Listener {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	return ln
}

func quietApp() *App {
	return &App{Log: slog.New(slog.NewTextHandler(io.Discard, nil))}
}

func TestServeStopsOnContextAndDrainsWorkers(t *testing.T) {
	a := quietApp()
	stopped := make(chan struct{})
	a.AddWorker(Worker{Name: "loop", Run: func(ctx context.Context) error {
		<-ctx.Done()
		close(stopped)
		return ctx.Err()
	}})
	ln := listen(t)
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- a.serve(ctx, ln, http.NotFoundHandler()) }()

	resp, err := http.Get("http://" + ln.Addr().String() + "/")
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusNotFound {
		t.Fatalf("status = %d", resp.StatusCode)
	}

	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("serve returned %v after cancellation", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("serve did not return")
	}
	select {
	case <-stopped:
	default:
		t.Fatal("worker was not drained before serve returned")
	}
}

func TestServeReturnsWorkerFailure(t *testing.T) {
	a := quietApp()
	boom := errors.New("boom")
	closed := make(chan struct{})
	a.AddShutdownHook(func(context.Context) error { close(closed); return nil })
	a.AddWorker(
		Worker{Name: "failing", Run: func(context.Context) error { return boom }},
		Worker{Name: "loop", Run: func(ctx context.Context) error { <-ctx.Done(); return nil }},
	)
	done := make(chan error, 1)
	go func() { done <- a.serve(context.Background(), listen(t), http.NotFoundHandler()) }()
	select {
	case err := <-done:
		if !errors.Is(err, boom) {
			t.Fatalf("serve returned %v, want the worker error", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("a failing worker did not stop serve")
	}
	select {
	case <-closed:
	default:
		t.Fatal("shutdown hook did not run after worker failure")
	}
}

func TestServeReturnsListenerFailure(t *testing.T) {
	a := quietApp()
	ln := listen(t)
	ln.Close()
	done := make(chan error, 1)
	go func() { done <- a.serve(context.Background(), ln, http.NotFoundHandler()) }()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("serve returned nil for a closed listener")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("a failing listener did not stop serve")
	}
}

func TestServeKeepsRequestContextsDuringDrain(t *testing.T) {
	a := quietApp()
	started := make(chan struct{})
	release := make(chan struct{})
	ctxErr := make(chan error, 1)
	hookRan := make(chan struct{})
	a.AddShutdownHook(func(context.Context) error { close(hookRan); return nil })
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		select {
		case <-release:
		case <-r.Context().Done():
		}
		ctxErr <- r.Context().Err()
		w.WriteHeader(http.StatusNoContent)
	})
	ln := listen(t)
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- a.serve(ctx, ln, handler) }()

	resp := make(chan *http.Response, 1)
	go func() {
		r, err := http.Get("http://" + ln.Addr().String() + "/")
		if err != nil {
			t.Errorf("in-flight request failed: %v", err)
			resp <- nil
			return
		}
		r.Body.Close()
		resp <- r
	}()
	<-started

	// Ending the serve context starts the drain; the in-flight request keeps
	// a live context and completes.
	cancel()
	select {
	case err := <-ctxErr:
		t.Fatalf("the request context ended with the serve context: %v", err)
	case <-time.After(200 * time.Millisecond):
	}
	select {
	case <-hookRan:
		t.Fatal("shutdown hook ran before the request drained")
	default:
	}
	close(release)
	if err := <-ctxErr; err != nil {
		t.Fatalf("the request context ended during the drain: %v", err)
	}
	if r := <-resp; r == nil || r.StatusCode != http.StatusNoContent {
		t.Fatalf("in-flight request response = %v", r)
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("serve returned %v after the drain", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("serve did not return after the drain")
	}
	select {
	case <-hookRan:
	default:
		t.Fatal("shutdown hook did not run after the request drained")
	}
}

func TestServeWorkerFailureStopsActiveRequest(t *testing.T) {
	a := quietApp()
	started := make(chan struct{})
	fail := make(chan struct{})
	boom := errors.New("worker failed")
	a.AddWorker(Worker{Name: "failing", Run: func(context.Context) error {
		<-fail
		return boom
	}})
	a.AddShutdownHook(func(context.Context) error { return nil })
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		<-r.Context().Done()
	})
	ln := listen(t)
	done := make(chan error, 1)
	go func() { done <- a.serve(context.Background(), ln, handler) }()
	requestDone := make(chan struct{})
	go func() {
		resp, err := http.Get("http://" + ln.Addr().String() + "/")
		if err == nil {
			resp.Body.Close()
		}
		close(requestDone)
	}()
	select {
	case <-started:
	case <-time.After(5 * time.Second):
		t.Fatal("request did not start")
	}
	close(fail)
	select {
	case err := <-done:
		if !errors.Is(err, boom) {
			t.Fatalf("serve returned %v, want worker failure", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("worker failure waited for the active request")
	}
	select {
	case <-requestDone:
	case <-time.After(2 * time.Second):
		t.Fatal("active request did not stop after worker failure")
	}
}

func TestServeShutdownHookReceivesDeadlineContext(t *testing.T) {
	a := quietApp()
	called := make(chan context.Context, 1)
	a.AddShutdownHook(func(ctx context.Context) error {
		called <- ctx
		return nil
	})

	ln := listen(t)
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- a.serve(ctx, ln, http.NotFoundHandler()) }()
	cancel()

	select {
	case got := <-called:
		if _, ok := got.Deadline(); !ok {
			t.Fatal("shutdown hook context has no deadline")
		}
	case <-time.After(5 * time.Second):
		t.Fatal("shutdown hook was not called")
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("serve returned %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("serve did not return after the shutdown hook")
	}
}

func TestServeKeepsDrainWorkerUntilRequestsFinish(t *testing.T) {
	a := quietApp()
	workerStarted := make(chan struct{})
	workerStopped := make(chan struct{})
	a.AddWorker(Worker{
		Name:                   "drain-worker",
		KeepRunningDuringDrain: true,
		Run: func(ctx context.Context) error {
			close(workerStarted)
			<-ctx.Done()
			close(workerStopped)
			return ctx.Err()
		},
	})
	requestStarted := make(chan struct{})
	releaseRequest := make(chan struct{})
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(requestStarted)
		<-releaseRequest
		w.WriteHeader(http.StatusNoContent)
	})
	ln := listen(t)
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- a.serve(ctx, ln, handler) }()
	select {
	case <-workerStarted:
	case <-time.After(5 * time.Second):
		t.Fatal("drain worker did not start")
	}
	response := make(chan *http.Response, 1)
	go func() {
		resp, err := http.Get("http://" + ln.Addr().String() + "/")
		if err != nil {
			t.Errorf("request failed: %v", err)
			return
		}
		resp.Body.Close()
		response <- resp
	}()
	select {
	case <-requestStarted:
	case <-time.After(5 * time.Second):
		t.Fatal("request did not start")
	}
	cancel()
	select {
	case <-workerStopped:
		t.Fatal("drain worker stopped before the request finished")
	case <-time.After(100 * time.Millisecond):
	}
	close(releaseRequest)
	select {
	case resp := <-response:
		if resp.StatusCode != http.StatusNoContent {
			t.Fatalf("response status = %d", resp.StatusCode)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("request did not finish")
	}
	select {
	case <-workerStopped:
	case <-time.After(5 * time.Second):
		t.Fatal("drain worker did not stop after the request finished")
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("serve returned %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("serve did not return")
	}
}
