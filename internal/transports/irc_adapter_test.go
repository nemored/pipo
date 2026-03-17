package transports

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"strings"
	"testing"
	"time"

	"github.com/nemored/pipo/internal/config"
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

	rt := newIRCRuntime(config.Transport{Nickname: "pipo", Server: listener.Addr().String(), IRCPass: "secret"}, nil)
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
