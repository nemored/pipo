package transports

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
	"github.com/nemored/pipo/internal/model"
)

const slackSocketOpenURL = "https://slack.com/api/apps.connections.open"
const slackAuthTestURL = "https://slack.com/api/auth.test"
const slackConversationsListURL = "https://slack.com/api/conversations.list"
const slackUsersListURL = "https://slack.com/api/users.list"

type slackWSDialer interface {
	DialContext(ctx context.Context, urlStr string, requestHeader http.Header) (slackWSConn, *http.Response, error)
}

type slackWSConn interface {
	ReadMessage() (messageType int, p []byte, err error)
	WriteJSON(v any) error
	WriteControl(messageType int, data []byte, deadline time.Time) error
	Close() error
}

type slackDialer struct{ d *websocket.Dialer }

func (d slackDialer) DialContext(ctx context.Context, urlStr string, requestHeader http.Header) (slackWSConn, *http.Response, error) {
	conn, resp, err := d.d.DialContext(ctx, urlStr, requestHeader)
	if err != nil {
		return nil, resp, err
	}
	return conn, resp, nil
}

type slackRuntime struct {
	cfg      config.Transport
	logger   *slog.Logger
	http     *http.Client
	dialer   slackWSDialer
	conn     slackWSConn
	appToken string
	botToken string

	wsURL   string
	selfID  string
	teamID  string
	botID   string
	session string

	mu              sync.RWMutex
	userDisplayName map[string]string
	channelMeta     map[string]slackChannelMeta
	refreshInterval time.Duration
}

type slackChannelMeta struct {
	ID        string
	Name      string
	IsPrivate bool
}

func newSlackRuntime(cfg config.Transport, logger *slog.Logger) *slackRuntime {
	return &slackRuntime{
		cfg:             cfg,
		logger:          logger,
		http:            &http.Client{Timeout: 20 * time.Second},
		dialer:          slackDialer{d: &websocket.Dialer{HandshakeTimeout: 15 * time.Second}},
		appToken:        slackAppToken(cfg),
		botToken:        slackBotToken(cfg),
		userDisplayName: map[string]string{},
		channelMeta:     map[string]slackChannelMeta{},
		refreshInterval: 10 * time.Minute,
	}
}

func (s *slackRuntime) connect(ctx context.Context) error {
	if strings.TrimSpace(s.appToken) == "" || strings.TrimSpace(s.botToken) == "" {
		return asTerminal(fmt.Errorf("slack auth: token and bot token are required"))
	}
	url, err := s.openSocketMode(ctx)
	if err != nil {
		return err
	}
	meta, err := s.authTest(ctx)
	if err != nil {
		return err
	}
	conn, _, err := s.dialer.DialContext(ctx, url, nil)
	if err != nil {
		return fmt.Errorf("dial websocket: %w", err)
	}
	s.conn = conn
	s.wsURL = url
	s.selfID, s.teamID, s.botID = meta.UserID, meta.TeamID, meta.BotID

	if err := s.expectHello(); err != nil {
		s.close()
		return err
	}
	if err := s.refreshCaches(ctx); err != nil {
		s.close()
		return err
	}
	if s.logger != nil {
		s.logger.Info("slack lifecycle phase", "phase", "ready", "team_id", s.teamID, "user_id", s.selfID)
	}
	return nil
}

func (s *slackRuntime) runSession(ctx context.Context, api core.RuntimeAPI, remoteToChannel, channelToRemote map[string]string, transportID int) error {
	subs, err := s.subscribeOutbound(ctx, api, remoteToChannel)
	if err != nil {
		return err
	}
	acks := make(chan string, 32)
	errCh := make(chan error, 2)
	var wg sync.WaitGroup

	wg.Add(1)
	go func() {
		defer wg.Done()
		errCh <- s.readLoop(ctx, api, remoteToChannel, transportID, acks)
	}()
	wg.Add(1)
	go func() {
		defer wg.Done()
		errCh <- s.refreshLoop(ctx)
	}()
	wg.Add(1)
	go func() {
		defer wg.Done()
		errCh <- s.writeLoop(ctx, acks, subs, channelToRemote, transportID)
	}()

	var outErr error
	for i := 0; i < 3; i++ {
		err := <-errCh
		if outErr == nil && err != nil && !errors.Is(err, context.Canceled) {
			outErr = err
		}
	}
	close(acks)
	wg.Wait()
	if outErr != nil {
		return outErr
	}
	return nil
}

