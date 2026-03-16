package core

import (
	"context"
	"errors"
	"fmt"
	"sync"

	"github.com/nemored/pipo/internal/model"
)

var ErrUnknownBus = errors.New("unknown bus")

type Broker struct {
	mu    sync.RWMutex
	buses map[string]*bus
}

type bus struct {
	id          string
	subscribers map[chan model.Event]struct{}
}

func NewBroker(busIDs []string) *Broker {
	buses := make(map[string]*bus, len(busIDs))
	for _, id := range busIDs {
		buses[id] = &bus{id: id, subscribers: map[chan model.Event]struct{}{}}
	}
	return &Broker{buses: buses}
}

func (b *Broker) Publish(busID string, event model.Event) error {
	b.mu.RLock()
	bus, ok := b.buses[busID]
	if !ok {
		b.mu.RUnlock()
		return fmt.Errorf("%w: %s", ErrUnknownBus, busID)
	}
	for ch := range bus.subscribers {
		select {
		case ch <- event:
		default:
			// Drop when subscriber is saturated to avoid global stalls.
		}
	}
	b.mu.RUnlock()
	return nil
}

func (b *Broker) Subscribe(ctx context.Context, busID string, buffer int) (<-chan model.Event, error) {
	b.mu.Lock()
	bus, ok := b.buses[busID]
	if !ok {
		b.mu.Unlock()
		return nil, fmt.Errorf("%w: %s", ErrUnknownBus, busID)
	}
	ch := make(chan model.Event, buffer)
	bus.subscribers[ch] = struct{}{}
	b.mu.Unlock()

	go func() {
		<-ctx.Done()
		b.mu.Lock()
		if _, ok := bus.subscribers[ch]; ok {
			delete(bus.subscribers, ch)
			close(ch)
		}
		b.mu.Unlock()
	}()

	return ch, nil
}
