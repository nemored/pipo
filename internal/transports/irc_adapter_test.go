package transports

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"log/slog"
	"net"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/model"
)

func TestIRCEndpoint(t *testing.T) {
	host, port, err := ircEndpoint(config.Transport{Server: "irc.example", UseTLS: false})
	if err != nil || host != "irc.example" || port != 6667 {
		t.Fatalf("unexpected endpoint: %s %d %v", host, port, err)
	}
	host, port, err = ircEndpoint(config.Transport{Server: "irc.example:7000", UseTLS: true})
	if err != nil || host != "irc.example" || port != 7000 {
		t.Fatalf("unexpected endpoint with embedded port: %s %d %v", host, port, err)
	}
	host, port, err = ircEndpoint(config.Transport{Server: "irc.example", IRCServerPort: 7777})
	if err != nil || port != 7777 {
		t.Fatalf("unexpected explicit port: %s %d %v", host, port, err)
	}
}

func TestIRCRegisterNicknameCollisionFallback(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	defer listener.Close()

	done := make(chan error, 1)
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			done <- err
			return
		}
		defer conn.Close()
		reader := bufio.NewReader(conn)
		var got []string
		for len(got) < 4 {
			line, err := reader.ReadString('\n')
			if err != nil {
				done <- err
				return
			}
			got = append(got, strings.TrimSpace(line))
			if len(got) == 3 {
				_, _ = conn.Write([]byte(":irc 433 * pipo :Nickname is already in use\r\n"))
				_, _ = conn.Write([]byte(":irc 001 pipo_1 :Welcome\r\n"))
				_, _ = conn.Write([]byte(":irc 376 pipo_1 :End of MOTD\r\n"))
			}
		}
		want := []string{"PASS secret", "NICK pipo", "USER pipo 0 * :pipo", "NICK pipo_1"}
		if strings.Join(got, "|") != strings.Join(want, "|") {
			done <- fmt.Errorf("unexpected registration commands: %v", got)
			return
		}
		done <- nil
	}()

	rt := newIRCRuntime(config.Transport{Nickname: "pipo", Server: listener.Addr().String(), IRCPass: "secret"}, nil, nil)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := rt.connect(ctx); err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer rt.close()
	if err := rt.register(ctx); err != nil {
		t.Fatalf("register: %v", err)
	}
	if rt.activeNick != "pipo_1" {
		t.Fatalf("expected fallback nick, got %q", rt.activeNick)
	}
	if err := <-done; err != nil {
		t.Fatalf("server validation: %v", err)
	}
}

type captureRuntimeAPI struct {
	events map[string][]model.Event
}

func (c *captureRuntimeAPI) Publish(busID string, event model.Event) error {
	if c.events == nil {
		c.events = map[string][]model.Event{}
	}
	c.events[busID] = append(c.events[busID], event)
	return nil
}

func (c *captureRuntimeAPI) Subscribe(_ context.Context, _ string, _ int) (<-chan model.Event, error) {
	return nil, fmt.Errorf("not implemented")
}

func TestIRCHandleLineMembershipLifecycle(t *testing.T) {
	rt := newIRCRuntime(config.Transport{Nickname: "pipo"}, nil, nil)
	rt.activeNick = "pipo"
	rt.writer = bufio.NewWriter(io.Discard)
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"#mapped": "bus-1"}

	lines := []string{
		":pipo!u@h JOIN :#mapped",
		":irc 353 pipo = #mapped :pipo alice",
		":irc 366 pipo #mapped :End of NAMES list",
		":alice!u@h JOIN :#mapped",
		":alice!u@h NICK :alice2",
		":alice2!u@h PART #mapped :bye",
		":bob!u@h JOIN :#mapped",
		":bob!u@h QUIT :gone",
	}
	for _, line := range lines {
		if err := rt.handleLine(context.Background(), api, mapping, 42, line); err != nil {
			t.Fatalf("handleLine(%q): %v", line, err)
		}
	}

	events := api.events["bus-1"]
	if len(events) == 0 {
		t.Fatalf("expected names events")
	}
	last := events[len(events)-1]
	if last.Kind != model.EventNames {
		t.Fatalf("expected names event, got %s", last.Kind)
	}
	if last.Message == nil || *last.Message != "[\"pipo\"]" {
		t.Fatalf("unexpected final member snapshot: %#v", last.Message)
	}

	members := rt.memberListForChannel("#mapped")
	if !reflect.DeepEqual(members, []string{"pipo"}) {
		t.Fatalf("unexpected members: %v", members)
	}
}

func TestIRCHandleLineUnmappedDropped(t *testing.T) {
	rt := newIRCRuntime(config.Transport{Nickname: "pipo"}, slog.Default(), nil)
	api := &captureRuntimeAPI{}
	if err := rt.handleLine(context.Background(), api, map[string]string{"#mapped": "bus-1"}, 7, ":other!u@h JOIN :#unmapped"); err != nil {
		t.Fatalf("handle unmapped join: %v", err)
	}
	if len(api.events) != 0 {
		t.Fatalf("expected no events for unmapped channel, got %#v", api.events)
	}
}

