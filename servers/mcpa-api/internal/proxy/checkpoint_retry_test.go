// SPDX-License-Identifier: FSL-1.1-ALv2

package proxy

import (
	"context"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"
	"time"

	mcpav1 "github.com/delinoio/apps/protos/mcpa/v1"
)

type timeoutCheckpointSink struct {
	mu        sync.Mutex
	allow     bool
	attempts  []checkpointEntry
	committed chan struct{}
}

func (s *timeoutCheckpointSink) Checkpoint(ctx context.Context, callID string, deltaReq, deltaResp int64, final bool, record CallRecordUpdate) error {
	s.mu.Lock()
	allowed := s.allow
	s.attempts = append(s.attempts, checkpointEntry{
		callID: callID, deltaReq: deltaReq, deltaResp: deltaResp, final: final, committed: allowed, record: record,
	})
	s.mu.Unlock()
	if !allowed {
		<-ctx.Done()
		return ctx.Err()
	}
	select {
	case s.committed <- struct{}{}:
	default:
	}
	return nil
}

func (s *timeoutCheckpointSink) snapshot() []checkpointEntry {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]checkpointEntry(nil), s.attempts...)
}

type checkpointTestClock struct {
	mu  sync.Mutex
	now time.Time
}

func (c *checkpointTestClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *checkpointTestClock) Set(now time.Time) {
	c.mu.Lock()
	c.now = now
	c.mu.Unlock()
}

func TestFinalCheckpointSucceedsBeforeResponseCompletes(t *testing.T) {
	h := newHarness(t)
	up := openAIUpstream(t, "ok")
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	res := h.call(http.MethodPost, "/r/"+resourceID+"/v1/chat/completions", key, chatBody, nil)
	if res.resp.StatusCode != http.StatusOK {
		t.Fatalf("response %d %s", res.resp.StatusCode, res.body)
	}
	committed := h.sink.committed(res.callID)
	if len(committed) != 1 || !committed[0].final || committed[0].record.Status != mcpav1.CallStatus_CALL_STATUS_SUCCEEDED {
		t.Fatalf("final checkpoint was not committed before response: %+v", committed)
	}
}

func TestFailedFinalCheckpointRetriesAfterResponse(t *testing.T) {
	sink := &timeoutCheckpointSink{committed: make(chan struct{}, 1)}
	const checkpointTimeout = 80 * time.Millisecond
	h := newHarness(t, func(o *Options) {
		o.Sink = sink
		o.CheckpointTimeout = checkpointTimeout
	})
	up := openAIUpstream(t, "ok")
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))

	start := time.Now()
	res := h.call(http.MethodPost, "/r/"+resourceID+"/v1/chat/completions", key, chatBody, nil)
	if elapsed := time.Since(start); elapsed >= 3*checkpointTimeout {
		t.Fatalf("response waited %s for final checkpoint retries", elapsed)
	}
	if res.resp.StatusCode != http.StatusOK || !strings.Contains(string(res.body), "ok") {
		t.Fatalf("response %d %s", res.resp.StatusCode, res.body)
	}
	first := sink.snapshot()
	if len(first) != 1 || !first[0].final {
		t.Fatalf("final attempts before response: %+v", first)
	}
	sink.mu.Lock()
	sink.allow = true
	sink.mu.Unlock()
	select {
	case <-sink.committed:
	case <-time.After(2 * time.Second):
		t.Fatal("final checkpoint did not commit after response")
	}
	// The sink reports success before commit updates its state. Closing the
	// engine waits for that background commit to finish.
	if err := h.engine.Close(); err != nil {
		t.Fatal(err)
	}
	attempts := sink.snapshot()
	if len(attempts) != 2 || attempts[0].record.CheckpointSeq != attempts[1].record.CheckpointSeq ||
		attempts[0].deltaReq != attempts[1].deltaReq || attempts[0].deltaResp != attempts[1].deltaResp ||
		attempts[1].record.Status != mcpav1.CallStatus_CALL_STATUS_SUCCEEDED {
		t.Fatalf("retried checkpoint changed: %+v", attempts)
	}
}

