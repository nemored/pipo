package transports

import (
	"context"
	"errors"
	"log/slog"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/store"
	"github.com/nemored/pipo/internal/telemetry"
)

type slackAdapterRuntime interface {
	connect(context.Context) error
	runSession(context.Context, core.RuntimeAPI, map[string]string, map[string]string, int) error
	close()
}

var slackRuntimeFactory = func(tc config.Transport, logger *slog.Logger) slackAdapterRuntime {
	return newSlackRuntime(tc, logger)
}

func buildSlack(idx int, tc config.Transport, s *store.SQLiteStore, logger *slog.Logger, m *telemetry.Metrics) core.Transport {
	t := baseAdapter("Slack", idx, tc, s, logger, m)
	rt := slackRuntimeFactory(tc, logger)
	t.connectFn = rt.connect
	t.sessionFn = func(ctx context.Context, api core.RuntimeAPI) error {
		err := rt.runSession(ctx, api, t.remoteToChannel, t.channelToRemote, t.transportID)
		rt.close()
		if errors.Is(err, errReconnect) && logger != nil {
			logger.Info("slack lifecycle phase", "phase", "reconnect", "transport", t.name)
		}
		return err
	}
	return t
}