func (s *slackRuntime) subscribeOutbound(ctx context.Context, api core.RuntimeAPI, remoteToChannel map[string]string) (map[string]<-chan model.Event, error) {
	subs := make(map[string]<-chan model.Event, len(remoteToChannel))
	seen := map[string]struct{}{}
	for _, busID := range remoteToChannel {
		if _, ok := seen[busID]; ok {
			continue
		}
		seen[busID] = struct{}{}
		sub, err := api.Subscribe(ctx, busID, 64)
		if err != nil {
			return nil, err
		}
		subs[busID] = sub
	}
	return subs, nil
}

type slackEventsEnvelope struct {
	Type       string `json:"type"`
	EnvelopeID string `json:"envelope_id"`
	Reason     string `json:"reason"`
	Payload    struct {
		Event json.RawMessage `json:"event"`
	} `json:"payload"`
}

func (s *slackRuntime) readLoop(ctx context.Context, api core.RuntimeAPI, remoteToChannel map[string]string, transportID int, acks chan<- string) error {
	for {
		if ctx.Err() != nil {
			return nil
		}
		_, msg, err := s.conn.ReadMessage()
		if err != nil {
			return classifySlackReadError(err, s.logger)
		}
		var envelope slackEventsEnvelope
		if err := json.Unmarshal(msg, &envelope); err != nil {
			if s.logger != nil {
				s.logger.Warn("slack websocket decode failed", "error", err)
			}
			continue
		}
		if envelope.EnvelopeID != "" {
			select {
			case acks <- envelope.EnvelopeID:
			case <-ctx.Done():
				return nil
			}
		}
		if envelope.Type == "disconnect" {
			if s.logger != nil {
				s.logger.Warn("slack lifecycle phase", "phase", "disconnect", "reason", envelope.Reason)
			}
			return errReconnect
		}
		if envelope.Type != "events_api" {
			continue
		}
		s.handleEventsAPI(envelope.Payload.Event, api, remoteToChannel, transportID)
	}
}

func (s *slackRuntime) handleEventsAPI(raw json.RawMessage, api core.RuntimeAPI, remoteToChannel map[string]string, transportID int) {
	var base struct {
		Type string `json:"type"`
	}
	if err := json.Unmarshal(raw, &base); err != nil {
		if s.logger != nil {
			s.logger.Warn("slack events_api decode failed", "error", err)
		}
		return
	}
	switch base.Type {
	case "message":
		var event struct {
			Type    string `json:"type"`
			Subtype string `json:"subtype"`
			User    string `json:"user"`
			Channel string `json:"channel"`
			Text    string `json:"text"`
			TS      string `json:"ts"`
		}
		if err := json.Unmarshal(raw, &event); err != nil {
			return
		}
		if event.Subtype != "" {
			return
		}
		busID, ok := remoteToChannel[event.Channel]
		if !ok {
			if s.logger != nil {
				s.logger.Warn("slack inbound channel unmapped", "channel_id", event.Channel)
			}
			return
		}
		txt := event.Text
		username := s.displayNameForUser(event.User)
		norm := model.Event{
			Kind:     model.EventText,
			Sender:   transportID,
			Source:   model.SourceRef{Transport: "Slack", BusID: busID, MessageID: &event.TS},
			Message:  &txt,
			Username: &username,
			Metadata: map[string]string{"slack_channel_id": event.Channel, "slack_channel_name": s.channelNameForID(event.Channel)},
		}
		if err := api.Publish(busID, norm); err != nil && s.logger != nil {
			s.logger.Warn("slack inbound publish failed", "bus_id", busID, "error", err)
		}
	case "channel_created":
		var event struct {
			Channel slackChannelMeta `json:"channel"`
		}
		if err := json.Unmarshal(raw, &event); err != nil || strings.TrimSpace(event.Channel.ID) == "" {
			return
		}
		s.upsertChannel(event.Channel)
	case "channel_rename":
		var event struct {
			Channel struct {
				ID   string `json:"id"`
				Name string `json:"name"`
			} `json:"channel"`
		}
		if err := json.Unmarshal(raw, &event); err != nil || strings.TrimSpace(event.Channel.ID) == "" {
			return
		}
		s.upsertChannel(slackChannelMeta{ID: event.Channel.ID, Name: event.Channel.Name})
	case "user_change", "team_join":
		var event struct {
			User struct {
				ID      string `json:"id"`
				Name    string `json:"name"`
				Profile struct {
					DisplayName string `json:"display_name"`
					RealName    string `json:"real_name"`
				} `json:"profile"`
			} `json:"user"`
		}
		if err := json.Unmarshal(raw, &event); err != nil || strings.TrimSpace(event.User.ID) == "" {
			return
		}
		s.upsertUserName(event.User.ID, firstNonEmptySlack(event.User.Profile.DisplayName, event.User.Profile.RealName, event.User.Name, event.User.ID))
	}
}

