package transports

import (
	"bufio"
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"log/slog"
	"net"
	"strconv"
	"strings"
	"time"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/core"
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
}

func newIRCRuntime(cfg config.Transport, logger *slog.Logger) *ircRuntime {
	return &ircRuntime{cfg: cfg, logger: logger, activeNick: cfg.Nickname}
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

func (i *ircRuntime) runSession(ctx context.Context, api core.RuntimeAPI, buses []string) error {
	_ = api
	_ = buses
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
		lastActivity = time.Now()
		switch ircCommand(line) {
		case "PING":
			if err := i.handlePing(line); err != nil {
				return errReconnect
			}
		case "PONG":
			// heartbeat acknowledgment
		}
	}
}

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
