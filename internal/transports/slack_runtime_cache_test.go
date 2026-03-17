package transports

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"

	"github.com/nemored/pipo/internal/model"
	"github.com/nemored/pipo/internal/store"
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

func TestSlackRuntimeInboundThreadNormalization(t *testing.T) {
	rt := &slackRuntime{userDisplayName: map[string]string{"U1": "Ali"}, channelMeta: map[string]slackChannelMeta{"C1": {ID: "C1", Name: "general"}}}
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"C1": "main"}

	rt.handleEventsAPI([]byte(`{"type":"message","user":"U1","channel":"C1","text":"root","ts":"1700.1","thread_ts":"1700.1"}`), api, mapping, 1)
	rt.handleEventsAPI([]byte(`{"type":"message","user":"U1","channel":"C1","text":"reply","ts":"1700.2","thread_ts":"1700.1"}`), api, mapping, 1)

	events := api.events["main"]
	if len(events) != 2 {
		t.Fatalf("expected 2 events, got %d", len(events))
	}
	if events[0].Thread == nil || events[0].Thread.SlackThreadTS == nil || *events[0].Thread.SlackThreadTS != "1700.1" {
		t.Fatalf("expected root thread ts to be captured, got %+v", events[0].Thread)
	}
	if events[1].Thread == nil || events[1].Thread.SlackThreadTS == nil || *events[1].Thread.SlackThreadTS != "1700.1" {
		t.Fatalf("expected reply to reference root thread ts, got %+v", events[1].Thread)
	}
}

func TestBuildSlackOutboundPayloadKinds(t *testing.T) {
	msg := "hello"
	name := "BridgeBot"
	avatar := "https://example.invalid/a.png"
	threadTS := "1700.1"
	pipoID := int64(42)
	cases := []struct {
		name       string
		ev         model.Event
		wantText   string
		wantOK     bool
		wantBot    bool
		wantMeta   bool
		wantThread bool
	}{
		{name: "text", ev: model.Event{Kind: model.EventText, Message: &msg}, wantText: "hello", wantOK: true},
		{name: "action", ev: model.Event{Kind: model.EventAction, Message: &msg}, wantText: "/me hello", wantOK: true},
		{name: "bot", ev: model.Event{Kind: model.EventBot, Message: &msg, Username: &name, AvatarURL: &avatar}, wantText: "hello", wantOK: true, wantBot: true},
		{name: "metadata", ev: model.Event{Kind: model.EventText, Message: &msg, PipoID: &pipoID, Thread: &model.ThreadRef{SlackThreadTS: &threadTS}, Attachments: []model.Attachment{{ID: 1}}, Metadata: map[string]string{"k": "v"}}, wantText: "hello", wantOK: true, wantMeta: true, wantThread: true},
		{name: "unsupported kind", ev: model.Event{Kind: model.EventDelete, Message: &msg}, wantOK: false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			payload, ok := buildSlackOutboundPayload(tc.ev, "C1")
			if ok != tc.wantOK {
				t.Fatalf("ok mismatch: got %v want %v", ok, tc.wantOK)
			}
			if !tc.wantOK {
				return
			}
			if payload["channel"] != "C1" || payload["text"] != tc.wantText {
				t.Fatalf("unexpected payload basics: %+v", payload)
			}
			if tc.wantBot {
				if payload["username"] != name || payload["icon_url"] != avatar {
					t.Fatalf("expected bot fields, payload=%+v", payload)
				}
			}
			if tc.wantThread {
				if payload["thread_ts"] != threadTS {
					t.Fatalf("expected thread_ts, payload=%+v", payload)
				}
			}
			_, hasMeta := payload["metadata"]
			if hasMeta != tc.wantMeta {
				t.Fatalf("metadata presence mismatch: got %v want %v payload=%+v", hasMeta, tc.wantMeta, payload)
			}
		})
	}
}

