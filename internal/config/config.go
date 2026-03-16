package config

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/nemored/pipo/internal/store"
)

type Bus struct {
	ID string `json:"id"`
}

type ParsedConfig struct {
	Buses      []Bus       `json:"buses"`
	Transports []Transport `json:"transports"`
}

type Transport struct {
	Kind string          `json:"transport"`
	Raw  json.RawMessage `json:"-"`

	Nickname            string            `json:"nickname,omitempty"`
	Server              string            `json:"server,omitempty"`
	UseTLS              bool              `json:"use_tls,omitempty"`
	ImgRoot             string            `json:"img_root,omitempty"`
	Token               string            `json:"token,omitempty"`
	BotToken            string            `json:"bot_token,omitempty"`
	GuildID             uint64            `json:"guild_id,omitempty"`
	Username            string            `json:"username,omitempty"`
	Password            *string           `json:"password"`
	ClientCert          *string           `json:"client_cert"`
	ServerCert          *string           `json:"server_cert"`
	Comment             *string           `json:"comment,omitempty"`
	APIKey              string            `json:"api_key,omitempty"`
	Interval            uint64            `json:"interval,omitempty"`
	ChannelMapping      map[string]string `json:"channel_mapping,omitempty"`
	VoiceChannelMapping map[string]string `json:"voice_channel_mapping,omitempty"`
	Buses               []string          `json:"buses,omitempty"`
}

func (t *Transport) UnmarshalJSON(data []byte) error {
	type alias Transport
	aux := struct {
		Transport string `json:"transport"`
		*alias
	}{alias: (*alias)(t)}
	if err := json.Unmarshal(data, &aux); err != nil {
		return err
	}
	t.Kind = aux.Transport
	t.Raw = append(t.Raw[:0], data...)
	return nil
}

func Load(path string) (ParsedConfig, error) {
	blob, err := os.ReadFile(path)
	if err != nil {
		return ParsedConfig{}, fmt.Errorf("read config: %w", err)
	}
	blob = store.StripJSONComments(blob)
	var cfg ParsedConfig
	if err := json.Unmarshal(blob, &cfg); err != nil {
		return ParsedConfig{}, fmt.Errorf("parse config: %w", err)
	}
	if len(cfg.Buses) == 0 {
		return ParsedConfig{}, fmt.Errorf("parse config: buses must not be empty")
	}
	for _, bus := range cfg.Buses {
		if bus.ID == "" {
			return ParsedConfig{}, fmt.Errorf("parse config: buses[].id is required")
		}
	}
	for i, transport := range cfg.Transports {
		if transport.Kind == "" {
			return ParsedConfig{}, fmt.Errorf("parse config: transports[%d].transport is required", i)
		}
	}
	return cfg, nil
}
