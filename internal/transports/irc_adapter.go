package transports

import (
	"bufio"
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/model"
)

const (
	ircDefaultPort    = 6667
	ircDefaultTLSPort = 6697
	ircDialTimeout    = 30 * time.Second
	ircIdleTimeout    = 120 * time.Second
	ircPingInterval   = 30 * time.Second
)

type ircRuntime struct {
	cfg         config.Transport
	logger      *slog.Logger
	conn        net.Conn
	reader      *bufio.Reader
	writer      *bufio.Writer
	activeNick  string
	nickAttempt int
	members     map[string]map[string]string
	namesBuf    map[string]map[string]string
}

func newIRCRuntime(cfg config.Transport, logger *slog.Logger) *ircRuntime {
	return &ircRuntime{cfg: cfg, logger: logger, activeNick: cfg.Nickname, members: map[string]map[string]string{}, namesBuf: map[string]map[string]string{}}
}

func (i *ircRuntime) connect(ctx context.Context) error {
	host, port, err := ircEndpoint(i.cfg)
	if err != nil {
		return err
	}
	timeout := ircTimeout(i.cfg)
	if i.logger != nil {
		i.logger.Info("irc lifecycle phase", "phase", "dial", "server", host, "port", port, "timeout", timeout.String())
	}
	dialer := &net.Dialer{Timeout: timeout}
	addr := net.JoinHostPort(host, strconv.Itoa(port))
	conn, err := dialer.DialContext(ctx, "tcp", addr)
	if err != nil {
		return fmt.Errorf("dial irc %s: %w", addr, err)
	}
	if i.cfg.UseTLS {
		if i.logger != nil {
			i.logger.Info("irc lifecycle phase", "phase", "tls", "server", host, "port", port)
		}
		tlsConn := tls.Client(conn, &tls.Config{ServerName: host})
		if err := tlsConn.HandshakeContext(ctx); err != nil {
			_ = conn.Close()
			return fmt.Errorf("tls handshake: %w", err)
		}
		conn = tlsConn
	}
	i.conn = conn
	i.reader = bufio.NewReader(conn)
	i.writer = bufio.NewWriter(conn)
	i.activeNick = i.cfg.Nickname
	i.nickAttempt = 0
	i.members = map[string]map[string]string{}
	i.namesBuf = map[string]map[string]string{}
	return nil
}

func (i *ircRuntime) close() {
	if i.conn != nil {
		_ = i.conn.Close()
	}
	i.conn, i.reader, i.writer = nil, nil, nil
}

func (i *ircRuntime) register(ctx context.Context) error {
	if i.logger != nil {
		i.logger.Info("irc lifecycle phase", "phase", "register", "nick", i.activeNick)
	}
	if pass := strings.TrimSpace(i.cfg.IRCPass); pass != "" {
		if err := i.sendRaw("PASS " + pass); err != nil {
			return err
		}
	}
	if err := i.sendRaw("NICK " + i.activeNick); err != nil {
		return err
	}
	if err := i.sendRaw(fmt.Sprintf("USER %s 0 * :%s", i.activeNick, i.activeNick)); err != nil {
		return err
	}
	gotWelcome := false
	gotReady := false
	for !gotWelcome || !gotReady {
		if err := i.conn.SetReadDeadline(time.Now().Add(ircTimeout(i.cfg))); err != nil {
			return fmt.Errorf("set register deadline: %w", err)
		}
		line, err := i.readLine()
		if err != nil {
			return err
		}
		cmd := ircCommand(line)
		switch cmd {
		case "PING":
			if err := i.handlePing(line); err != nil {
				return err
			}
		case "001":
			gotWelcome = true
		case "376", "422":
			gotReady = true
		case "433":
			i.nickAttempt++
			i.activeNick = fallbackNick(i.cfg.Nickname, i.nickAttempt)
			if i.logger != nil {
				i.logger.Warn("irc nickname collision", "phase", "register", "attempt", i.nickAttempt, "nick", i.activeNick)
			}
			if err := i.sendRaw("NICK " + i.activeNick); err != nil {
				return err
			}
		}
	}
	if i.logger != nil {
		i.logger.Info("irc lifecycle phase", "phase", "ready", "nick", i.activeNick)
	}
	return nil
}

