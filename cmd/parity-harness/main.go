package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"reflect"
	"strings"

	"github.com/nemored/pipo/internal/config"
	"github.com/nemored/pipo/internal/store"
)

type fixtureSet struct {
	ConfigCases    []configCase    `json:"config_cases"`
	OperationCases []operationCase `json:"operation_cases"`
}

type configCase struct {
	Name string `json:"name"`
	JSON string `json:"json"`
}

type operationCase struct {
	Name  string `json:"name"`
	Steps []step `json:"steps"`
}

type step struct {
	Op              string  `json:"op"`
	SlackID         *string `json:"slack_id,omitempty"`
	DiscordID       *uint64 `json:"discord_id,omitempty"`
	PipoID          *int64  `json:"pipo_id,omitempty"`
	Emoji           *string `json:"emoji,omitempty"`
	Remove          *bool   `json:"remove,omitempty"`
	ThreadSlackTS   *string `json:"thread_slack_ts,omitempty"`
	ThreadDiscordID *uint64 `json:"thread_discord_id,omitempty"`
}

type summary struct {
	ConfigResults    []configResult    `json:"config_results"`
	OperationResults []operationResult `json:"operation_results"`
}

type configResult struct {
	Name   string `json:"name"`
	OK     bool   `json:"ok"`
	Detail string `json:"detail"`
}

type operationResult struct {
	Name    string             `json:"name"`
	Outputs []map[string]any   `json:"outputs"`
	DBRows  []store.MessageRow `json:"db_rows"`
}

func main() {
	fixturePath := flag.String("fixture", "docs/parity/fixtures/high-risk.json", "fixture set path")
	flag.Parse()

	fixture, err := readFixture(*fixturePath)
	if err != nil {
		fatal(err)
	}

	goSummary, err := runGoSummary(fixture)
	if err != nil {
		fatal(err)
	}
	rustSummary, err := runRustSummary(*fixturePath)
	if err != nil {
		fatal(err)
	}

	goNorm, err := json.Marshal(goSummary)
	if err != nil {
		fatal(err)
	}
	rustNorm, err := json.Marshal(rustSummary)
	if err != nil {
		fatal(err)
	}

	if !reflect.DeepEqual(json.RawMessage(goNorm), json.RawMessage(rustNorm)) {
		fmt.Fprintln(os.Stderr, "divergence detected between Go and Rust outputs")
		fmt.Println("=== GO ===")
		fmt.Println(string(must(json.MarshalIndent(goSummary, "", "  ")).([]byte)))
		fmt.Println("=== RUST ===")
		fmt.Println(string(must(json.MarshalIndent(rustSummary, "", "  ")).([]byte)))
		os.Exit(1)
	}

	fmt.Println("parity OK")
	fmt.Println(string(must(json.MarshalIndent(goSummary, "", "  ")).([]byte)))
}

