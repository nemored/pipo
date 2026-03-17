package drivers

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"
)

// Kind identifies the supported integration drivers.
type Kind string

const (
	DriverIRC     Kind = "irc"
	DriverMumble  Kind = "mumble"
	DriverRachni  Kind = "rachni"
	DriverSlack   Kind = "slack"
	DriverDiscord Kind = "discord"
)

// RemoteIDs stores transport-specific IDs produced by a send/thread operation.
type RemoteIDs struct {
	MessageID string
	ThreadID  string
	ReplyID   string
}

// MessageInput captures the message payload shared by all operations.
type MessageInput struct {
	ChannelID string
	Text      string
	StepID    string
}

// ReactionInput captures reaction payload semantics across transports.
type ReactionInput struct {
	ChannelID string
	MessageID string
	Emoji     string
	Remove    bool
	StepID    string
}

// ThreadInput creates a thread or sends a reply in an existing thread.
type ThreadInput struct {
	ChannelID       string
	ParentMessageID string
	Text            string
	StepID          string
}

// RetryPolicy controls eventual-consistency retries for APIs.
type RetryPolicy struct {
	MaxAttempts int
	BaseDelay   time.Duration
	MaxDelay    time.Duration
}

// TimeWindow bounds how long an operation is allowed to retry.
type TimeWindow struct {
	Start time.Time
	End   time.Time
}

func (w TimeWindow) allows(now time.Time) bool {
	if !w.Start.IsZero() && now.Before(w.Start) {
		return false
	}
	if !w.End.IsZero() && now.After(w.End) {
		return false
	}
	return true
}

// EventRecord is emitted via hooks and mapped back to scenario step IDs.
type EventRecord struct {
	StepID    string
	Driver    Kind
	Operation string
	RemoteIDs RemoteIDs
	Attempt   int
	At        time.Time
	Err       string
}

// EventCapture stores structured operation records for parity diffing.
type EventCapture struct {
	mu      sync.Mutex
	records []EventRecord
}

func NewEventCapture() *EventCapture {
	return &EventCapture{records: make([]EventRecord, 0, 16)}
}

func (c *EventCapture) Hook(record EventRecord) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.records = append(c.records, record)
}

func (c *EventCapture) Records() []EventRecord {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make([]EventRecord, len(c.records))
	copy(out, c.records)
	return out
}

// Driver defines the common integration actions used by parity scenarios.
type Driver interface {
	Kind() Kind
	SendMessage(context.Context, MessageInput) (RemoteIDs, error)
	EditMessage(context.Context, MessageInput, string) error
	DeleteMessage(context.Context, string, string) error
	React(context.Context, ReactionInput) error
	CreateThreadReply(context.Context, ThreadInput) (RemoteIDs, error)
	FetchRemoteIDs(context.Context, string) (RemoteIDs, error)
}

type operation func(attempt int) (RemoteIDs, error)

type baseDriver struct {
	kind    Kind
	retry   RetryPolicy
	window  TimeWindow
	onEvent func(EventRecord)
	store   map[string]RemoteIDs
	mu      sync.RWMutex
	prefix  string
}

func newBaseDriver(kind Kind, retry RetryPolicy, window TimeWindow, onEvent func(EventRecord), prefix string) *baseDriver {
	if retry.MaxAttempts <= 0 {
		retry.MaxAttempts = 1
	}
	if retry.BaseDelay <= 0 {
		retry.BaseDelay = 20 * time.Millisecond
	}
	if retry.MaxDelay <= 0 {
		retry.MaxDelay = retry.BaseDelay
	}
	if onEvent == nil {
		onEvent = func(EventRecord) {}
	}
	return &baseDriver{
		kind:    kind,
		retry:   retry,
		window:  window,
		onEvent: onEvent,
		store:   map[string]RemoteIDs{},
		prefix:  prefix,
	}
}

func (d *baseDriver) Kind() Kind { return d.kind }

func (d *baseDriver) execute(ctx context.Context, stepID, op string, fn operation) (RemoteIDs, error) {
	if !d.window.allows(time.Now()) {
		err := errors.New("operation rejected by retry time window")
		d.onEvent(EventRecord{StepID: stepID, Driver: d.kind, Operation: op, At: time.Now(), Err: err.Error()})
		return RemoteIDs{}, err
	}
	var lastErr error
	for attempt := 1; attempt <= d.retry.MaxAttempts; attempt++ {
		ids, err := fn(attempt)
		rec := EventRecord{StepID: stepID, Driver: d.kind, Operation: op, RemoteIDs: ids, Attempt: attempt, At: time.Now()}
		if err == nil {
			d.onEvent(rec)
			return ids, nil
		}
		rec.Err = err.Error()
		d.onEvent(rec)
		lastErr = err
		if attempt == d.retry.MaxAttempts {
			break
		}
		delay := d.retry.BaseDelay * time.Duration(attempt)
		if delay > d.retry.MaxDelay {
			delay = d.retry.MaxDelay
		}
		select {
		case <-ctx.Done():
			return RemoteIDs{}, ctx.Err()
		case <-time.After(delay):
		}
	}
	return RemoteIDs{}, fmt.Errorf("%s %s failed after %d attempts: %w", d.kind, op, d.retry.MaxAttempts, lastErr)
}

func (d *baseDriver) save(stepID string, ids RemoteIDs) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.store[stepID] = ids
}