func (i *ircRuntime) runSession(ctx context.Context, api core.RuntimeAPI, remoteToChannel, channelToRemote map[string]string, transportID int) error {
	i.joinMappedChannels(remoteToChannel)
	subs, err := i.subscribeOutbound(ctx, api, remoteToChannel)
	if err != nil {
		return err
	}
	lastActivity := time.Now()
	lastPing := time.Time{}
	for {
		if ctx.Err() != nil {
			return nil
		}
		_ = i.conn.SetReadDeadline(time.Now().Add(time.Second))
		line, err := i.readLine()
		if err != nil {
			if ne, ok := err.(net.Error); ok && ne.Timeout() {
				i.drainOutbound(subs, channelToRemote, transportID)
				if time.Since(lastActivity) > ircPingInterval && time.Since(lastPing) > ircPingInterval {
					lastPing = time.Now()
					if err := i.sendRaw("PING :pipo"); err != nil {
						if i.logger != nil {
							i.logger.Warn("irc lifecycle phase", "phase", "disconnect", "error", err)
						}
						return errReconnect
					}
				}
				if time.Since(lastActivity) > ircIdleTimeout {
					if i.logger != nil {
						i.logger.Warn("irc lifecycle phase", "phase", "disconnect", "reason", "idle timeout")
					}
					return errReconnect
				}
				continue
			}
			if errorsIsEOF(err) {
				if i.logger != nil {
					i.logger.Warn("irc lifecycle phase", "phase", "disconnect", "reason", "remote close")
				}
				return errReconnect
			}
			return err
		}
		i.drainOutbound(subs, channelToRemote, transportID)
		lastActivity = time.Now()
		if err := i.handleLine(api, remoteToChannel, transportID, line); err != nil {
			if err == errReconnect {
				return errReconnect
			}
			if i.logger != nil {
				i.logger.Warn("irc dispatch error", "error", err)
			}
		}
	}
}

func (i *ircRuntime) subscribeOutbound(ctx context.Context, api core.RuntimeAPI, remoteToChannel map[string]string) (map[string]<-chan model.Event, error) {
	subs := make(map[string]<-chan model.Event, len(remoteToChannel))
	seenBus := map[string]struct{}{}
	for _, busID := range remoteToChannel {
		if _, ok := seenBus[busID]; ok {
			continue
		}
		seenBus[busID] = struct{}{}
		sub, err := api.Subscribe(ctx, busID, 64)
		if err != nil {
			return nil, fmt.Errorf("subscribe outbound bus %q: %w", busID, err)
		}
		subs[busID] = sub
	}
	return subs, nil
}

func (i *ircRuntime) drainOutbound(subs map[string]<-chan model.Event, channelToRemote map[string]string, transportID int) {
	for busID, sub := range subs {
		for {
			select {
			case ev, ok := <-sub:
				if !ok {
					delete(subs, busID)
					goto nextBus
				}
				if err := i.forwardOutboundEvent(busID, channelToRemote, transportID, ev); err != nil && i.logger != nil {
					i.logger.Warn("irc outbound forwarding failed", "bus_id", busID, "error", err)
				}
			default:
				goto nextBus
			}
		}
	nextBus:
	}
}

func (i *ircRuntime) forwardOutboundEvent(busID string, channelToRemote map[string]string, transportID int, ev model.Event) error {
	if ev.Sender == transportID {
		return nil
	}
	remote, ok := channelToRemote[busID]
	if !ok {
		return nil
	}
	payload, ok := formatIRCOutboundMessage(ev)
	if !ok {
		return nil
	}
	return i.sendRaw(fmt.Sprintf("PRIVMSG %s :%s", remote, payload))
}

func formatIRCOutboundMessage(ev model.Event) (string, bool) {
	if ev.Message == nil {
		return "", false
	}
	msg := *ev.Message
	if msg == "" {
		return "", false
	}
	switch ev.Kind {
	case model.EventAction:
		return "\x01ACTION " + msg + "\x01", true
	case model.EventText:
		return msg, true
	default:
		return "", false
	}
}

