package drivers

import (
	"context"
	"testing"
	"time"
)

func TestSlackDriverRetryAndEventCapture(t *testing.T) {
	capture := NewEventCapture()
	d := NewSlackDriver(
		RetryPolicy{MaxAttempts: 3, BaseDelay: time.Millisecond, MaxDelay: 2 * time.Millisecond},
		TimeWindow{},
		capture.Hook,
	)

	_, err := d.SendMessage(context.Background(), MessageInput{
		ChannelID: "C1",
		Text:      "hello",
		StepID:    "slack-eventual",
	})
	if err != nil {
		t.Fatalf("send message: %v", err)
	}

	records := capture.Records()
	if len(records) != 2 {
		t.Fatalf("expected two records (retry + success), got %d", len(records))
	}
	if records[0].Err == "" {
		t.Fatalf("first record should capture eventual-consistency error")
	}
	if records[1].Err != "" {
		t.Fatalf("second record should succeed, got err=%q", records[1].Err)
	}
}

func TestFetchRemoteIDsByStep(t *testing.T) {
	d := NewIRCDriver(RetryPolicy{MaxAttempts: 1}, TimeWindow{}, nil)
	ctx := context.Background()
	_, err := d.SendMessage(ctx, MessageInput{ChannelID: "#general", StepID: "s1", Text: "msg"})
	if err != nil {
		t.Fatalf("send: %v", err)
	}
	ids, err := d.FetchRemoteIDs(ctx, "s1")
	if err != nil {
		t.Fatalf("fetch ids: %v", err)
	}
	if ids.MessageID == "" {
		t.Fatalf("expected message id")
	}
}

func TestTimeWindowBlocksCalls(t *testing.T) {
	d := NewDiscordDriver(
		RetryPolicy{MaxAttempts: 2, BaseDelay: time.Millisecond, MaxDelay: time.Millisecond},
		TimeWindow{End: time.Now().Add(-time.Second)},
		nil,
	)
	_, err := d.SendMessage(context.Background(), MessageInput{ChannelID: "1", StepID: "s", Text: "x"})
	if err == nil {
		t.Fatalf("expected time-window rejection")
	}
}
