package core

import (
	"context"
	"fmt"
	"sync"

	"github.com/nemored/pipo/internal/model"
)

type RuntimeAPI interface {
	Publish(busID string, event model.Event) error
	Subscribe(ctx context.Context, busID string, buffer int) (<-chan model.Event, error)
}

type Transport interface {
	Name() string
	Run(ctx context.Context, api RuntimeAPI) error
}

type Runtime struct {
	broker     *Broker
	transports []Transport
}

func NewRuntime(busIDs []string, transports []Transport) *Runtime {
	return &Runtime{broker: NewBroker(busIDs), transports: transports}
}

func (r *Runtime) Publish(busID string, event model.Event) error {
	return r.broker.Publish(busID, event)
}

func (r *Runtime) Subscribe(ctx context.Context, busID string, buffer int) (<-chan model.Event, error) {
	return r.broker.Subscribe(ctx, busID, buffer)
}

func (r *Runtime) Run(ctx context.Context) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	errCh := make(chan error, len(r.transports))
	var wg sync.WaitGroup

	for _, transport := range r.transports {
		transport := transport
		wg.Add(1)
		go func() {
			defer wg.Done()
			if err := transport.Run(ctx, r); err != nil {
				errCh <- fmt.Errorf("transport %s: %w", transport.Name(), err)
				cancel()
			}
		}()
	}

	done := make(chan struct{})
	go func() {
		wg.Wait()
		close(done)
	}()

	select {
	case <-ctx.Done():
		<-done
		select {
		case err := <-errCh:
			return err
		default:
			return nil
		}
	case <-done:
		select {
		case err := <-errCh:
			return err
		default:
			return nil
		}
	}
}
