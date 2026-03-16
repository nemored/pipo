package transports

import (
	"fmt"
	"iter"
	"log/slog"
	"maps"
	"slices"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/store"
	"github.com/nemored/pipo/internal/telemetry"
)

func Build(cfg config.ParsedConfig, db *store.SQLiteStore, logger *slog.Logger, m *telemetry.Metrics) ([]core.Transport, error) {
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
		switch tc.Kind {
		case "Slack":
			out = append(out, buildSlack(idx, tc, db, logger, m))
		case "Discord":
			out = append(out, buildDiscord(idx, tc, db, logger, m))
		case "IRC":
			out = append(out, buildIRC(idx, tc, db, logger, m))
		case "Mumble":
			out = append(out, buildMumble(idx, tc, db, logger, m))
		case "Rachni":
			out = append(out, buildRachni(idx, tc, db, logger, m))
		case "Minecraft":
			out = append(out, notImplementedTransport{name: fmt.Sprintf("%s[%d]", tc.Kind, idx)})
		default:
			return nil, fmt.Errorf("transport %d has unsupported type %q", idx, tc.Kind)
		}
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