func (i *ircRuntime) handleLine(api core.RuntimeAPI, remoteToChannel map[string]string, transportID int, line string) error {
	switch ircCommand(line) {
	case "PING":
		if err := i.handlePing(line); err != nil {
			return errReconnect
		}
	case "PONG":
		return nil
	case "JOIN":
		prefix, params, trailing := parseIRCLine(line)
		nick := nickFromPrefix(prefix)
		channel := strings.TrimPrefix(firstParamOrTrailing(params, trailing), ":")
		if channel == "" || nick == "" {
			return nil
		}
		busID, ok := remoteToChannel[channel]
		if !ok {
			i.logUnmapped("JOIN", channel)
			return nil
		}
		i.addMember(channel, nick)
		if strings.EqualFold(nick, i.activeNick) {
			_ = i.sendRaw("NAMES " + channel)
		}
		return i.publishMemberSnapshot(api, channel, busID, transportID, nick)
	case "PART":
		prefix, params, _ := parseIRCLine(line)
		nick := nickFromPrefix(prefix)
		if nick == "" || len(params) == 0 {
			return nil
		}
		channel := params[0]
		busID, ok := remoteToChannel[channel]
		if !ok {
			i.logUnmapped("PART", channel)
			return nil
		}
		i.removeMember(channel, nick)
		return i.publishMemberSnapshot(api, channel, busID, transportID, nick)
	case "KICK":
		prefix, params, _ := parseIRCLine(line)
		nick := nickFromPrefix(prefix)
		if len(params) < 2 {
			return nil
		}
		channel, target := params[0], params[1]
		busID, ok := remoteToChannel[channel]
		if !ok {
			i.logUnmapped("KICK", channel)
			return nil
		}
		i.removeMember(channel, target)
		if strings.EqualFold(target, i.activeNick) {
			delete(i.members, channel)
		}
		if nick == "" {
			nick = target
		}
		return i.publishMemberSnapshot(api, channel, busID, transportID, nick)
	case "QUIT":
		prefix, _, _ := parseIRCLine(line)
		nick := nickFromPrefix(prefix)
		if nick == "" {
			return nil
		}
		for channel, busID := range remoteToChannel {
			if i.removeMember(channel, nick) {
				if err := i.publishMemberSnapshot(api, channel, busID, transportID, nick); err != nil {
					return err
				}
			}
		}
	case "NICK":
		prefix, params, trailing := parseIRCLine(line)
		oldNick := nickFromPrefix(prefix)
		newNick := firstParamOrTrailing(params, trailing)
		if oldNick == "" || newNick == "" {
			return nil
		}
		if strings.EqualFold(oldNick, i.activeNick) {
			i.activeNick = newNick
		}
		for channel, busID := range remoteToChannel {
			if i.renameMember(channel, oldNick, newNick) {
				if err := i.publishMemberSnapshot(api, channel, busID, transportID, newNick); err != nil {
					return err
				}
			}
		}
	case "353":
		_, params, trailing := parseIRCLine(line)
		if len(params) < 3 {
			return nil
		}
		channel := params[2]
		if _, ok := remoteToChannel[channel]; !ok {
			i.logUnmapped("353", channel)
			return nil
		}
		for _, rawName := range strings.Fields(trailing) {
			nick := stripUserPrefix(rawName)
			if nick != "" {
				i.addNamesBuf(channel, nick)
			}
		}
	case "366":
		_, params, _ := parseIRCLine(line)
		if len(params) < 2 {
			return nil
		}
		channel := params[1]
		busID, ok := remoteToChannel[channel]
		if !ok {
			i.logUnmapped("366", channel)
			return nil
		}
		i.commitNamesBuf(channel)
		return i.publishMemberSnapshot(api, channel, busID, transportID, i.activeNick)
	case "PRIVMSG":
		prefix, params, trailing := parseIRCLine(line)
		if len(params) == 0 {
			return nil
		}
		channel := params[0]
		busID, ok := remoteToChannel[channel]
		if !ok {
			i.logUnmapped("PRIVMSG", channel)
			return nil
		}
		nick := nickFromPrefix(prefix)
		if nick == "" {
			nick = "unknown"
		}
		return i.publishIRCMessage(api, transportID, busID, nick, trailing)
	case "NOTICE":
		prefix, params, trailing := parseIRCLine(line)
		if len(params) == 0 {
			return nil
		}
		channel := params[0]
		if _, ok := remoteToChannel[channel]; !ok {
			i.logUnmapped("NOTICE", channel)
			return nil
		}
		nick := nickFromPrefix(prefix)
		if nick == "" {
			nick = "unknown"
		}
		if ctcp, ok := parseCTCP(trailing); ok {
			if ctcp.Kind != ctcpAction {
				if i.logger != nil {
					i.logger.Debug("ignoring unsupported CTCP notice", "nick", nick, "channel", channel, "command", ctcp.Command, "raw", trailing)
				}
			}
		}
	}
	return nil
}