func TestSlackRuntimeForwardOutboundRecordsSlackReference(t *testing.T) {
	ctx := context.Background()
	sqlite, err := store.OpenSQLite(ctx, ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer sqlite.Close()

	var seen map[string]any
	postCount := 0
	rt := &slackRuntime{botToken: "xoxb-test", store: sqlite, http: &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		if req.URL.Path != "/api/chat.postMessage" {
			t.Fatalf("unexpected request path: %s", req.URL.Path)
		}
		defer req.Body.Close()
		if err := json.NewDecoder(req.Body).Decode(&seen); err != nil {
			t.Fatalf("decode body: %v", err)
		}
		postCount++
		if postCount == 1 {
			return jsonResponse(`{"ok":true,"ts":"1700000000.100"}`), nil
		}
		return jsonResponse(`{"ok":true,"ts":"1700000000.101"}`), nil
	})}}

	msg := "hello"
	id := int64(7)
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil { // creates id=1; ensure explicit pipo id exists for update path
		t.Fatalf("insert allocated id: %v", err)
	}
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil {
		t.Fatalf("insert allocated id: %v", err)
	}
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil {
		t.Fatalf("insert allocated id: %v", err)
	}
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil {
		t.Fatalf("insert allocated id: %v", err)
	}
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil {
		t.Fatalf("insert allocated id: %v", err)
	}
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil {
		t.Fatalf("insert allocated id: %v", err)
	}
	if _, err := sqlite.InsertAllocatedID(ctx); err != nil {
		t.Fatalf("insert allocated id: %v", err)
	}

	ev := model.Event{Kind: model.EventAction, Message: &msg, Sender: 2, PipoID: &id}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 1, ev); err != nil {
		t.Fatalf("forwardOutboundEvent error: %v", err)
	}
	if seen["text"] != "/me hello" {
		t.Fatalf("expected action text format, got %+v", seen)
	}
	gotSlack, err := sqlite.SelectSlackByID(ctx, id)
	if err != nil || gotSlack == nil || *gotSlack != "1700000000.100" {
		t.Fatalf("expected updated slack id for pipo id: got=%v err=%v", gotSlack, err)
	}

	msg2 := "plain"
	ev2 := model.Event{Kind: model.EventText, Message: &msg2, Sender: 2}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 1, ev2); err != nil {
		t.Fatalf("forwardOutboundEvent text error: %v", err)
	}
	pipo, err := sqlite.SelectIDBySlack(ctx, "1700000000.101")
	if err != nil || pipo == nil {
		t.Fatalf("expected inserted slack mapping for non-pipo outbound, got id=%v err=%v", pipo, err)
	}
	if *pipo == id {
		t.Fatalf("expected insert path to allocate a new pipo id, got existing id %d", *pipo)
	}
}

func TestSlackRuntimeForwardOutboundCrossTransportThreadTranslation(t *testing.T) {
	ctx := context.Background()
	sqlite, err := store.OpenSQLite(ctx, ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer sqlite.Close()

	discordThreadID := uint64(1234)
	pipoID, err := sqlite.InsertOrReplaceDiscord(ctx, discordThreadID)
	if err != nil {
		t.Fatalf("insert discord mapping: %v", err)
	}

	msg := "reply"
	var seen []map[string]any
	rt := &slackRuntime{botToken: "xoxb-test", store: sqlite, http: &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		if req.URL.Path != "/api/chat.postMessage" {
			t.Fatalf("unexpected request path: %s", req.URL.Path)
		}
		defer req.Body.Close()
		decoded := map[string]any{}
		if err := json.NewDecoder(req.Body).Decode(&decoded); err != nil {
			t.Fatalf("decode body: %v", err)
		}
		seen = append(seen, decoded)
		switch len(seen) {
		case 1:
			return jsonResponse(`{"ok":true,"ts":"1700000000.200"}`), nil
		case 2:
			return jsonResponse(`{"ok":true,"ts":"1700000000.201"}`), nil
		default:
			t.Fatalf("unexpected post count: %d", len(seen))
			return nil, nil
		}
	})}}

	evRoot := model.Event{Kind: model.EventText, Message: &msg, Sender: 2, Thread: &model.ThreadRef{DiscordThread: &discordThreadID}}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 1, evRoot); err != nil {
		t.Fatalf("forward root event failed: %v", err)
	}
	if len(seen) != 1 {
		t.Fatalf("expected only root post call, got %d", len(seen))
	}
	if _, has := seen[0]["thread_ts"]; has {
		t.Fatalf("root post should not set thread_ts: %+v", seen[0])
	}
	storedRootTS, err := sqlite.SelectSlackByID(ctx, pipoID)
	if err != nil || storedRootTS == nil || *storedRootTS != "1700000000.200" {
		t.Fatalf("expected mapped thread root ts, got=%v err=%v", storedRootTS, err)
	}

	evReply := model.Event{Kind: model.EventText, Message: &msg, Sender: 2, Thread: &model.ThreadRef{DiscordThread: &discordThreadID}}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 1, evReply); err != nil {
		t.Fatalf("forward reply event failed: %v", err)
	}
	if len(seen) != 2 {
		t.Fatalf("expected second post call for reply, got %d", len(seen))
	}
	if seen[1]["thread_ts"] != "1700000000.200" {
		t.Fatalf("expected reply thread_ts to use mapped root ts, got %+v", seen[1])
	}
}

