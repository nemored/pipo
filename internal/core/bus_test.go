package core

import (
	"context"
	"testing"
	"time"

	"github.com/nemored/pipo/internal/model"
)

func TestBrokerPublishesToBusSubscribers(t *testing.T) {
	b := NewBroker([]string{"alpha", "beta"})
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	sub, err := b.Subscribe(ctx, "alpha", 1)
	if err != nil {
		t.Fatalf("subscribe: %v", err)
	}

	if err := b.Publish("alpha", model.Event{Kind: model.EventText}); err != nil {
		t.Fatalf("publish: %v", err)
	}

	select {
	case ev := <-sub:
		if ev.Kind != model.EventText {
			t.Fatalf("unexpected kind: %s", ev.Kind)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for event")
	}
}
