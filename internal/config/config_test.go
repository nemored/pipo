package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeConfig(t *testing.T, body string) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "config.json")
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}
	return p
}

func TestLoadSupportsAllTransportsAndComments(t *testing.T) {
	path := writeConfig(t, `{
// top-level comment
"buses": [{"id": "main"}],
"transports": [
  {"transport":"IRC","nickname":"n","server":"s","use_tls":true,"img_root":"i","channel_mapping":{"main":"#chan"}},
  {"transport":"Discord","token":"t","guild_id":1,"channel_mapping":{"main":"1"}},
  {"transport":"Slack","token":"t","bot_token":"b","channel_mapping":{"main":"C1"}},
  {"transport":"Minecraft","username":"steve","buses":["main"]},
  {"transport":"Mumble","server":"m","password":null,"nickname":"pipo","client_cert":null,"server_cert":null,
   "channel_mapping":{"main":"text"},"voice_channel_mapping":{"main":"voice"}},
  {"transport":"Rachni","server":"r","api_key":"k","interval":10,"buses":["main"]}
]// trailing comment
}`)

	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	if len(cfg.Transports) != 6 {
		t.Fatalf("expected 6 transports, got %d", len(cfg.Transports))
	}
	if cfg.Transports[0].ChannelMapping["main"] != "#chan" {
		t.Fatalf("expected channel mapping to be preserved")
	}
	if cfg.Transports[4].VoiceChannelMapping["main"] != "voice" {
		t.Fatalf("expected voice mapping to be preserved")
	}
}

func TestLoadValidationMessagesPerTransport(t *testing.T) {
	path := writeConfig(t, `{
"buses": [{"id": "main"}],
"transports": [
  {"transport":"Mumble","server":"m","nickname":"pipo","client_cert":null,"server_cert":null,
   "channel_mapping":{"main":"text"},"voice_channel_mapping":{"main":"voice"}}
]
}`)
	_, err := Load(path)
	if err == nil {
		t.Fatalf("expected error")
	}
	if got := err.Error(); !strings.Contains(got, `transports[0] (Mumble): field "password" is required`) {
		t.Fatalf("unexpected error message: %s", got)
	}
}

func TestLoadIRCOptionalConnectionFields(t *testing.T) {
	path := writeConfig(t, `{
"buses": [{"id": "main"}],
"transports": [
  {"transport":"IRC","nickname":"n","server":"irc.example","port":7000,"timeout_seconds":12,"pass":"sekret","use_tls":true,"img_root":"i","channel_mapping":{"main":"#chan"}}
]
}`)

	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("Load returned error: %v", err)
	}
	irc := cfg.Transports[0]
	if irc.IRCServerPort != 7000 || irc.IRCTimeoutSeconds != 12 || irc.IRCPass != "sekret" {
		t.Fatalf("expected optional IRC connection fields to be parsed, got port=%d timeout=%d pass=%q", irc.IRCServerPort, irc.IRCTimeoutSeconds, irc.IRCPass)
	}
}

func TestLoadRejectsUnknownTransportField(t *testing.T) {
	path := writeConfig(t, `{
"buses": [{"id": "main"}],
"transports": [
  {"transport":"IRC","nickname":"n","server":"s","use_tls":true,"img_root":"i","channel_mapping":{"main":"#chan"},"extra":1}
]
}`)
	_, err := Load(path)
	if err == nil {
		t.Fatalf("expected error")
	}
	if got := err.Error(); !strings.Contains(got, `transports[0] (IRC): json: unknown field "extra"`) {
		t.Fatalf("unexpected error message: %s", got)
	}
}