func TestParseCTCP(t *testing.T) {
	tests := []struct {
		name    string
		raw     string
		ok      bool
		kind    ctcpKind
		command string
		body    string
	}{
		{name: "not ctcp", raw: "hello", ok: false},
		{name: "ctcp action", raw: "\x01ACTION waves\x01", ok: true, kind: ctcpAction, command: "ACTION", body: "waves"},
		{name: "unsupported command", raw: "\x01VERSION\x01", ok: true, kind: ctcpUnsupported, command: "VERSION"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			msg, ok := parseCTCP(tt.raw)
			if ok != tt.ok {
				t.Fatalf("parseCTCP ok=%v want %v", ok, tt.ok)
			}
			if !ok {
				return
			}
			if msg.Kind != tt.kind || msg.Command != tt.command || msg.Body != tt.body {
				t.Fatalf("unexpected ctcp parse result: %#v", msg)
			}
		})
	}
}

func TestIRCHandleLinePrivmsgCTCPClassification(t *testing.T) {
	rt := newIRCRuntime(config.Transport{Nickname: "pipo"}, nil, nil)
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"#mapped": "bus-1"}

	lines := []string{
		":alice!u@h PRIVMSG #mapped :hello world",
		":alice!u@h PRIVMSG #mapped :\x01ACTION waves\x01",
		":alice!u@h PRIVMSG #mapped :\x01VERSION\x01",
	}
	for _, line := range lines {
		if err := rt.handleLine(context.Background(), api, mapping, 5, line); err != nil {
			t.Fatalf("handleLine(%q): %v", line, err)
		}
	}

	events := api.events["bus-1"]
	if len(events) != 2 {
		t.Fatalf("expected 2 published events (text + action), got %d", len(events))
	}
	if events[0].Kind != model.EventText || events[0].Message == nil || *events[0].Message != "hello world" {
		t.Fatalf("unexpected text event: %#v", events[0])
	}
	if events[1].Kind != model.EventAction || events[1].Message == nil || *events[1].Message != "waves" {
		t.Fatalf("unexpected action event: %#v", events[1])
	}
}

func TestFormatIRCOutboundMessage(t *testing.T) {
	msg := "waves"
	payload, ok := formatIRCOutboundMessage(model.Event{Kind: model.EventAction, Message: &msg})
	if !ok || payload != "\x01ACTION waves\x01" {
		t.Fatalf("unexpected action payload: %q (ok=%v)", payload, ok)
	}

	text := "hello"
	payload, ok = formatIRCOutboundMessage(model.Event{Kind: model.EventText, Message: &text})
	if !ok || payload != "hello" {
		t.Fatalf("unexpected text payload: %q (ok=%v)", payload, ok)
	}
}

func TestIRCCompatSyntheticEditDeleteAndReaction(t *testing.T) {
	rt := newIRCRuntime(config.Transport{Nickname: "pipo", IRCCompatMode: "synthetic", IRCCompatReactions: true, IRCReactionPrefix: "react "}, nil, nil)
	api := &captureRuntimeAPI{}
	mapping := map[string]string{"#mapped": "bus-1"}

	lines := []string{
		":alice!u@h PRIVMSG #mapped :!edit 101 rewritten",
		":alice!u@h PRIVMSG #mapped :!delete 102",
		":alice!u@h PRIVMSG #mapped :react 😀 103",
	}
	for _, line := range lines {
		if err := rt.handleLine(context.Background(), api, mapping, 5, line); err != nil {
			t.Fatalf("handleLine(%q): %v", line, err)
		}
	}
	events := api.events["bus-1"]
	if len(events) != 3 {
		t.Fatalf("expected 3 compat events, got %d", len(events))
	}
	if events[0].Kind != model.EventText || !events[0].IsEdit || events[0].Message == nil || *events[0].Message != "rewritten" {
		t.Fatalf("unexpected edit event: %#v", events[0])
	}
	if events[1].Kind != model.EventDelete || events[1].Source.MessageID == nil || *events[1].Source.MessageID != "102" {
		t.Fatalf("unexpected delete event: %#v", events[1])
	}
	if events[2].Kind != model.EventReaction || events[2].Emoji == nil || *events[2].Emoji != "😀" || events[2].Source.MessageID == nil || *events[2].Source.MessageID != "103" {
		t.Fatalf("unexpected reaction event: %#v", events[2])
	}
	if events[0].Metadata["native"] != "false" || events[2].Metadata["compat_op"] != "reaction" {
		t.Fatalf("unexpected compat metadata: %#v %#v", events[0].Metadata, events[2].Metadata)
	}
}

func TestIRCCompatDisabledCreatesAnnotation(t *testing.T) {
	rt := newIRCRuntime(config.Transport{Nickname: "pipo"}, nil, nil)
	api := &captureRuntimeAPI{}
	if err := rt.handleLine(context.Background(), api, map[string]string{"#mapped": "bus-1"}, 5, ":alice!u@h PRIVMSG #mapped :!delete 9"); err != nil {
		t.Fatalf("handleLine: %v", err)
	}
	events := api.events["bus-1"]
	if len(events) != 1 {
		t.Fatalf("expected single annotation event, got %d", len(events))
	}
	if events[0].Kind != model.EventBot || events[0].Metadata["compat_reason"] != "capability_disabled" {
		t.Fatalf("unexpected disabled compat annotation: %#v", events[0])
	}
}
