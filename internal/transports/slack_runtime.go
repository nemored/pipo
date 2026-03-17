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
}

func newSlackRuntime(cfg config.Transport, logger *slog.Logger) *slackRuntime {
	return &slackRuntime{cfg: cfg, logger: logger, http: &http.Client{Timeout: 20 * time.Second}, dialer: slackDialer{d: &websocket.Dialer{HandshakeTimeout: 15 * time.Second}}, appToken: slackAppToken(cfg), botToken: slackBotToken(cfg)}
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
		errCh <- s.readLoop(ctx, acks)
	}()
	wg.Add(1)
	go func() {
		defer wg.Done()
		errCh <- s.writeLoop(ctx, acks, subs, channelToRemote, transportID)
	}()

	var outErr error
	for i := 0; i < 2; i++ {
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

func (s *slackRuntime) readLoop(ctx context.Context, acks chan<- string) error {
	for {
		if ctx.Err() != nil {
			return nil
		}
		_, msg, err := s.conn.ReadMessage()
		if err != nil {
			return classifySlackReadError(err, s.logger)
		}
		var envelope struct {
			Type       string `json:"type"`
			EnvelopeID string `json:"envelope_id"`
			Reason     string `json:"reason"`
		}
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
				if ev.Sender == transportID {
					continue
				}
				_ = channelToRemote
			default:
				goto next
			}
		}
	next:
	}
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