func runGoSummary(f fixtureSet) (summary, error) {
	ctx := context.Background()
	out := summary{}
	for _, c := range f.ConfigCases {
		var cfg struct {
			Buses      []config.Bus       `json:"buses"`
			Transports []config.Transport `json:"transports"`
		}
		err := decodeStrict([]byte(c.JSON), &cfg)
		if err != nil {
			out.ConfigResults = append(out.ConfigResults, configResult{Name: c.Name, OK: false, Detail: normalizeConfigError(err.Error())})
			continue
		}
		out.ConfigResults = append(out.ConfigResults, configResult{Name: c.Name, OK: true, Detail: fmt.Sprintf("parsed buses=%d transports=%d", len(cfg.Buses), len(cfg.Transports))})
	}

	for _, c := range f.OperationCases {
		dbPath, err := tempDBPath()
		if err != nil {
			return summary{}, err
		}
		defer os.Remove(dbPath)
		s, err := store.OpenSQLite(ctx, dbPath)
		if err != nil {
			return summary{}, err
		}
		outputs := make([]map[string]any, 0, len(c.Steps))
		for _, st := range c.Steps {
			switch st.Op {
			case "insert_slack":
				id, err := s.InsertOrReplaceSlack(ctx, mustStr(st.SlackID, "slack_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "insert_slack", "pipo_id": id})
			case "lookup_id_by_slack":
				id, err := s.SelectIDBySlack(ctx, mustStr(st.SlackID, "slack_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "lookup_id_by_slack", "pipo_id": id})
			case "update_discord":
				err := s.UpdateDiscordByID(ctx, mustI64(st.PipoID, "pipo_id"), mustU64(st.DiscordID, "discord_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "update_discord", "ok": true})
			case "lookup_discord_by_slack":
				id, err := s.SelectDiscordBySlack(ctx, mustStr(st.SlackID, "slack_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "lookup_discord_by_slack", "discord_id": id})
			case "insert_discord":
				id, err := s.InsertOrReplaceDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "insert_discord", "pipo_id": id})
			case "lookup_id_by_discord":
				id, err := s.SelectIDByDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "lookup_id_by_discord", "pipo_id": id})
			case "update_slack":
				err := s.UpdateSlackByID(ctx, mustI64(st.PipoID, "pipo_id"), mustStr(st.SlackID, "slack_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "update_slack", "ok": true})
			case "route_edit_slack":
				pipo, err := s.SelectIDBySlack(ctx, mustStr(st.SlackID, "slack_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "route_edit_slack", "kind": "Text", "is_edit": true, "pipo_id": pipo, "thread": map[string]any{"slack_thread_ts": st.ThreadSlackTS}})
			case "route_delete_discord":
				pipo, err := s.SelectIDByDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "route_delete_discord", "kind": "Delete", "pipo_id": pipo, "thread": map[string]any{"discord_thread": st.ThreadDiscordID}})
			case "route_reaction_slack":
				pipo, err := s.SelectIDBySlack(ctx, mustStr(st.SlackID, "slack_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "route_reaction_slack", "kind": "Reaction", "pipo_id": pipo, "emoji": mustStr(st.Emoji, "emoji"), "remove": mustBool(st.Remove), "thread": map[string]any{"slack_thread_ts": st.ThreadSlackTS}})
			case "route_reaction_discord":
				pipo, err := s.SelectIDByDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
				if err != nil {
					return summary{}, err
				}
				outputs = append(outputs, map[string]any{"op": "route_reaction_discord", "kind": "Reaction", "pipo_id": pipo, "emoji": mustStr(st.Emoji, "emoji"), "remove": mustBool(st.Remove), "thread": map[string]any{"discord_thread": st.ThreadDiscordID}})
			default:
				return summary{}, fmt.Errorf("unsupported op: %s", st.Op)
			}
		}
		rows, err := s.DebugRows(ctx)
		if err != nil {
			return summary{}, err
		}
		_ = s.Close()
		out.OperationResults = append(out.OperationResults, operationResult{Name: c.Name, Outputs: outputs, DBRows: rows})
	}
	return out, nil
}

func runRustSummary(fixturePath string) (summary, error) {
	cmd := exec.Command("cargo", "run", "--quiet", "--bin", "parity_fixture_runner", "--", fixturePath)
	cmd.Dir = "."
	cmd.Stderr = os.Stderr
	blob, err := cmd.Output()
	if err != nil {
		return summary{}, fmt.Errorf("run rust fixture runner: %w", err)
	}
	var out summary
	if err := json.Unmarshal(blob, &out); err != nil {
		return summary{}, fmt.Errorf("decode rust summary: %w", err)
	}
	return out, nil
}

func readFixture(path string) (fixtureSet, error) {
	blob, err := os.ReadFile(path)
	if err != nil {
		return fixtureSet{}, err
	}
	var out fixtureSet
	if err := json.Unmarshal(blob, &out); err != nil {
		return fixtureSet{}, err
	}
	return out, nil
}

func decodeStrict(data []byte, target any) error {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	if err := dec.Decode(target); err != nil {
		return err
	}
	if dec.More() {
		return errors.New("unexpected trailing JSON content")
	}
	return nil
}

func normalizeConfigError(msg string) string {
	if strings.Contains(msg, "unknown field") {
		return "config parse error: unknown field"
	}
	return "config parse error"
}

func tempDBPath() (string, error) {
	f, err := os.CreateTemp("", "pipo-parity-*.db")
	if err != nil {
		return "", err
	}
	path := f.Name()
	if err := f.Close(); err != nil {
		return "", err
	}
	return path, nil
}

func must(v any, err error) any {
	if err != nil {
		panic(err)
	}
	return v
}
func mustStr(v *string, n string) string {
	if v == nil {
		panic("missing " + n)
	}
	return *v
}
func mustI64(v *int64, n string) int64 {
	if v == nil {
		panic("missing " + n)
	}
	return *v
}
func mustU64(v *uint64, n string) uint64 {
	if v == nil {
		panic("missing " + n)
	}
	return *v
}
func mustBool(v *bool) bool {
	if v == nil {
		return false
	}
	return *v
}
func fatal(err error) { fmt.Fprintln(os.Stderr, err); os.Exit(2) }
