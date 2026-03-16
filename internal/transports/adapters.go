package transports

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/model"
	"github.com/nemored/pipo/internal/store"
)

var errReconnect = errors.New("reconnect requested")

// transportAdapter captures parity-focused lifecycle and mapping behavior.
type transportAdapter struct {
	name            string
	transportID     int
	channelToRemote map[string]string
	remoteToChannel map[string]string
	buses           []string
	store           *store.SQLiteStore
	connectFn       func(context.Context) error
	sessionFn       func(context.Context, core.RuntimeAPI) error
}

func (t *transportAdapter) Name() string { return t.name }

func (t *transportAdapter) Run(ctx context.Context, api core.RuntimeAPI) error {
	backoff := time.Second
	for {
		if err := t.connectFn(ctx); err != nil {
			if ctx.Err() != nil {
				return nil
			}
			if !sleepWithContext(ctx, backoff) {
				return nil
			}
			backoff = minDuration(backoff*2, 30*time.Second)
			continue
		}
		backoff = time.Second
		err := t.sessionFn(ctx, api)
		if ctx.Err() != nil {
			return nil
		}
		if err == nil {
			return nil
		}
		if errors.Is(err, errReconnect) {
			if !sleepWithContext(ctx, time.Second) {
				return nil
			}
			continue
		}
		return err
	}
}

func minDuration(a, b time.Duration) time.Duration {
	if a < b {
		return a
	}
	return b
}

func sleepWithContext(ctx context.Context, d time.Duration) bool {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return false
	case <-t.C:
		return true
	}
}

func baseAdapter(kind string, idx int, tc config.Transport, s *store.SQLiteStore) *transportAdapter {
	remoteToChannel := make(map[string]string, len(tc.ChannelMapping))
	buses := make([]string, 0, len(tc.ChannelMapping))
	for busID, remoteID := range tc.ChannelMapping {
		buses = append(buses, busID)
		remoteToChannel[remoteID] = busID
	}
	return &transportAdapter{
		name:            fmt.Sprintf("%s[%d]", kind, idx),
		transportID:     idx,
		channelToRemote: cloneMap(tc.ChannelMapping),
		remoteToChannel: remoteToChannel,
		buses:           buses,
		store:           s,
		connectFn:       func(context.Context) error { return nil },
		sessionFn:       consumeOnlySession(buses),
	}
}

func cloneMap(in map[string]string) map[string]string {
	out := make(map[string]string, len(in))
	for k, v := range in {
		out[k] = v
	}
	return out
}

func consumeOnlySession(buses []string) func(context.Context, core.RuntimeAPI) error {
	return func(ctx context.Context, api core.RuntimeAPI) error {
		var wg sync.WaitGroup
		for _, busID := range buses {
			sub, err := api.Subscribe(ctx, busID, 64)
			if err != nil {
				return err
			}
			wg.Add(1)
			go func(ch <-chan model.Event) {
				defer wg.Done()
				for {
					select {
					case <-ctx.Done():
						return
					case _, ok := <-ch:
						if !ok {
							return
						}
					}
				}
			}(sub)
		}
		<-ctx.Done()
		wg.Wait()
		return nil
	}
}

func buildSlack(idx int, tc config.Transport, s *store.SQLiteStore) core.Transport {
	t := baseAdapter("Slack", idx, tc, s)
	t.connectFn = func(context.Context) error { return nil }
	return t
}

func buildDiscord(idx int, tc config.Transport, s *store.SQLiteStore) core.Transport {
	t := baseAdapter("Discord", idx, tc, s)
	t.connectFn = func(context.Context) error { return nil }
	return t
}

func buildIRC(idx int, tc config.Transport, s *store.SQLiteStore) core.Transport {
	t := baseAdapter("IRC", idx, tc, s)
	t.connectFn = func(context.Context) error { return nil }
	return t
}

func buildMumble(idx int, tc config.Transport, s *store.SQLiteStore) core.Transport {
	t := baseAdapter("Mumble", idx, tc, s)
	t.connectFn = func(context.Context) error { return nil }
	return t
}

func buildRachni(idx int, tc config.Transport, s *store.SQLiteStore) core.Transport {
	t := baseAdapter("Rachni", idx, tc, s)
	t.buses = append([]string(nil), tc.Buses...)
	t.connectFn = func(context.Context) error { return nil }
	return t
}

type notImplementedTransport struct{ name string }

func (t notImplementedTransport) Name() string { return t.name }
func (t notImplementedTransport) Run(context.Context, core.RuntimeAPI) error {
	return errors.New("minecraft transport not implemented")
}
