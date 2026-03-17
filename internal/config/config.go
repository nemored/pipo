package config

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"

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
	IRCServerPort       int               `json:"port,omitempty"`
	IRCTimeoutSeconds   int               `json:"timeout_seconds,omitempty"`
	IRCPass             string            `json:"pass,omitempty"`
	ImgRoot             string            `json:"img_root,omitempty"`
	Token               string            `json:"token,omitempty"`
	BotToken            string            `json:"bot_token,omitempty"`
	SlackAppToken       string            `json:"slack_app_token,omitempty"`
	SlackBotToken       string            `json:"slack_bot_token,omitempty"`
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
	IRCCompatMode       string            `json:"compat_mode,omitempty"`
	IRCCompatReactions  bool              `json:"compat_reactions,omitempty"`
	IRCReactionPrefix   string            `json:"reaction_prefix,omitempty"`
}

func (t *Transport) UnmarshalJSON(data []byte) error {
	tt, err := decodeTransport(data, -1)
	if err != nil {
		return err
	}
	*t = tt
	return nil
}

func Load(path string) (ParsedConfig, error) {
	blob, err := os.ReadFile(path)
	if err != nil {
		return ParsedConfig{}, fmt.Errorf("read config: %w", err)
	}
	blob = store.StripJSONComments(blob)

	top := struct {
		Buses      []Bus             `json:"buses"`
		Transports []json.RawMessage `json:"transports"`
	}{}
	if err := decodeStrict(blob, &top); err != nil {
		return ParsedConfig{}, fmt.Errorf("parse config: %w", err)
	}
	if len(top.Buses) == 0 {
		return ParsedConfig{}, fmt.Errorf("parse config: buses must not be empty")
	}
	for i, bus := range top.Buses {
		if bus.ID == "" {
			return ParsedConfig{}, fmt.Errorf("parse config: buses[%d].id is required", i)
		}
	}

	cfg := ParsedConfig{Buses: top.Buses, Transports: make([]Transport, 0, len(top.Transports))}
	for i, raw := range top.Transports {
		transport, err := decodeTransport(raw, i)
		if err != nil {
			return ParsedConfig{}, err
		}
		cfg.Transports = append(cfg.Transports, transport)
	}
	return cfg, nil
}