func (s *slackRuntime) upsertUserName(userID, name string) {
	if strings.TrimSpace(userID) == "" {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	s.userDisplayName[userID] = firstNonEmptySlack(name, userID)
}

func (s *slackRuntime) upsertChannel(meta slackChannelMeta) {
	if strings.TrimSpace(meta.ID) == "" {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	existing, ok := s.channelMeta[meta.ID]
	if ok {
		if strings.TrimSpace(meta.Name) == "" {
			meta.Name = existing.Name
		}
		if !meta.IsPrivate {
			meta.IsPrivate = existing.IsPrivate
		}
	}
	s.channelMeta[meta.ID] = meta
}

func (s *slackRuntime) refreshLoop(ctx context.Context) error {
	ticker := time.NewTicker(s.refreshInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
			if err := s.refreshCaches(ctx); err != nil && s.logger != nil {
				s.logger.Warn("slack cache refresh failed", "error", err)
			}
		}
	}
}

func (s *slackRuntime) writeLoop(ctx context.Context, acks <-chan string, subs map[string]<-chan model.Event, channelToRemote map[string]string, transportID int) error {
	ticker := time.NewTicker(25 * time.Second)
	defer ticker.Stop()
	for {
		s.drainOutbound(subs, channelToRemote, transportID)
		select {
		case <-ctx.Done():
			_ = s.conn.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(websocket.CloseNormalClosure, "shutdown"), time.Now().Add(2*time.Second))
			return nil
		case envelopeID, ok := <-acks:
			if !ok {
				return nil
			}
			if err := s.conn.WriteJSON(map[string]any{"envelope_id": envelopeID}); err != nil {
				return classifySlackWriteError(err, s.logger)
			}
		case <-ticker.C:
			if err := s.conn.WriteJSON(map[string]any{"type": "ping", "id": time.Now().UnixNano()}); err != nil {
				return classifySlackWriteError(err, s.logger)
			}
		}
	}
}

func (s *slackRuntime) drainOutbound(subs map[string]<-chan model.Event, channelToRemote map[string]string, transportID int) {
	for busID, sub := range subs {
		for {
			select {
			case ev, ok := <-sub:
				if !ok {
					delete(subs, busID)
					goto next
				}
				if err := s.forwardOutboundEvent(busID, channelToRemote, transportID, ev); err != nil && s.logger != nil {
					s.logger.Warn("slack outbound forwarding failed", "bus_id", busID, "error", err)
				}
			default:
				goto next
			}
		}
	next:
	}
}

func (s *slackRuntime) forwardOutboundEvent(busID string, channelToRemote map[string]string, transportID int, ev model.Event) error {
	if ev.Sender == transportID {
		return nil
	}
	remoteID, ok := channelToRemote[busID]
	if !ok {
		if s.logger != nil {
			s.logger.Warn("slack outbound bus unmapped", "bus_id", busID)
		}
		return nil
	}
	if ev.Kind != model.EventText || ev.Message == nil || *ev.Message == "" {
		return nil
	}
	return s.callChatPostMessage(remoteID, *ev.Message)
}

func (s *slackRuntime) callChatPostMessage(channelID, text string) error {
	body := strings.NewReader(fmt.Sprintf(`{"channel":%q,"text":%q}`, channelID, text))
	req, err := http.NewRequest(http.MethodPost, "https://slack.com/api/chat.postMessage", body)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+s.botToken)
	req.Header.Set("Content-Type", "application/json")
	resp, err := s.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= http.StatusBadRequest {
		return fmt.Errorf("chat.postMessage status %d", resp.StatusCode)
	}
	return nil
}

func (s *slackRuntime) displayNameForUser(userID string) string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if name, ok := s.userDisplayName[userID]; ok && strings.TrimSpace(name) != "" {
		return name
	}
	return "unknown-user:" + userID
}