func TestSlackRuntimeInboundMutationEventsResolvePipoID(t *testing.T) {
	ctx := context.Background()
	sqlite, err := store.OpenSQLite(ctx, ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer sqlite.Close()
	id, err := sqlite.InsertOrReplaceSlack(ctx, "1700000000.100")
	if err != nil {
		t.Fatalf("insert slack mapping: %v", err)
	}

	rt := &slackRuntime{store: sqlite, userDisplayName: map[string]string{"U1": "Alice"}, channelMeta: map[string]slackChannelMeta{"C1": {ID: "C1", Name: "general"}}}
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"C1": "main"}

	rt.handleEventsAPI([]byte(`{"type":"message","subtype":"message_changed","channel":"C1","message":{"ts":"1700000000.100","user":"U1","text":"edited"}}`), api, mapping, 7)
	rt.handleEventsAPI([]byte(`{"type":"message","subtype":"message_deleted","channel":"C1","deleted_ts":"1700000000.100"}`), api, mapping, 7)
	rt.handleEventsAPI([]byte(`{"type":"reaction_added","reaction":"thumbsup","item":{"type":"message","channel":"C1","ts":"1700000000.100"}}`), api, mapping, 7)
	rt.handleEventsAPI([]byte(`{"type":"reaction_removed","reaction":"thumbsup","item":{"type":"message","channel":"C1","ts":"1700000000.100"}}`), api, mapping, 7)

	events := api.events["main"]
	if len(events) != 4 {
		t.Fatalf("expected 4 events, got %d", len(events))
	}
	if events[0].Kind != model.EventText || !events[0].IsEdit || events[0].PipoID == nil || *events[0].PipoID != id {
		t.Fatalf("unexpected edit event: %+v", events[0])
	}
	if events[1].Kind != model.EventDelete || events[1].PipoID == nil || *events[1].PipoID != id {
		t.Fatalf("unexpected delete event: %+v", events[1])
	}
	if events[2].Kind != model.EventReaction || events[2].Emoji == nil || *events[2].Emoji != "thumbsup" || events[2].Remove {
		t.Fatalf("unexpected reaction add event: %+v", events[2])
	}
	if events[3].Kind != model.EventReaction || !events[3].Remove {
		t.Fatalf("unexpected reaction remove event: %+v", events[3])
	}
}

func TestSlackRuntimeInboundMutationEventsMissingMappingNoop(t *testing.T) {
	rt := &slackRuntime{userDisplayName: map[string]string{"U1": "Alice"}, channelMeta: map[string]slackChannelMeta{"C1": {ID: "C1", Name: "general"}}}
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"C1": "main"}

	rt.handleEventsAPI([]byte(`{"type":"message","subtype":"message_deleted","channel":"C1","deleted_ts":"1700000000.999"}`), api, mapping, 7)
	rt.handleEventsAPI([]byte(`{"type":"reaction_added","reaction":"thumbsup","item":{"type":"message","channel":"C1","ts":"1700000000.999"}}`), api, mapping, 7)

	if got := len(api.events["main"]); got != 0 {
		t.Fatalf("expected no published events for missing mapping, got %d", got)
	}
}

func TestSlackRuntimeOutboundMutationsAndStaleTargets(t *testing.T) {
	ctx := context.Background()
	sqlite, err := store.OpenSQLite(ctx, ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer sqlite.Close()
	id, err := sqlite.InsertOrReplaceSlack(ctx, "1700000000.100")
	if err != nil {
		t.Fatalf("insert slack mapping: %v", err)
	}
	msg := "rewritten"
	emoji := "thumbsup"

	var paths []string
	rt := &slackRuntime{botToken: "xoxb-test", store: sqlite, http: &http.Client{Transport: roundTripFunc(func(req *http.Request) (*http.Response, error) {
		paths = append(paths, req.URL.Path)
		switch req.URL.Path {
		case "/api/chat.update":
			return jsonResponse(`{"ok":true,"ts":"1700000000.101"}`), nil
		case "/api/chat.delete":
			return jsonResponse(`{"ok":true}`), nil
		case "/api/reactions.add":
			return jsonResponse(`{"ok":false,"error":"message_not_found"}`), nil
		case "/api/reactions.remove":
			return jsonResponse(`{"ok":false,"error":"no_reaction"}`), nil
		default:
			t.Fatalf("unexpected request path: %s", req.URL.Path)
			return nil, nil
		}
	})}}

	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 99, model.Event{Sender: 1, IsEdit: true, Kind: model.EventText, PipoID: &id, Message: &msg}); err != nil {
		t.Fatalf("edit forward failed: %v", err)
	}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 99, model.Event{Sender: 1, Kind: model.EventDelete, PipoID: &id}); err != nil {
		t.Fatalf("delete forward failed: %v", err)
	}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 99, model.Event{Sender: 1, Kind: model.EventReaction, PipoID: &id, Emoji: &emoji}); err != nil {
		t.Fatalf("reaction add stale should be no-op: %v", err)
	}
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 99, model.Event{Sender: 1, Kind: model.EventReaction, PipoID: &id, Emoji: &emoji, Remove: true}); err != nil {
		t.Fatalf("reaction remove stale should be no-op: %v", err)
	}

	if len(paths) != 4 {
		t.Fatalf("expected 4 API calls, got %d (%v)", len(paths), paths)
	}
	updatedSlack, err := sqlite.SelectSlackByID(ctx, id)
	if err != nil || updatedSlack == nil || *updatedSlack != "1700000000.101" {
		t.Fatalf("expected updated slack mapping after edit, got=%v err=%v", updatedSlack, err)
	}

	paths = nil
	missing := int64(id + 100)
	if err := rt.forwardOutboundEvent("main", map[string]string{"main": "C1"}, 99, model.Event{Sender: 1, Kind: model.EventDelete, PipoID: &missing}); err != nil {
		t.Fatalf("missing mapping should be no-op: %v", err)
	}
	if len(paths) != 0 {
		t.Fatalf("expected no API call for missing mapping, got %v", paths)
	}
}