func decodeTransport(raw []byte, index int) (Transport, error) {
	prefix := "parse config"
	if index >= 0 {
		prefix = fmt.Sprintf("parse config: transports[%d]", index)
	}

	tag := struct {
		Kind string `json:"transport"`
	}{}
	if err := json.Unmarshal(raw, &tag); err != nil {
		return Transport{}, fmt.Errorf("%s: invalid transport block: %w", prefix, err)
	}
	if tag.Kind == "" {
		return Transport{}, fmt.Errorf("%s: transport is required", prefix)
	}

	t := Transport{Kind: tag.Kind, Raw: append(json.RawMessage(nil), raw...)}
	fields, err := parseObject(raw)
	if err != nil {
		return Transport{}, fmt.Errorf("%s: invalid transport block: %w", prefix, err)
	}

	switch tag.Kind {
	case "IRC":
		var cfg struct {
			Kind            string            `json:"transport"`
			Nickname        string            `json:"nickname"`
			Server          string            `json:"server"`
			UseTLS          bool              `json:"use_tls"`
			Port            *int              `json:"port"`
			TimeoutSeconds  *int              `json:"timeout_seconds"`
			Pass            *string           `json:"pass"`
			ImgRoot         string            `json:"img_root"`
			ChannelMapping  map[string]string `json:"channel_mapping"`
			CompatMode      *string           `json:"compat_mode"`
			CompatReactions *bool             `json:"compat_reactions"`
			ReactionPrefix  *string           `json:"reaction_prefix"`
		}
		if err := decodeStrict(raw, &cfg); err != nil {
			return Transport{}, fmt.Errorf("%s (IRC): %w", prefix, err)
		}
		if err := requireFields(fields, map[string]bool{"nickname": false, "server": false, "use_tls": false, "img_root": false, "channel_mapping": false}); err != nil {
			return Transport{}, fmt.Errorf("%s (IRC): %w", prefix, err)
		}
		t.Nickname, t.Server, t.UseTLS, t.ImgRoot, t.ChannelMapping = cfg.Nickname, cfg.Server, cfg.UseTLS, cfg.ImgRoot, cfg.ChannelMapping
		if cfg.CompatMode != nil {
			t.IRCCompatMode = *cfg.CompatMode
		}
		if cfg.CompatReactions != nil {
			t.IRCCompatReactions = *cfg.CompatReactions
		}
		if cfg.ReactionPrefix != nil {
			t.IRCReactionPrefix = *cfg.ReactionPrefix
		}
		if cfg.Port != nil {
			t.IRCServerPort = *cfg.Port
		}
		if cfg.TimeoutSeconds != nil {
			t.IRCTimeoutSeconds = *cfg.TimeoutSeconds
		}
		if cfg.Pass != nil {
			t.IRCPass = *cfg.Pass
		}
	case "Discord":
		var cfg struct {
			Kind           string            `json:"transport"`
			Token          string            `json:"token"`
			GuildID        uint64            `json:"guild_id"`
			ChannelMapping map[string]string `json:"channel_mapping"`
		}
		if err := decodeStrict(raw, &cfg); err != nil {
			return Transport{}, fmt.Errorf("%s (Discord): %w", prefix, err)
		}
		if err := requireFields(fields, map[string]bool{"token": false, "guild_id": false, "channel_mapping": false}); err != nil {
			return Transport{}, fmt.Errorf("%s (Discord): %w", prefix, err)
		}
		t.Token, t.GuildID, t.ChannelMapping = cfg.Token, cfg.GuildID, cfg.ChannelMapping
	case "Slack":
		var cfg struct {
			Kind           string            `json:"transport"`
			Token          *string           `json:"token"`
			BotToken       *string           `json:"bot_token"`
			SlackAppToken  *string           `json:"slack_app_token"`
			SlackBotToken  *string           `json:"slack_bot_token"`
			ChannelMapping map[string]string `json:"channel_mapping"`
		}
		if err := decodeStrict(raw, &cfg); err != nil {
			return Transport{}, fmt.Errorf("%s (Slack): %w", prefix, err)
		}
		if err := requireFields(fields, map[string]bool{"channel_mapping": false}); err != nil {
			return Transport{}, fmt.Errorf("%s (Slack): %w", prefix, err)
		}
		if cfg.Token != nil {
			t.Token = *cfg.Token
		}
		if cfg.BotToken != nil {
			t.BotToken = *cfg.BotToken
		}
		if cfg.SlackAppToken != nil {
			t.SlackAppToken = *cfg.SlackAppToken
		}
		if cfg.SlackBotToken != nil {
			t.SlackBotToken = *cfg.SlackBotToken
		}
		if strings.TrimSpace(t.SlackAppToken) == "" {
			t.SlackAppToken = t.Token
		}
		if strings.TrimSpace(t.SlackBotToken) == "" {
			t.SlackBotToken = t.BotToken
		}
		if strings.TrimSpace(t.SlackAppToken) == "" || strings.TrimSpace(t.SlackBotToken) == "" {
			return Transport{}, fmt.Errorf("%s (Slack): must provide slack_app_token/slack_bot_token (or token/bot_token)", prefix)
		}
		t.ChannelMapping = cfg.ChannelMapping
	case "Minecraft":
		var cfg struct {
			Kind     string   `json:"transport"`
			Username string   `json:"username"`
			Buses    []string `json:"buses"`
		}
		if err := decodeStrict(raw, &cfg); err != nil {
			return Transport{}, fmt.Errorf("%s (Minecraft): %w", prefix, err)
		}
		if err := requireFields(fields, map[string]bool{"username": false, "buses": false}); err != nil {
			return Transport{}, fmt.Errorf("%s (Minecraft): %w", prefix, err)
		}
		t.Username, t.Buses = cfg.Username, cfg.Buses
	case "Mumble":
		var cfg struct {
			Kind                string            `json:"transport"`
			Server              string            `json:"server"`
			Password            *string           `json:"password"`
			Nickname            string            `json:"nickname"`
			ClientCert          *string           `json:"client_cert"`
			ServerCert          *string           `json:"server_cert"`
			Comment             *string           `json:"comment,omitempty"`
			ChannelMapping      map[string]string `json:"channel_mapping"`
			VoiceChannelMapping map[string]string `json:"voice_channel_mapping"`
		}
		if err := decodeStrict(raw, &cfg); err != nil {
			return Transport{}, fmt.Errorf("%s (Mumble): %w", prefix, err)
		}
		if err := requireFields(fields, map[string]bool{
			"server": false, "password": true, "nickname": false, "client_cert": true,
			"server_cert": true, "channel_mapping": false, "voice_channel_mapping": false,
		}); err != nil {
			return Transport{}, fmt.Errorf("%s (Mumble): %w", prefix, err)
		}
		t.Server, t.Password, t.Nickname, t.ClientCert, t.ServerCert, t.Comment = cfg.Server, cfg.Password, cfg.Nickname, cfg.ClientCert, cfg.ServerCert, cfg.Comment
		t.ChannelMapping, t.VoiceChannelMapping = cfg.ChannelMapping, cfg.VoiceChannelMapping
	case "Rachni":
		var cfg struct {
			Kind     string   `json:"transport"`
			Server   string   `json:"server"`
			APIKey   string   `json:"api_key"`
			Interval uint64   `json:"interval"`
			Buses    []string `json:"buses"`
		}
		if err := decodeStrict(raw, &cfg); err != nil {
			return Transport{}, fmt.Errorf("%s (Rachni): %w", prefix, err)
		}
		if err := requireFields(fields, map[string]bool{"server": false, "api_key": false, "interval": false, "buses": false}); err != nil {
			return Transport{}, fmt.Errorf("%s (Rachni): %w", prefix, err)
		}
		t.Server, t.APIKey, t.Interval, t.Buses = cfg.Server, cfg.APIKey, cfg.Interval, cfg.Buses
	default:
		return Transport{}, fmt.Errorf("%s: unsupported transport %q (expected one of IRC, Discord, Slack, Minecraft, Mumble, Rachni)", prefix, tag.Kind)
	}

	return t, nil
}

func decodeStrict(data []byte, target any) error {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	if err := dec.Decode(target); err != nil {
		return err
	}
	if dec.More() {
		return fmt.Errorf("unexpected trailing JSON content")
	}
	return nil
}

func parseObject(raw []byte) (map[string]json.RawMessage, error) {
	var out map[string]json.RawMessage
	if err := json.Unmarshal(raw, &out); err != nil {
		return nil, err
	}
	if out == nil {
		return nil, fmt.Errorf("expected JSON object")
	}
	return out, nil
}

func requireFields(fields map[string]json.RawMessage, required map[string]bool) error {
	for key, nullable := range required {
		v, ok := fields[key]
		if !ok {
			if nullable {
				return fmt.Errorf("field %q is required (use null when unset)", key)
			}
			return fmt.Errorf("field %q is required", key)
		}
		if !nullable && strings.TrimSpace(string(v)) == "null" {
			return fmt.Errorf("field %q cannot be null", key)
		}
	}
	return nil
}

func SupportedTransportKinds() []string {
	kinds := []string{"IRC", "Discord", "Slack", "Minecraft", "Mumble", "Rachni"}
	sort.Strings(kinds)
	return kinds
}
