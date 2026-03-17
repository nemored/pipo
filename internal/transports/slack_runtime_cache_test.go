package transports

import (
	"context"
	"io"
	"net/http"
	"strings"
	"testing"

	"github.com/nemored/pipo/internal/model"
)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) { return f(req) }

func jsonResponse(body string) *http.Response {
	return &http.Response{StatusCode: http.StatusOK, Header: make(http.Header), Body: io.NopCloser(strings.NewReader(body))}
}

func TestSlackRuntimeRefreshCachesHandlesPagination(t *testing.T) {
	rt := &slackRuntime{botToken: "xoxb-test", http: &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		switch req.URL.Path {
		case "/api/conversations.list":
			if req.URL.Query().Get("cursor") == "c-next" {
				return jsonResponse(`{"ok":true,"channels":[{"id":"C2","name":"beta","is_private":true}],"response_metadata":{"next_cursor":""}}`), nil
			}
			return jsonResponse(`{"ok":true,"channels":[{"id":"C1","name":"alpha","is_private":false}],"response_metadata":{"next_cursor":"c-next"}}`), nil
		case "/api/users.list":
			if req.URL.Query().Get("cursor") == "u-next" {
				return jsonResponse(`{"ok":true,"members":[{"id":"U2","name":"bob","profile":{"display_name":"Bobby","real_name":"Bob"}}],"response_metadata":{"next_cursor":""}}`), nil
			}
			return jsonResponse(`{"ok":true,"members":[{"id":"U1","name":"alice","profile":{"display_name":"","real_name":"Alice"}}],"response_metadata":{"next_cursor":"u-next"}}`), nil
		default:
			t.Fatalf("unexpected request path: %s", req.URL.Path)
			return nil, nil
		}
	})}, userDisplayName: map[string]string{}, channelMeta: map[string]slackChannelMeta{}}

	if err := rt.refreshCaches(context.Background()); err != nil {
		t.Fatalf("refreshCaches error: %v", err)
	}

	if got := rt.channelNameForID("C1"); got != "alpha" {
		t.Fatalf("expected alpha, got %q", got)
	}
	if got := rt.channelNameForID("C2"); got != "beta" {
		t.Fatalf("expected beta, got %q", got)
	}
	if got := rt.displayNameForUser("U1"); got != "Alice" {
		t.Fatalf("expected Alice, got %q", got)
	}
	if got := rt.displayNameForUser("U2"); got != "Bobby" {
		t.Fatalf("expected Bobby, got %q", got)
	}
}

func TestSlackRuntimeUnknownFallbacks(t *testing.T) {
	rt := &slackRuntime{userDisplayName: map[string]string{"U1": "Known"}, channelMeta: map[string]slackChannelMeta{"C1": {ID: "C1", Name: "known"}}}

	if got := rt.displayNameForUser("U-missing"); got != "unknown-user:U-missing" {
		t.Fatalf("unexpected user fallback: %q", got)
	}
	if got := rt.channelNameForID("C-missing"); got != "unknown-channel:C-missing" {
		t.Fatalf("unexpected channel fallback: %q", got)
	}
}

func TestSlackRuntimeRefreshPreservesExistingCacheMappings(t *testing.T) {
	rt := &slackRuntime{botToken: "xoxb-test", http: &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		switch req.URL.Path {
		case "/api/conversations.list":
			return jsonResponse(`{"ok":true,"channels":[{"id":"CNEW","name":"new-room","is_private":false}],"response_metadata":{"next_cursor":""}}`), nil
		case "/api/users.list":
			return jsonResponse(`{"ok":true,"members":[{"id":"UNEW","name":"new-user","profile":{"display_name":"New User","real_name":"New User"}}],"response_metadata":{"next_cursor":""}}`), nil
		default:
			t.Fatalf("unexpected request path: %s", req.URL.Path)
			return nil, nil
		}
	})}, userDisplayName: map[string]string{"UOLD": "Old User"}, channelMeta: map[string]slackChannelMeta{"COLD": {ID: "COLD", Name: "legacy-room"}}}

	if err := rt.refreshCaches(context.Background()); err != nil {
		t.Fatalf("refreshCaches error: %v", err)
	}

	if got := rt.channelNameForID("COLD"); got != "legacy-room" {
		t.Fatalf("expected preserved legacy-room, got %q", got)
	}
	if got := rt.channelNameForID("CNEW"); got != "new-room" {
		t.Fatalf("expected new-room, got %q", got)
	}
	if got := rt.displayNameForUser("UOLD"); got != "Old User" {
		t.Fatalf("expected preserved old user, got %q", got)
	}
	if got := rt.displayNameForUser("UNEW"); got != "New User" {
		t.Fatalf("expected new user, got %q", got)
	}
}

func TestSlackRuntimeForwardOutboundUsesChannelMapping(t *testing.T) {
	requests := 0
	rt := &slackRuntime{botToken: "xoxb-test", http: &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		requests++
		if req.URL.Path != "/api/chat.postMessage" {
			t.Fatalf("unexpected request path: %s", req.URL.Path)
		}
		return jsonResponse(`{"ok":true}`), nil
	})}}
	msg := "hello"
	ev := model.Event{Kind: model.EventText, Message: &msg, Sender: 2}
	if err := rt.forwardOutboundEvent("main", map[string]string{}, 1, ev); err != nil {
		t.Fatalf("unexpected error for unmapped bus: %v", err)
	}
	if requests != 0 {
		t.Fatalf("expected no request for unmapped bus, got %d", requests)
	}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 1, ev); err != nil {
		t.Fatalf("forwardOutboundEvent error: %v", err)
	}
	if requests != 1 {
		t.Fatalf("expected exactly one request, got %d", requests)
	}
}

func TestSlackRuntimeEventsAPIUpdatesCacheAndPublishes(t *testing.T) {
	rt := &slackRuntime{userDisplayName: map[string]string{}, channelMeta: map[string]slackChannelMeta{}}
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"C1": "main"}

	rt.handleEventsAPI([]byte(`{"type":"channel_created","channel":{"id":"C1","name":"general","is_private":false}}`), api, mapping, 1)
	rt.handleEventsAPI([]byte(`{"type":"channel_rename","channel":{"id":"C1","name":"all-hands"}}`), api, mapping, 1)
	rt.handleEventsAPI([]byte(`{"type":"user_change","user":{"id":"U1","name":"alice","profile":{"display_name":"Ali","real_name":"Alice"}}}`), api, mapping, 1)
	rt.handleEventsAPI([]byte(`{"type":"message","user":"U1","channel":"C1","text":"hello","ts":"1700.1"}`), api, mapping, 1)

	if got := rt.channelNameForID("C1"); got != "all-hands" {
		t.Fatalf("expected renamed channel, got %q", got)
	}
	if got := rt.displayNameForUser("U1"); got != "Ali" {
		t.Fatalf("expected updated user name, got %q", got)
	}
	events := api.events["main"]
	if len(events) != 1 {
		t.Fatalf("expected one published message event, got %d", len(events))
	}
	if events[0].Username == nil || *events[0].Username != "Ali" {
		t.Fatalf("expected published username Ali, got %+v", events[0].Username)
	}
}
