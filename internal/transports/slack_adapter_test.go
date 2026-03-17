package transports

import (
	"bytes"
	"context"
	"errors"
	"log/slog"
	"sync"
	"testing"
	"time"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/model"
	"github.com/nemored/pipo/internal/store"
	"github.com/nemored/pipo/internal/telemetry"
)

type fakeSlackRuntime struct {
	mu           sync.Mutex
	connectErrs  []error
	sessionErrs  []error
	connectCalls int
	sessionCalls int
}

func (f *fakeSlackRuntime) connect(context.Context) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.connectCalls++
	if len(f.connectErrs) == 0 {
		return nil
	}
	err := f.connectErrs[0]
	f.connectErrs = f.connectErrs[1:]
	return err
}

func (f *fakeSlackRuntime) runSession(context.Context, core.RuntimeAPI, map[string]string, map[string]string, int) error {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.sessionCalls++
	if len(f.sessionErrs) == 0 {
		return nil
	}
	err := f.sessionErrs[0]
	f.sessionErrs = f.sessionErrs[1:]
	return err
}

func (f *fakeSlackRuntime) close() {}

type noopRuntimeAPI struct{}

func (noopRuntimeAPI) Publish(string, model.Event) error { return nil }
func (noopRuntimeAPI) Subscribe(context.Context, string, int) (<-chan model.Event, error) {
	ch := make(chan model.Event)
	close(ch)
	return ch, nil
}

func TestSlackAdapterReconnectOnTransientFailure(t *testing.T) {
	fake := &fakeSlackRuntime{sessionErrs: []error{errReconnect, nil}}
	prev := slackRuntimeFactory
	slackRuntimeFactory = func(config.Transport, *slog.Logger, *store.SQLiteStore) slackAdapterRuntime { return fake }
	defer func() { slackRuntimeFactory = prev }()

	adapter := buildSlack(0, config.Transport{ChannelMapping: map[string]string{"main": "C1"}}, nil, nil, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 4*time.Second)
	defer cancel()
	if err := adapter.Run(ctx, noopRuntimeAPI{}); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}
	if fake.connectCalls != 2 || fake.sessionCalls != 2 {
		t.Fatalf("expected two connect/session calls, got connect=%d session=%d", fake.connectCalls, fake.sessionCalls)
	}
}

func TestSlackAdapterTerminalAuthErrorPropagates(t *testing.T) {
	want := errors.New("invalid auth")
	fake := &fakeSlackRuntime{connectErrs: []error{asTerminal(want)}}
	prev := slackRuntimeFactory
	slackRuntimeFactory = func(config.Transport, *slog.Logger, *store.SQLiteStore) slackAdapterRuntime { return fake }
	defer func() { slackRuntimeFactory = prev }()

	adapter := buildSlack(0, config.Transport{ChannelMapping: map[string]string{"main": "C1"}}, nil, nil, nil)
	err := adapter.Run(context.Background(), noopRuntimeAPI{})
	if !errors.Is(err, want) {
		t.Fatalf("expected terminal auth error, got %v", err)
	}
	if fake.connectCalls != 1 {
		t.Fatalf("expected a single connect attempt, got %d", fake.connectCalls)
	}
}

func TestSlackAdapterLifecycleMetricsAndLogs(t *testing.T) {
	fake := &fakeSlackRuntime{connectErrs: []error{errors.New("temporary dial")}}
	prev := slackRuntimeFactory
	slackRuntimeFactory = func(config.Transport, *slog.Logger, *store.SQLiteStore) slackAdapterRuntime { return fake }
	defer func() { slackRuntimeFactory = prev }()

	var buf bytes.Buffer
	logger := slog.New(slog.NewTextHandler(&buf, nil))
	metrics := &telemetry.Metrics{}
	adapter := buildSlack(0, config.Transport{ChannelMapping: map[string]string{"main": "C1"}}, nil, logger, metrics)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := adapter.Run(ctx, noopRuntimeAPI{}); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	snap := metrics.Snapshot()
	if snap.ConnectAttempts != 2 || snap.ConnectFailures != 1 || snap.ConnectSuccesses != 1 {
		t.Fatalf("unexpected metrics snapshot: %+v", snap)
	}
	logs := buf.String()
	for _, needle := range []string{"transport connect attempt", "transport connect failed", "transport connected"} {
		if !bytes.Contains([]byte(logs), []byte(needle)) {
			t.Fatalf("expected log %q in output: %s", needle, logs)
		}
	}
}
