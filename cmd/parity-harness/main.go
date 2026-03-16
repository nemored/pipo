package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"net/http"
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

type integrationFixtureSet struct {
	Scenarios []integrationScenario `json:"scenarios"`
}

type integrationScenario struct {
	Name       string              `json:"name"`
	ConfigJSON string              `json:"config_json"`
	Actions    []integrationAction `json:"actions"`
}

type integrationAction struct {
	Driver          string          `json:"driver,omitempty"`
	Method          string          `json:"method,omitempty"`
	URL             string          `json:"url,omitempty"`
	Body            json.RawMessage `json:"body,omitempty"`
	ExpectedStatus  int             `json:"expected_status,omitempty"`
	Op              string          `json:"op"`
	SlackID         *string         `json:"slack_id,omitempty"`
	DiscordID       *uint64         `json:"discord_id,omitempty"`
	PipoID          *int64          `json:"pipo_id,omitempty"`
	Emoji           *string         `json:"emoji,omitempty"`
	Remove          *bool           `json:"remove,omitempty"`
	ThreadSlackTS   *string         `json:"thread_slack_ts,omitempty"`
	ThreadDiscordID *uint64         `json:"thread_discord_id,omitempty"`
}

func (a integrationAction) op() string { return a.Op }
func (a integrationAction) toStep() step {
	return step{
		Op:              a.Op,
		SlackID:         a.SlackID,
		DiscordID:       a.DiscordID,
		PipoID:          a.PipoID,
		Emoji:           a.Emoji,
		Remove:          a.Remove,
		ThreadSlackTS:   a.ThreadSlackTS,
		ThreadDiscordID: a.ThreadDiscordID,
	}
}

type integrationScenarioResult struct {
	Name          string             `json:"name"`
	Config        configResult       `json:"config"`
	Outputs       []map[string]any   `json:"outputs"`
	DBRows        []store.MessageRow `json:"db_rows"`
	ActionResults []actionResult     `json:"action_results"`
}

type actionResult struct {
	Index  int            `json:"index"`
	Driver string         `json:"driver"`
	Op     string         `json:"op"`
	OK     bool           `json:"ok"`
	Output map[string]any `json:"output,omitempty"`
	Error  string         `json:"error,omitempty"`
}

type integrationComparisonReport struct {
	Mode      string                      `json:"mode"`
	Scenarios []integrationScenarioReport `json:"scenarios"`
	Passed    int                         `json:"passed"`
	Failed    int                         `json:"failed"`
}

