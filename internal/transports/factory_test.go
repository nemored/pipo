package transports

import (
	"context"
	"testing"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/store"
)

func TestBuildConstructsTypedAdapters(t *testing.T) {
	cfg := config.ParsedConfig{
		Buses: []config.Bus{{ID: "main"}},
		Transports: []config.Transport{
			{Kind: "Slack", ChannelMapping: map[string]string{"main": "C1"}},
			{Kind: "Discord", ChannelMapping: map[string]string{"main": "1"}},
			{Kind: "IRC", ChannelMapping: map[string]string{"main": "#chan"}},
			{Kind: "Mumble", ChannelMapping: map[string]string{"main": "text"}, VoiceChannelMapping: map[string]string{"main": "voice"}},
			{Kind: "Rachni", Buses: []string{"main"}},
			{Kind: "Minecraft", Buses: []string{"main"}},
		},
	}
	s, err := store.OpenSQLite(context.Background(), ":memory:")
	if err != nil {
		t.Fatalf("open sqlite: %v", err)
	}
	defer s.Close()

	got, err := Build(cfg, s, nil, nil)
	if err != nil {
		t.Fatalf("Build: %v", err)
	}
	if len(got) != len(cfg.Transports) {
		t.Fatalf("expected %d transports, got %d", len(cfg.Transports), len(got))
	}
	if got[0].Name() != "Slack[0]" || got[1].Name() != "Discord[1]" || got[2].Name() != "IRC[2]" {
		t.Fatalf("unexpected adapter names: %q, %q, %q", got[0].Name(), got[1].Name(), got[2].Name())
	}
	if err := got[5].Run(context.Background(), nil); err == nil {
		t.Fatalf("minecraft adapter should stay not implemented")
	}
}