func (s *slackRuntime) channelNameForID(channelID string) string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if meta, ok := s.channelMeta[channelID]; ok && strings.TrimSpace(meta.Name) != "" {
		return meta.Name
	}
	return "unknown-channel:" + channelID
}

func (s *slackRuntime) refreshCaches(ctx context.Context) error {
	channels, err := s.fetchChannels(ctx)
	if err != nil {
		return err
	}
	users, err := s.fetchUsers(ctx)
	if err != nil {
		return err
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	for id, meta := range channels {
		s.channelMeta[id] = meta
	}
	for id, name := range users {
		s.userDisplayName[id] = name
	}
	return nil
}

func (s *slackRuntime) fetchChannels(ctx context.Context) (map[string]slackChannelMeta, error) {
	out := map[string]slackChannelMeta{}
	cursor := ""
	for {
		resp, err := s.callSlackAPIWithQuery(ctx, s.botToken, slackConversationsListURL, map[string]string{"limit": "200", "types": "public_channel,private_channel", "cursor": cursor})
		if err != nil {
			return nil, err
		}
		var page struct {
			OK       bool   `json:"ok"`
			Error    string `json:"error"`
			Channels []struct {
				ID        string `json:"id"`
				Name      string `json:"name"`
				IsPrivate bool   `json:"is_private"`
			} `json:"channels"`
			ResponseMetadata struct {
				NextCursor string `json:"next_cursor"`
			} `json:"response_metadata"`
		}
		if err := decodeSlackResponse(resp, &page); err != nil {
			return nil, err
		}
		if !page.OK {
			return nil, fmt.Errorf("slack conversations.list failed: %s", page.Error)
		}
		for _, ch := range page.Channels {
			out[ch.ID] = slackChannelMeta{ID: ch.ID, Name: ch.Name, IsPrivate: ch.IsPrivate}
		}
		cursor = strings.TrimSpace(page.ResponseMetadata.NextCursor)
		if cursor == "" {
			break
		}
	}
	return out, nil
}

func (s *slackRuntime) fetchUsers(ctx context.Context) (map[string]string, error) {
	out := map[string]string{}
	cursor := ""
	for {
		resp, err := s.callSlackAPIWithQuery(ctx, s.botToken, slackUsersListURL, map[string]string{"limit": "200", "cursor": cursor})
		if err != nil {
			return nil, err
		}
		var page struct {
			OK      bool   `json:"ok"`
			Error   string `json:"error"`
			Members []struct {
				ID      string `json:"id"`
				Name    string `json:"name"`
				Deleted bool   `json:"deleted"`
				Profile struct {
					DisplayName string `json:"display_name"`
					RealName    string `json:"real_name"`
				} `json:"profile"`
			} `json:"members"`
			ResponseMetadata struct {
				NextCursor string `json:"next_cursor"`
			} `json:"response_metadata"`
		}
		if err := decodeSlackResponse(resp, &page); err != nil {
			return nil, err
		}
		if !page.OK {
			return nil, fmt.Errorf("slack users.list failed: %s", page.Error)
		}
		for _, member := range page.Members {
			if member.Deleted {
				continue
			}
			name := firstNonEmptySlack(member.Profile.DisplayName, member.Profile.RealName, member.Name, member.ID)
			out[member.ID] = name
		}
		cursor = strings.TrimSpace(page.ResponseMetadata.NextCursor)
		if cursor == "" {
			break
		}
	}
	return out, nil
}

func decodeSlackResponse(resp *http.Response, out any) error {
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(body, out); err != nil {
		return err
	}
	return nil
}

func firstNonEmptySlack(vals ...string) string {
	for _, v := range vals {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}

func (s *slackRuntime) callSlackAPIWithQuery(ctx context.Context, token, endpoint string, query map[string]string) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, err
	}
	q := req.URL.Query()
	for k, v := range query {
		if strings.TrimSpace(v) == "" {
			continue
		}
		q.Set(k, v)
	}
	req.URL.RawQuery = q.Encode()
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Content-Type", "application/json")
	return s.http.Do(req)
}

func (s *slackRuntime) close() {
	if s.conn != nil {
		_ = s.conn.Close()
		s.conn = nil
	}
}

type slackSocketOpenResponse struct {
	OK    bool   `json:"ok"`
	Error string `json:"error"`
	URL   string `json:"url"`
}