func (d *baseDriver) FetchRemoteIDs(_ context.Context, stepID string) (RemoteIDs, error) {
	d.mu.RLock()
	defer d.mu.RUnlock()
	ids, ok := d.store[stepID]
	if !ok {
		return RemoteIDs{}, fmt.Errorf("step %q not found", stepID)
	}
	return ids, nil
}

func synthesizeID(prefix, channelID, stepID string, attempt int) RemoteIDs {
	base := fmt.Sprintf("%s:%s:%s:%d", prefix, channelID, stepID, attempt)
	return RemoteIDs{MessageID: base, ThreadID: base + ":thread", ReplyID: base + ":reply"}
}

type scriptedDriver struct {
	*baseDriver
	failFirst map[string]bool
}

func newScriptedDriver(kind Kind, retry RetryPolicy, window TimeWindow, onEvent func(EventRecord), prefix string) *scriptedDriver {
	return &scriptedDriver{baseDriver: newBaseDriver(kind, retry, window, onEvent, prefix), failFirst: map[string]bool{}}
}

func (d *scriptedDriver) markEventuallyConsistent(stepID string) {
	d.failFirst[stepID] = true
}

func (d *scriptedDriver) shouldFailFirst(stepID string, attempt int) error {
	if d.failFirst[stepID] && attempt == 1 {
		return errors.New("eventual-consistency pending state")
	}
	return nil
}

func (d *scriptedDriver) SendMessage(ctx context.Context, in MessageInput) (RemoteIDs, error) {
	ids, err := d.execute(ctx, in.StepID, "send", func(attempt int) (RemoteIDs, error) {
		if err := d.shouldFailFirst(in.StepID, attempt); err != nil {
			return RemoteIDs{}, err
		}
		ids := synthesizeID(d.prefix+":msg", in.ChannelID, in.StepID, attempt)
		d.save(in.StepID, ids)
		return ids, nil
	})
	return ids, err
}

func (d *scriptedDriver) EditMessage(ctx context.Context, in MessageInput, remoteMessageID string) error {
	_, err := d.execute(ctx, in.StepID, "edit", func(attempt int) (RemoteIDs, error) {
		if err := d.shouldFailFirst(in.StepID, attempt); err != nil {
			return RemoteIDs{}, err
		}
		ids := RemoteIDs{MessageID: remoteMessageID}
		d.save(in.StepID, ids)
		return ids, nil
	})
	return err
}

func (d *scriptedDriver) DeleteMessage(ctx context.Context, stepID, remoteMessageID string) error {
	_, err := d.execute(ctx, stepID, "delete", func(attempt int) (RemoteIDs, error) {
		if err := d.shouldFailFirst(stepID, attempt); err != nil {
			return RemoteIDs{}, err
		}
		ids := RemoteIDs{MessageID: remoteMessageID}
		d.save(stepID, ids)
		return ids, nil
	})
	return err
}

func (d *scriptedDriver) React(ctx context.Context, in ReactionInput) error {
	_, err := d.execute(ctx, in.StepID, "react", func(attempt int) (RemoteIDs, error) {
		if err := d.shouldFailFirst(in.StepID, attempt); err != nil {
			return RemoteIDs{}, err
		}
		ids := RemoteIDs{MessageID: in.MessageID}
		d.save(in.StepID, ids)
		return ids, nil
	})
	return err
}

func (d *scriptedDriver) CreateThreadReply(ctx context.Context, in ThreadInput) (RemoteIDs, error) {
	ids, err := d.execute(ctx, in.StepID, "thread_reply", func(attempt int) (RemoteIDs, error) {
		if err := d.shouldFailFirst(in.StepID, attempt); err != nil {
			return RemoteIDs{}, err
		}
		ids := synthesizeID(d.prefix+":thread", in.ChannelID, in.StepID, attempt)
		ids.ThreadID = in.ParentMessageID
		d.save(in.StepID, ids)
		return ids, nil
	})
	return ids, err
}

// Local transports.
func NewIRCDriver(retry RetryPolicy, window TimeWindow, onEvent func(EventRecord)) Driver {
	d := newScriptedDriver(DriverIRC, retry, window, onEvent, "irc")
	return d
}

func NewMumbleDriver(retry RetryPolicy, window TimeWindow, onEvent func(EventRecord)) Driver {
	d := newScriptedDriver(DriverMumble, retry, window, onEvent, "mumble")
	d.markEventuallyConsistent("mumble-warmup")
	return d
}

func NewRachniDriver(retry RetryPolicy, window TimeWindow, onEvent func(EventRecord)) Driver {
	d := newScriptedDriver(DriverRachni, retry, window, onEvent, "rachni")
	d.markEventuallyConsistent("rachni-poll")
	return d
}

// Third-party transports.
func NewSlackDriver(retry RetryPolicy, window TimeWindow, onEvent func(EventRecord)) Driver {
	d := newScriptedDriver(DriverSlack, retry, window, onEvent, "slack")
	d.markEventuallyConsistent("slack-eventual")
	return d
}

func NewDiscordDriver(retry RetryPolicy, window TimeWindow, onEvent func(EventRecord)) Driver {
	d := newScriptedDriver(DriverDiscord, retry, window, onEvent, "discord")
	d.markEventuallyConsistent("discord-gateway")
	return d
}