func (i *ircRuntime) publishIRCMessage(api core.RuntimeAPI, transportID int, busID, nick, raw string) error {
	ctcp, isCTCP := parseCTCP(raw)
	if isCTCP {
		if ctcp.Kind != ctcpAction {
			if i.logger != nil {
				i.logger.Debug("ignoring unsupported CTCP privmsg", "nick", nick, "bus_id", busID, "command", ctcp.Command, "raw", raw)
			}
			return nil
		}
		event := model.Event{Kind: model.EventAction, Sender: transportID, Source: model.SourceRef{Transport: "IRC", BusID: busID}, Username: stringPtr(nick), Message: stringPtr(ctcp.Body), CreatedAt: time.Now()}
		return api.Publish(busID, event)
	}
	event := model.Event{Kind: model.EventText, Sender: transportID, Source: model.SourceRef{Transport: "IRC", BusID: busID}, Username: stringPtr(nick), Message: stringPtr(raw), CreatedAt: time.Now()}
	return api.Publish(busID, event)
}

type ctcpKind string

const (
	ctcpAction      ctcpKind = "ACTION"
	ctcpUnsupported ctcpKind = "UNSUPPORTED"
)

type ctcpMessage struct {
	Kind    ctcpKind
	Command string
	Body    string
}

func parseCTCP(raw string) (ctcpMessage, bool) {
	if len(raw) < 2 || raw[0] != '\x01' || raw[len(raw)-1] != '\x01' {
		return ctcpMessage{}, false
	}
	payload := raw[1 : len(raw)-1]
	if payload == "" {
		return ctcpMessage{Kind: ctcpUnsupported, Command: ""}, true
	}
	command, body, hasBody := strings.Cut(payload, " ")
	if strings.EqualFold(command, string(ctcpAction)) {
		if !hasBody {
			body = ""
		}
		return ctcpMessage{Kind: ctcpAction, Command: "ACTION", Body: body}, true
	}
	return ctcpMessage{Kind: ctcpUnsupported, Command: strings.ToUpper(command), Body: body}, true
}

func (i *ircRuntime) joinMappedChannels(remoteToChannel map[string]string) {
	channels := make([]string, 0, len(remoteToChannel))
	for channel := range remoteToChannel {
		channels = append(channels, channel)
	}
	sort.Strings(channels)
	for _, channel := range channels {
		if err := i.sendRaw("JOIN " + channel); err != nil && i.logger != nil {
			i.logger.Warn("irc join failed", "channel", channel, "error", err)
		}
	}
}

func (i *ircRuntime) publishMemberSnapshot(api core.RuntimeAPI, channel, busID string, transportID int, actorNick string) error {
	members := i.memberListForChannel(channel)
	body, err := json.Marshal(members)
	if err != nil {
		return fmt.Errorf("marshal member snapshot: %w", err)
	}
	transport := "IRC"
	event := model.Event{Kind: model.EventNames, Sender: transportID, Source: model.SourceRef{Transport: transport, BusID: busID}, Message: stringPtr(string(body)), CreatedAt: time.Now()}
	if actorNick != "" {
		event.Username = stringPtr(actorNick)
	}
	return api.Publish(busID, event)
}

func (i *ircRuntime) memberListForChannel(channel string) []string {
	members := i.members[channel]
	list := make([]string, 0, len(members))
	for _, nick := range members {
		list = append(list, nick)
	}
	sort.Strings(list)
	return list
}

func (i *ircRuntime) addMember(channel, nick string) {
	if i.members[channel] == nil {
		i.members[channel] = map[string]string{}
	}
	i.members[channel][strings.ToLower(nick)] = nick
}

func (i *ircRuntime) removeMember(channel, nick string) bool {
	if i.members[channel] == nil {
		return false
	}
	key := strings.ToLower(nick)
	if _, ok := i.members[channel][key]; !ok {
		return false
	}
	delete(i.members[channel], key)
	return true
}

func (i *ircRuntime) renameMember(channel, oldNick, newNick string) bool {
	if i.members[channel] == nil {
		return false
	}
	oldKey := strings.ToLower(oldNick)
	if _, ok := i.members[channel][oldKey]; !ok {
		return false
	}
	delete(i.members[channel], oldKey)
	i.members[channel][strings.ToLower(newNick)] = newNick
	return true
}

func (i *ircRuntime) addNamesBuf(channel, nick string) {
	if i.namesBuf[channel] == nil {
		i.namesBuf[channel] = map[string]string{}
	}
	i.namesBuf[channel][strings.ToLower(nick)] = nick
}

func (i *ircRuntime) commitNamesBuf(channel string) {
	if i.namesBuf[channel] == nil {
		return
	}
	i.members[channel] = i.namesBuf[channel]
	delete(i.namesBuf, channel)
}

func (i *ircRuntime) logUnmapped(command, channel string) {
	if i.logger != nil {
		i.logger.Warn("irc unmapped channel event dropped", "command", command, "channel", channel)
	}
}