type integrationScenarioReport struct {
	Name        string                    `json:"name"`
	Pass        bool                      `json:"pass"`
	Divergences []string                  `json:"divergences,omitempty"`
	Go          integrationScenarioResult `json:"go"`
	Rust        integrationScenarioResult `json:"rust"`
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
	mode := flag.String("mode", "fixture", "harness mode: fixture|integration")
	flag.Parse()

	if *mode == "integration" {
		report, err := runIntegrationMode(*fixturePath)
		if err != nil {
			fatal(err)
		}
		fmt.Println(string(must(json.MarshalIndent(report, "", "  ")).([]byte)))
		if report.Failed > 0 {
			os.Exit(1)
		}
		return
	}

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

func runIntegrationMode(fixturePath string) (integrationComparisonReport, error) {
	fixture, err := readIntegrationFixture(fixturePath)
	if err != nil {
		return integrationComparisonReport{}, err
	}
	goResults, err := runGoIntegrationScenarios(fixture)
	if err != nil {
		return integrationComparisonReport{}, err
	}
	rustResults, err := runRustIntegrationScenarios(fixturePath)
	if err != nil {
		return integrationComparisonReport{}, err
	}
	if len(goResults) != len(rustResults) {
		return integrationComparisonReport{}, fmt.Errorf("scenario count mismatch go=%d rust=%d", len(goResults), len(rustResults))
	}
	report := integrationComparisonReport{Mode: "integration", Scenarios: make([]integrationScenarioReport, 0, len(goResults))}
	for i := range goResults {
		diffs := diffScenario(goResults[i], rustResults[i])
		pass := len(diffs) == 0
		if pass {
			report.Passed++
		} else {
			report.Failed++
		}
		report.Scenarios = append(report.Scenarios, integrationScenarioReport{
			Name:        goResults[i].Name,
			Pass:        pass,
			Divergences: diffs,
			Go:          goResults[i],
			Rust:        rustResults[i],
		})
	}
	return report, nil
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
			output, err := applyStep(ctx, s, st)
			if err != nil {
				return summary{}, err
			}
			outputs = append(outputs, output)
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
	cmd := exec.Command("cargo", "run", "--quiet", "--bin", "parity_fixture_runner", "--", "--mode", "fixture", fixturePath)
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

func runRustIntegrationScenarios(fixturePath string) ([]integrationScenarioResult, error) {
	cmd := exec.Command("cargo", "run", "--quiet", "--bin", "parity_fixture_runner", "--", "--mode", "integration", fixturePath)
	cmd.Dir = "."
	cmd.Stderr = os.Stderr
	blob, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("run rust integration runner: %w", err)
	}
	var out []integrationScenarioResult
	if err := json.Unmarshal(blob, &out); err != nil {
		return nil, fmt.Errorf("decode rust integration report: %w", err)
	}
	return out, nil
}

func runGoIntegrationScenarios(f integrationFixtureSet) ([]integrationScenarioResult, error) {
	ctx := context.Background()
	out := make([]integrationScenarioResult, 0, len(f.Scenarios))
	for _, scenario := range f.Scenarios {
		cfgResult := runConfigCase(configCase{Name: scenario.Name + "-config", JSON: scenario.ConfigJSON})
		dbPath, err := tempDBPath()
		if err != nil {
			return nil, err
		}
		s, err := store.OpenSQLite(ctx, dbPath)
		if err != nil {
			return nil, err
		}
		outputs := make([]map[string]any, 0, len(scenario.Actions))
		actions := make([]actionResult, 0, len(scenario.Actions))
		for i, action := range scenario.Actions {
			driver := action.Driver
			if driver == "" {
				driver = "local"
			}
			res := actionResult{Index: i, Driver: driver, Op: action.op()}
			switch driver {
			case "local":
				output, err := executeLocalStep(ctx, s, action.toStep())
				if err != nil {
					res.OK = false
					res.Error = err.Error()
				} else {
					res.OK = true
					res.Output = output
					outputs = append(outputs, output)
				}
			case "api":
				output, err := executeAPIAction(action)
				if err != nil {
					res.OK = false
					res.Error = err.Error()
				} else {
					res.OK = true
					res.Output = output
					outputs = append(outputs, output)
				}
			default:
				res.OK = false
				res.Error = "unsupported driver: " + driver
			}
			actions = append(actions, res)
		}
		rows, err := s.DebugRows(ctx)
		if err != nil {
			return nil, err
		}
		_ = s.Close()
		_ = os.Remove(dbPath)
		out = append(out, integrationScenarioResult{Name: scenario.Name, Config: cfgResult, Outputs: outputs, DBRows: rows, ActionResults: actions})
	}
	return out, nil
}

func executeLocalStep(ctx context.Context, s *store.SQLiteStore, st step) (map[string]any, error) {
	return applyStep(ctx, s, st)
}

func applyStep(ctx context.Context, s *store.SQLiteStore, st step) (map[string]any, error) {
	switch st.Op {
	case "insert_slack":
		id, err := s.InsertOrReplaceSlack(ctx, mustStr(st.SlackID, "slack_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "insert_slack", "pipo_id": id}, nil
	case "lookup_id_by_slack":
		id, err := s.SelectIDBySlack(ctx, mustStr(st.SlackID, "slack_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "lookup_id_by_slack", "pipo_id": id}, nil
	case "update_discord":
		err := s.UpdateDiscordByID(ctx, mustI64(st.PipoID, "pipo_id"), mustU64(st.DiscordID, "discord_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "update_discord", "ok": true}, nil
	case "lookup_discord_by_slack":
		id, err := s.SelectDiscordBySlack(ctx, mustStr(st.SlackID, "slack_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "lookup_discord_by_slack", "discord_id": id}, nil
	case "insert_discord":
		id, err := s.InsertOrReplaceDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "insert_discord", "pipo_id": id}, nil
	case "lookup_id_by_discord":
		id, err := s.SelectIDByDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "lookup_id_by_discord", "pipo_id": id}, nil
	case "update_slack":
		err := s.UpdateSlackByID(ctx, mustI64(st.PipoID, "pipo_id"), mustStr(st.SlackID, "slack_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "update_slack", "ok": true}, nil
	case "route_edit_slack":
		pipo, err := s.SelectIDBySlack(ctx, mustStr(st.SlackID, "slack_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "route_edit_slack", "kind": "Text", "is_edit": true, "pipo_id": pipo, "thread": map[string]any{"slack_thread_ts": st.ThreadSlackTS}}, nil
	case "route_delete_discord":
		pipo, err := s.SelectIDByDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "route_delete_discord", "kind": "Delete", "pipo_id": pipo, "thread": map[string]any{"discord_thread": st.ThreadDiscordID}}, nil
	case "route_reaction_slack":
		pipo, err := s.SelectIDBySlack(ctx, mustStr(st.SlackID, "slack_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "route_reaction_slack", "kind": "Reaction", "pipo_id": pipo, "emoji": mustStr(st.Emoji, "emoji"), "remove": mustBool(st.Remove), "thread": map[string]any{"slack_thread_ts": st.ThreadSlackTS}}, nil
	case "route_reaction_discord":
		pipo, err := s.SelectIDByDiscord(ctx, mustU64(st.DiscordID, "discord_id"))
		if err != nil {
			return nil, err
		}
		return map[string]any{"op": "route_reaction_discord", "kind": "Reaction", "pipo_id": pipo, "emoji": mustStr(st.Emoji, "emoji"), "remove": mustBool(st.Remove), "thread": map[string]any{"discord_thread": st.ThreadDiscordID}}, nil
	default:
		return nil, fmt.Errorf("unsupported op: %s", st.Op)
	}
}

func executeAPIAction(a integrationAction) (map[string]any, error) {
	method := a.Method
	if method == "" {
		method = http.MethodPost
	}
	req, err := http.NewRequest(method, a.URL, bytes.NewReader(a.Body))
	if err != nil {
		return nil, err
	}
	if len(a.Body) > 0 {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 2048))
	if a.ExpectedStatus > 0 && resp.StatusCode != a.ExpectedStatus {
		return nil, fmt.Errorf("unexpected status got=%d want=%d", resp.StatusCode, a.ExpectedStatus)
	}
	return map[string]any{"op": a.op(), "status": resp.StatusCode, "body": string(body)}, nil
}

func runConfigCase(c configCase) configResult {
	var cfg struct {
		Buses      []config.Bus       `json:"buses"`
		Transports []config.Transport `json:"transports"`
	}
	err := decodeStrict([]byte(c.JSON), &cfg)
	if err != nil {
		return configResult{Name: c.Name, OK: false, Detail: normalizeConfigError(err.Error())}
	}
	return configResult{Name: c.Name, OK: true, Detail: fmt.Sprintf("parsed buses=%d transports=%d", len(cfg.Buses), len(cfg.Transports))}
}

func diffScenario(goScenario, rustScenario integrationScenarioResult) []string {
	diffs := []string{}
	if goScenario.Name != rustScenario.Name {
		diffs = append(diffs, fmt.Sprintf("name mismatch: go=%q rust=%q", goScenario.Name, rustScenario.Name))
	}
	if !reflect.DeepEqual(goScenario.Config, rustScenario.Config) {
		diffs = append(diffs, "config result mismatch")
	}
	if !jsonEqual(goScenario.Outputs, rustScenario.Outputs) {
		diffs = append(diffs, "normalized outputs mismatch")
	}
	if !reflect.DeepEqual(goScenario.DBRows, rustScenario.DBRows) {
		diffs = append(diffs, "db snapshot mismatch")
	}
	if !jsonEqual(goScenario.ActionResults, rustScenario.ActionResults) {
		diffs = append(diffs, "action result log mismatch")
	}
	return diffs
}

func jsonEqual(a, b any) bool {
	aj, err := json.Marshal(a)
	if err != nil {
		return false
	}
	bj, err := json.Marshal(b)
	if err != nil {
		return false
	}
	return bytes.Equal(aj, bj)
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

func readIntegrationFixture(path string) (integrationFixtureSet, error) {
	blob, err := os.ReadFile(path)
	if err != nil {
		return integrationFixtureSet{}, err
	}
	var out integrationFixtureSet
	if err := json.Unmarshal(blob, &out); err != nil {
		return integrationFixtureSet{}, err
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