func TestFinalCheckpointUsesResponseLatencyAfterPeriodicRetry(t *testing.T) {
	start := time.Unix(1_700_000_000, 0)
	const responseLatency = 250 * time.Millisecond
	clock := &checkpointTestClock{now: start}
	sink := &timeoutCheckpointSink{committed: make(chan struct{}, 8)}
	h := newHarness(t, func(o *Options) {
		o.Sink = sink
		o.Now = clock.Now
		o.CheckpointInterval = 15 * time.Millisecond
		o.CheckpointTimeout = 40 * time.Millisecond
	})
	up := newUpstream(t, func(w http.ResponseWriter, r *http.Request) {
		_, _ = io.ReadAll(r.Body)
		w.Header().Set("Content-Type", "text/event-stream")
		_, _ = io.WriteString(w, openAIChunk("m1", "first ", false))
		w.(http.Flusher).Flush()
		time.Sleep(100 * time.Millisecond)
		_, _ = io.WriteString(w, openAIChunk("m1", "last", true))
		_, _ = io.WriteString(w, "data: [DONE]\n\n")
		clock.Set(start.Add(responseLatency))
	})
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	res := h.call(http.MethodPost, "/r/"+resourceID+"/v1/chat/completions", key, streamBody("m1"), nil)
	if res.resp.StatusCode != http.StatusOK || !strings.Contains(string(res.body), "last") {
		t.Fatalf("response %d %s", res.resp.StatusCode, res.body)
	}
	failed := sink.snapshot()
	periodic := make([]checkpointEntry, 0, len(failed))
	for _, attempt := range failed {
		if !attempt.final {
			periodic = append(periodic, attempt)
		}
	}
	if len(periodic) == 0 {
		t.Fatalf("no periodic checkpoint failed before response completion: %+v", failed)
	}
	clock.Set(start.Add(responseLatency + time.Hour))
	sink.mu.Lock()
	sink.allow = true
	sink.mu.Unlock()

	deadline := time.After(3 * time.Second)
	var attempts []checkpointEntry
	for {
		attempts = sink.snapshot()
		committedFinal := false
		for _, attempt := range attempts {
			if attempt.callID == res.callID && attempt.final && attempt.committed {
				committedFinal = true
				break
			}
		}
		if committedFinal {
			break
		}
		select {
		case <-sink.committed:
		case <-deadline:
			t.Fatal("delayed final checkpoint did not commit")
		}
	}

	var periodicSequence int64
	var periodicReq, periodicResp int64
	for _, attempt := range attempts {
		if attempt.final {
			continue
		}
		if periodicSequence == 0 {
			periodicSequence = attempt.record.CheckpointSeq
			periodicReq, periodicResp = attempt.deltaReq, attempt.deltaResp
			continue
		}
		if attempt.record.CheckpointSeq != periodicSequence || attempt.deltaReq != periodicReq || attempt.deltaResp != periodicResp {
			t.Fatalf("pending periodic checkpoint changed across retries: %+v", periodic)
		}
	}
	finals := make([]checkpointEntry, 0, 1)
	var finalSequence int64
	for _, attempt := range attempts {
		if attempt.callID != res.callID || !attempt.final {
			continue
		}
		if finalSequence == 0 {
			finalSequence = attempt.record.CheckpointSeq
		} else if attempt.record.CheckpointSeq != finalSequence {
			t.Fatalf("final checkpoint sequence changed across retries: %+v", attempts)
		}
		if attempt.committed {
			finals = append(finals, attempt)
		}
	}
	if len(finals) != 1 {
		t.Fatalf("committed final checkpoints = %d, want 1: %+v", len(finals), attempts)
	}
	final := finals[0]
	if final.record.Latency != responseLatency {
		t.Fatalf("final latency = %s, want response latency %s after delayed record construction", final.record.Latency, responseLatency)
	}
	if final.record.CheckpointSeq <= periodicSequence || final.record.Status != mcpav1.CallStatus_CALL_STATUS_SUCCEEDED {
		t.Fatalf("final record does not follow the pending periodic checkpoint: %+v", final)
	}
}

func TestFailedFinalCheckpointStopsOnEngineClose(t *testing.T) {
	sink := &timeoutCheckpointSink{committed: make(chan struct{}, 1)}
	h := newHarness(t, func(o *Options) {
		o.Sink = sink
		o.CheckpointTimeout = 30 * time.Millisecond
	})
	up := openAIUpstream(t, "ok")
	resourceID := newResourceID()
	key := h.addKey(resourceID)
	h.addCredential(compatCredential(resourceID, up.URL(), "sk", "m1"))
	h.call(http.MethodPost, "/r/"+resourceID+"/v1/chat/completions", key, chatBody, nil)

	deadline := time.Now().Add(2 * time.Second)
	for len(sink.snapshot()) < 2 && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	if len(sink.snapshot()) < 2 {
		t.Fatal("background retry did not start")
	}
	if err := h.engine.Close(); err != nil {
		t.Fatal(err)
	}
	count := len(sink.snapshot())
	time.Sleep(150 * time.Millisecond)
	if got := len(sink.snapshot()); got != count {
		t.Fatalf("retry continued after engine close: %d to %d", count, got)
	}
}