type slackAuthTestResponse struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error"`
	UserID string `json:"user_id"`
	TeamID string `json:"team_id"`
	BotID  string `json:"bot_id"`
}

func (s *slackRuntime) openSocketMode(ctx context.Context) (string, error) {
	resp, err := s.callSlackAPI(ctx, s.appToken, slackSocketOpenURL)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden {
		return "", asTerminal(fmt.Errorf("slack socket open status %d", resp.StatusCode))
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("read slack socket response: %w", err)
	}
	var out slackSocketOpenResponse
	if err := json.Unmarshal(body, &out); err != nil {
		return "", fmt.Errorf("decode slack socket response: %w", err)
	}
	if !out.OK {
		if out.Error == "invalid_auth" || out.Error == "not_authed" {
			return "", asTerminal(fmt.Errorf("slack socket open auth failed: %s", out.Error))
		}
		return "", fmt.Errorf("slack socket open failed: %s", out.Error)
	}
	if out.URL == "" {
		return "", fmt.Errorf("slack socket open returned empty url")
	}
	return out.URL, nil
}

func (s *slackRuntime) authTest(ctx context.Context) (slackAuthTestResponse, error) {
	resp, err := s.callSlackAPI(ctx, s.botToken, slackAuthTestURL)
	if err != nil {
		return slackAuthTestResponse{}, err
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden {
		return slackAuthTestResponse{}, asTerminal(fmt.Errorf("slack auth.test status %d", resp.StatusCode))
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return slackAuthTestResponse{}, fmt.Errorf("read slack auth.test response: %w", err)
	}
	var out slackAuthTestResponse
	if err := json.Unmarshal(body, &out); err != nil {
		return slackAuthTestResponse{}, fmt.Errorf("decode slack auth.test response: %w", err)
	}
	if !out.OK {
		if out.Error == "invalid_auth" || out.Error == "not_authed" {
			return slackAuthTestResponse{}, asTerminal(fmt.Errorf("slack auth.test auth failed: %s", out.Error))
		}
		return slackAuthTestResponse{}, fmt.Errorf("slack auth.test failed: %s", out.Error)
	}
	return out, nil
}

func (s *slackRuntime) callSlackAPI(ctx context.Context, token, endpoint string) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("Content-Type", "application/json")
	return s.http.Do(req)
}

func (s *slackRuntime) expectHello() error {
	_, payload, err := s.conn.ReadMessage()
	if err != nil {
		return classifySlackReadError(err, s.logger)
	}
	var msg struct {
		Type         string `json:"type"`
		ConnectionID string `json:"connection_info"`
		DebugInfo    struct {
			Host string `json:"host"`
		} `json:"debug_info"`
	}
	if err := json.Unmarshal(payload, &msg); err != nil {
		return fmt.Errorf("decode slack hello: %w", err)
	}
	if msg.Type != "hello" {
		return fmt.Errorf("slack expected hello but got %q", msg.Type)
	}
	s.session = msg.DebugInfo.Host
	return nil
}

func classifySlackReadError(err error, logger *slog.Logger) error {
	if websocket.IsUnexpectedCloseError(err, websocket.CloseNormalClosure) || websocket.IsCloseError(err, websocket.CloseGoingAway, websocket.CloseAbnormalClosure, websocket.CloseServiceRestart, websocket.CloseTryAgainLater) || errors.Is(err, io.EOF) {
		if logger != nil {
			logger.Warn("slack lifecycle phase", "phase", "disconnect", "error", err)
		}
		return errReconnect
	}
	return err
}

func classifySlackWriteError(err error, logger *slog.Logger) error {
	if websocket.IsCloseError(err, websocket.CloseGoingAway, websocket.CloseAbnormalClosure, websocket.CloseServiceRestart, websocket.CloseTryAgainLater) || errors.Is(err, io.EOF) {
		if logger != nil {
			logger.Warn("slack lifecycle phase", "phase", "disconnect", "error", err)
		}
		return errReconnect
	}
	return err
}

func slackAppToken(cfg config.Transport) string {
	if strings.TrimSpace(cfg.SlackAppToken) != "" {
		return cfg.SlackAppToken
	}
	return cfg.Token
}

func slackBotToken(cfg config.Transport) string {
	if strings.TrimSpace(cfg.SlackBotToken) != "" {
		return cfg.SlackBotToken
	}
	return cfg.BotToken
}
