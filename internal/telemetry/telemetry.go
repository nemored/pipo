package telemetry

import (
	"encoding/json"
	"log/slog"
	"os"
	"sync/atomic"
)

type Metrics struct {
	connectAttempts    atomic.Int64
	connectSuccesses   atomic.Int64
	connectFailures    atomic.Int64
	queuePressureDrops atomic.Int64
	forwardingFailures atomic.Int64
	dbErrors           atomic.Int64
}

type Snapshot struct {
	ConnectAttempts    int64 `json:"connect_attempts"`
	ConnectSuccesses   int64 `json:"connect_successes"`
	ConnectFailures    int64 `json:"connect_failures"`
	QueuePressureDrops int64 `json:"queue_pressure_drops"`
	ForwardingFailures int64 `json:"forwarding_failures"`
	DBErrors           int64 `json:"db_errors"`
}

func NewJSONLogger() *slog.Logger {
	return slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelInfo}))
}

func (m *Metrics) IncConnectAttempt() { m.connectAttempts.Add(1) }
func (m *Metrics) IncConnectSuccess() { m.connectSuccesses.Add(1) }
func (m *Metrics) IncConnectFailure() { m.connectFailures.Add(1) }
func (m *Metrics) IncQueuePressure()  { m.queuePressureDrops.Add(1) }
func (m *Metrics) IncForwardingFail() { m.forwardingFailures.Add(1) }
func (m *Metrics) IncDBError()        { m.dbErrors.Add(1) }
func (m *Metrics) MarshalJSON() ([]byte, error) {
	return json.Marshal(m.Snapshot())
}

func (m *Metrics) Snapshot() Snapshot {
	if m == nil {
		return Snapshot{}
	}
	return Snapshot{
		ConnectAttempts:    m.connectAttempts.Load(),
		ConnectSuccesses:   m.connectSuccesses.Load(),
		ConnectFailures:    m.connectFailures.Load(),
		QueuePressureDrops: m.queuePressureDrops.Load(),
		ForwardingFailures: m.forwardingFailures.Load(),
		DBErrors:           m.dbErrors.Load(),
	}
}
