package transports

import (
	"fmt"
	"iter"
	"maps"
	"slices"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/transports/noop"
)

func Build(cfg config.ParsedConfig) ([]core.Transport, error) {
	knownBuses := make(map[string]struct{}, len(cfg.Buses))
	for _, bus := range cfg.Buses {
		knownBuses[bus.ID] = struct{}{}
	}

	out := make([]core.Transport, 0, len(cfg.Transports))
	for idx, tc := range cfg.Transports {
		busIDs := resolveBuses(tc)
		if len(busIDs) == 0 {
			busIDs = slices.Collect(iter.Seq[string](maps.Keys(knownBuses)))
			slices.Sort(busIDs)
		}
		for _, busID := range busIDs {
			if _, ok := knownBuses[busID]; !ok {
				return nil, fmt.Errorf("transport %d (%s) references unknown bus %q", idx, tc.Kind, busID)
			}
		}
		out = append(out, noop.New(fmt.Sprintf("%s[%d]", tc.Kind, idx), busIDs))
	}
	return out, nil
}

func resolveBuses(tc config.Transport) []string {
	if len(tc.Buses) > 0 {
		return slices.Clone(tc.Buses)
	}
	set := map[string]struct{}{}
	for busID := range tc.ChannelMapping {
		set[busID] = struct{}{}
	}
	for busID := range tc.VoiceChannelMapping {
		set[busID] = struct{}{}
	}
	out := slices.Collect(iter.Seq[string](maps.Keys(set)))
	slices.Sort(out)
	return out
}