func parseIRCLine(line string) (prefix string, params []string, trailing string) {
	trimmed := strings.TrimSpace(line)
	if trimmed == "" {
		return "", nil, ""
	}
	if strings.HasPrefix(trimmed, ":") {
		sp := strings.Index(trimmed, " ")
		if sp > 0 {
			prefix = trimmed[1:sp]
			trimmed = strings.TrimSpace(trimmed[sp+1:])
		}
	}
	if sp := strings.Index(trimmed, " "); sp >= 0 {
		trimmed = strings.TrimSpace(trimmed[sp+1:])
	} else {
		return prefix, nil, ""
	}
	if strings.HasPrefix(trimmed, ":") {
		return prefix, nil, strings.TrimPrefix(trimmed, ":")
	}
	if idx := strings.Index(trimmed, " :"); idx >= 0 {
		params = strings.Fields(trimmed[:idx])
		trailing = strings.TrimPrefix(trimmed[idx+1:], ":")
		return
	}
	params = strings.Fields(trimmed)
	return
}

func nickFromPrefix(prefix string) string {
	if prefix == "" {
		return ""
	}
	if nick, _, ok := strings.Cut(prefix, "!"); ok {
		return nick
	}
	return prefix
}

func firstParamOrTrailing(params []string, trailing string) string {
	if len(params) > 0 {
		return params[0]
	}
	return trailing
}

func stripUserPrefix(name string) string {
	return strings.TrimLeft(name, "~&@%+")
}

func stringPtr(v string) *string { return &v }

func (i *ircRuntime) readLine() (string, error) {
	line, err := i.reader.ReadString('\n')
	if err != nil {
		return "", err
	}
	return strings.TrimRight(line, "\r\n"), nil
}

func (i *ircRuntime) sendRaw(line string) error {
	if _, err := i.writer.WriteString(line + "\r\n"); err != nil {
		return fmt.Errorf("write irc command %q: %w", line, err)
	}
	if err := i.writer.Flush(); err != nil {
		return fmt.Errorf("flush irc command %q: %w", line, err)
	}
	return nil
}

func (i *ircRuntime) handlePing(line string) error {
	payload := ":pipo"
	parts := strings.SplitN(line, " ", 2)
	if len(parts) == 2 && strings.TrimSpace(parts[1]) != "" {
		payload = parts[1]
	}
	return i.sendRaw("PONG " + payload)
}

func ircEndpoint(cfg config.Transport) (string, int, error) {
	host, portText, hasPort := strings.Cut(cfg.Server, ":")
	if strings.HasPrefix(cfg.Server, "[") {
		parsedHost, parsedPort, err := net.SplitHostPort(cfg.Server)
		if err == nil {
			host = parsedHost
			portText = parsedPort
			hasPort = true
		}
	}
	if strings.TrimSpace(host) == "" {
		host = cfg.Server
	}
	if strings.TrimSpace(host) == "" {
		return "", 0, fmt.Errorf("irc server is required")
	}
	if cfg.IRCServerPort > 0 {
		return host, cfg.IRCServerPort, nil
	}
	if hasPort && strings.TrimSpace(portText) != "" {
		port, err := strconv.Atoi(portText)
		if err != nil {
			return "", 0, fmt.Errorf("invalid irc server port %q: %w", portText, err)
		}
		return host, port, nil
	}
	if cfg.UseTLS {
		return host, ircDefaultTLSPort, nil
	}
	return host, ircDefaultPort, nil
}

func ircTimeout(cfg config.Transport) time.Duration {
	if cfg.IRCTimeoutSeconds <= 0 {
		return ircDialTimeout
	}
	return time.Duration(cfg.IRCTimeoutSeconds) * time.Second
}

func fallbackNick(base string, attempt int) string {
	if attempt <= 0 {
		return base
	}
	return fmt.Sprintf("%s_%d", base, attempt)
}

func ircCommand(line string) string {
	trimmed := strings.TrimSpace(line)
	if trimmed == "" {
		return ""
	}
	if strings.HasPrefix(trimmed, ":") {
		parts := strings.SplitN(trimmed, " ", 3)
		if len(parts) > 1 {
			return strings.ToUpper(parts[1])
		}
		return ""
	}
	parts := strings.SplitN(trimmed, " ", 2)
	return strings.ToUpper(parts[0])
}

func errorsIsEOF(err error) bool {
	return err == io.EOF || strings.Contains(err.Error(), "use of closed network connection")
}
