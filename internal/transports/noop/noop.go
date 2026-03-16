package noop

import (
	"context"
	"sync"

	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/model"
)

type Transport struct {
	name  string
	buses []string
}

func New(name string, buses []string) *Transport {
	return &Transport{name: name, buses: buses}
}

func (t *Transport) Name() string { return t.name }

func (t *Transport) Run(ctx context.Context, api core.RuntimeAPI) error {
	var wg sync.WaitGroup
	for _, busID := range t.buses {
		sub, err := api.Subscribe(ctx, busID, 32)
		if err != nil {
			return err
		}
		wg.Add(1)
		go func(ch <-chan model.Event) {
			defer wg.Done()
			for range ch {
			}
		}(sub)
	}
	<-ctx.Done()
	wg.Wait()
	return nil
}
