package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"time"

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

type runtimeExecution struct {
	Summary summary
	RawLog  string
}

type integrationRuntimeExecution struct {
	Results []integrationScenarioResult
	RawLog  string
}

type fixtureDiffReport struct {
	Mode        string   `json:"mode"`
	Pass        bool     `json:"pass"`
	Divergences []string `json:"divergences,omitempty"`
	Go          summary  `json:"go"`
	Rust        summary  `json:"rust"`
}

func main() {
	fixturePath := flag.String("fixture", "docs/parity/fixtures/high-risk.json", "fixture set path")
	mode := flag.String("mode", "fixture", "harness mode: fixture|integration")
	artifactRoot := flag.String("artifact-root", "artifacts/parity", "base directory for parity artifacts")
	runID := flag.String("run-id", time.Now().UTC().Format("20060102T150405Z"), "artifact run identifier")
	flag.Parse()

	runDir := filepath.Join(*artifactRoot, *runID)
	artifacts := newArtifactWriter(runDir, *mode, *fixturePath)

	if *mode == "integration" {
		report, goExec, rustExec, err := runIntegrationMode(*fixturePath)
		if err != nil {
			fatal(err)
		}
		if err := artifacts.writeIntegrationArtifacts(goExec, rustExec, report); err != nil {
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

	goExec, err := runGoSummary(fixture)
	if err != nil {
		fatal(err)
	}
	rustExec, err := runRustSummary(*fixturePath)
	if err != nil {
		fatal(err)
	}
	goSummary := goExec.Summary
	rustSummary := rustExec.Summary

	goNorm, err := json.Marshal(goSummary)
	if err != nil {
		fatal(err)
	}
	rustNorm, err := json.Marshal(rustSummary)
	if err != nil {
		fatal(err)
	}

	diffs := []string{}
	if !reflect.DeepEqual(json.RawMessage(goNorm), json.RawMessage(rustNorm)) {
		diffs = append(diffs, "summary mismatch between Go and Rust outputs")
	}
	report := fixtureDiffReport{Mode: "fixture", Pass: len(diffs) == 0, Divergences: diffs, Go: goSummary, Rust: rustSummary}
	if err := artifacts.writeFixtureArtifacts(goExec, rustExec, report); err != nil {
		fatal(err)
	}

	if len(diffs) > 0 {
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

func runIntegrationMode(fixturePath string) (integrationComparisonReport, integrationRuntimeExecution, integrationRuntimeExecution, error) {
	fixture, err := readIntegrationFixture(fixturePath)
	if err != nil {
		return integrationComparisonReport{}, integrationRuntimeExecution{}, integrationRuntimeExecution{}, err
	}
	goExec, err := runGoIntegrationScenarios(fixture)
	if err != nil {
		return integrationComparisonReport{}, integrationRuntimeExecution{}, integrationRuntimeExecution{}, err
	}
	rustExec, err := runRustIntegrationScenarios(fixturePath)
	if err != nil {
		return integrationComparisonReport{}, integrationRuntimeExecution{}, integrationRuntimeExecution{}, err
	}
	goResults := goExec.Results
	rustResults := rustExec.Results
	if len(goResults) != len(rustResults) {
		return integrationComparisonReport{}, integrationRuntimeExecution{}, integrationRuntimeExecution{}, fmt.Errorf("scenario count mismatch go=%d rust=%d", len(goResults), len(rustResults))
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
	return report, goExec, rustExec, nil
}

func runGoSummary(f fixtureSet) (runtimeExecution, error) {
	ctx := context.Background()
	out := summary{}
	var log strings.Builder
	for _, c := range f.ConfigCases {
		var cfg struct {
			Buses      []config.Bus       `json:"buses"`
			Transports []config.Transport `json:"transports"`
		}
		err := decodeStrict([]byte(c.JSON), &cfg)
		if err != nil {
			out.ConfigResults = append(out.ConfigResults, configResult{Name: c.Name, OK: false, Detail: normalizeConfigError(err.Error())})
			fmt.Fprintf(&log, "config_case=%s ok=false detail=%s\n", c.Name, normalizeConfigError(err.Error()))
			continue
		}
		out.ConfigResults = append(out.ConfigResults, configResult{Name: c.Name, OK: true, Detail: fmt.Sprintf("parsed buses=%d transports=%d", len(cfg.Buses), len(cfg.Transports))})
		fmt.Fprintf(&log, "config_case=%s ok=true buses=%d transports=%d\n", c.Name, len(cfg.Buses), len(cfg.Transports))
	}

	for _, c := range f.OperationCases {
		dbPath, err := tempDBPath()
		if err != nil {
			return runtimeExecution{}, err
		}
		defer os.Remove(dbPath)
		s, err := store.OpenSQLite(ctx, dbPath)
		if err != nil {
			return runtimeExecution{}, err
		}
		outputs := make([]map[string]any, 0, len(c.Steps))
		for _, st := range c.Steps {
			output, err := applyStep(ctx, s, st)
			if err != nil {
				return runtimeExecution{}, err
			}
			outputs = append(outputs, output)
			fmt.Fprintf(&log, "operation_case=%s op=%s output=%s\n", c.Name, st.Op, must(json.Marshal(output)).([]byte))
		}
		rows, err := s.DebugRows(ctx)
		if err != nil {
			return runtimeExecution{}, err
		}
		_ = s.Close()
		out.OperationResults = append(out.OperationResults, operationResult{Name: c.Name, Outputs: outputs, DBRows: rows})
		fmt.Fprintf(&log, "operation_case=%s db_rows=%d\n", c.Name, len(rows))
	}
	return runtimeExecution{Summary: out, RawLog: log.String()}, nil
}

func runRustSummary(fixturePath string) (runtimeExecution, error) {
	cmd := exec.Command("cargo", "run", "--quiet", "--bin", "parity_fixture_runner", "--", "--mode", "fixture", fixturePath)
	cmd.Dir = "."
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	if err != nil {
		return runtimeExecution{}, fmt.Errorf("run rust fixture runner: %w\nstderr:\n%s", err, stderr.String())
	}
	var out summary
	if err := json.Unmarshal(stdout.Bytes(), &out); err != nil {
		return runtimeExecution{}, fmt.Errorf("decode rust summary: %w", err)
	}
	return runtimeExecution{Summary: out, RawLog: strings.TrimSpace(stderr.String() + "\n" + stdout.String())}, nil
}

func runRustIntegrationScenarios(fixturePath string) (integrationRuntimeExecution, error) {
	cmd := exec.Command("cargo", "run", "--quiet", "--bin", "parity_fixture_runner", "--", "--mode", "integration", fixturePath)
	cmd.Dir = "."
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	if err != nil {
		return integrationRuntimeExecution{}, fmt.Errorf("run rust integration runner: %w\nstderr:\n%s", err, stderr.String())
	}
	var out []integrationScenarioResult
	if err := json.Unmarshal(stdout.Bytes(), &out); err != nil {
		return integrationRuntimeExecution{}, fmt.Errorf("decode rust integration report: %w", err)
	}
	return integrationRuntimeExecution{Results: out, RawLog: strings.TrimSpace(stderr.String() + "\n" + stdout.String())}, nil
}

func runGoIntegrationScenarios(f integrationFixtureSet) (integrationRuntimeExecution, error) {
	ctx := context.Background()
	out := make([]integrationScenarioResult, 0, len(f.Scenarios))
	var log strings.Builder
	for _, scenario := range f.Scenarios {
		cfgResult := runConfigCase(configCase{Name: scenario.Name + "-config", JSON: scenario.ConfigJSON})
		dbPath, err := tempDBPath()
		if err != nil {
			return integrationRuntimeExecution{}, err
		}
		s, err := store.OpenSQLite(ctx, dbPath)
		if err != nil {
			return integrationRuntimeExecution{}, err
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
			fmt.Fprintf(&log, "scenario=%s action=%d driver=%s op=%s ok=%t\n", scenario.Name, i, res.Driver, res.Op, res.OK)
		}
		rows, err := s.DebugRows(ctx)
		if err != nil {
			return integrationRuntimeExecution{}, err
		}
		_ = s.Close()
		_ = os.Remove(dbPath)
		out = append(out, integrationScenarioResult{Name: scenario.Name, Config: cfgResult, Outputs: outputs, DBRows: rows, ActionResults: actions})
		fmt.Fprintf(&log, "scenario=%s db_rows=%d outputs=%d\n", scenario.Name, len(rows), len(outputs))
	}
	return integrationRuntimeExecution{Results: out, RawLog: log.String()}, nil
}

type artifactWriter struct {
	runDir      string
	mode        string
	fixturePath string
}

func newArtifactWriter(runDir, mode, fixturePath string) artifactWriter {
	return artifactWriter{runDir: runDir, mode: mode, fixturePath: fixturePath}
}

func (w artifactWriter) writeFixtureArtifacts(goExec, rustExec runtimeExecution, report fixtureDiffReport) error {
	if err := w.init(); err != nil {
		return err
	}
	if err := w.writeRuntimeArtifacts("go", goExec.RawLog, goExec.Summary); err != nil {
		return err
	}
	if err := w.writeRuntimeArtifacts("rust", rustExec.RawLog, rustExec.Summary); err != nil {
		return err
	}
	if err := writeJSON(filepath.Join(w.runDir, "diff", "report.json"), report); err != nil {
		return err
	}
	status := "PASS"
	if !report.Pass {
		status = "FAIL"
	}
	return os.WriteFile(filepath.Join(w.runDir, "diff", "summary.txt"), []byte(fmt.Sprintf("mode=%s result=%s divergences=%d\n", w.mode, status, len(report.Divergences))), 0o644)
}

func (w artifactWriter) writeIntegrationArtifacts(goExec, rustExec integrationRuntimeExecution, report integrationComparisonReport) error {
	if err := w.init(); err != nil {
		return err
	}
	if err := w.writeIntegrationRuntimeArtifacts("go", goExec.RawLog, goExec.Results); err != nil {
		return err
	}
	if err := w.writeIntegrationRuntimeArtifacts("rust", rustExec.RawLog, rustExec.Results); err != nil {
		return err
	}
	if err := writeJSON(filepath.Join(w.runDir, "diff", "report.json"), report); err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(w.runDir, "diff", "summary.txt"), []byte(fmt.Sprintf("mode=%s passed=%d failed=%d\n", w.mode, report.Passed, report.Failed)), 0o644)
}

func (w artifactWriter) init() error {
	if err := os.MkdirAll(filepath.Join(w.runDir, "runtimes", "go"), 0o755); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(w.runDir, "runtimes", "rust"), 0o755); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Join(w.runDir, "diff"), 0o755); err != nil {
		return err
	}
	manifestBytes, err := os.ReadFile(w.fixturePath)
	if err != nil {
		return err
	}
	if err := os.WriteFile(filepath.Join(w.runDir, "scenario-manifest.json"), manifestBytes, 0o644); err != nil {
		return err
	}
	if err := writeJSON(filepath.Join(w.runDir, "runtimes", "go", "runtime-config.json"), map[string]any{"mode": w.mode, "fixture_path": w.fixturePath, "binary": "cmd/parity-harness"}); err != nil {
		return err
	}
	return writeJSON(filepath.Join(w.runDir, "runtimes", "rust", "runtime-config.json"), map[string]any{"mode": w.mode, "fixture_path": w.fixturePath, "binary": "src/bin/parity_fixture_runner.rs"})
}

func (w artifactWriter) writeRuntimeArtifacts(runtime, raw string, s summary) error {
	runtimeDir := filepath.Join(w.runDir, "runtimes", runtime)
	if err := os.WriteFile(filepath.Join(runtimeDir, "transport-transcript.log"), []byte(raw), 0o644); err != nil {
		return err
	}
	if err := writeJSON(filepath.Join(runtimeDir, "normalized-output.json"), s); err != nil {
		return err
	}
	dbDump := map[string]any{"messages": flattenRowsFromSummary(s)}
	return writeJSON(filepath.Join(runtimeDir, "messages-table.json"), dbDump)
}

func (w artifactWriter) writeIntegrationRuntimeArtifacts(runtime, raw string, results []integrationScenarioResult) error {
	runtimeDir := filepath.Join(w.runDir, "runtimes", runtime)
	if err := os.WriteFile(filepath.Join(runtimeDir, "transport-transcript.log"), []byte(raw), 0o644); err != nil {
		return err
	}
	if err := writeJSON(filepath.Join(runtimeDir, "normalized-output.json"), results); err != nil {
		return err
	}
	dbDump := map[string]any{"messages": flattenRowsFromIntegration(results)}
	return writeJSON(filepath.Join(runtimeDir, "messages-table.json"), dbDump)
}

func writeJSON(path string, v any) error {
	blob, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, blob, fs.FileMode(0o644))
}

func flattenRowsFromSummary(s summary) []store.MessageRow {
	rows := make([]store.MessageRow, 0)
	for _, c := range s.OperationResults {
		rows = append(rows, c.DBRows...)
	}
	return rows
}

func flattenRowsFromIntegration(results []integrationScenarioResult) []store.MessageRow {
	rows := make([]store.MessageRow, 0)
	for _, c := range results {
		rows = append(rows, c.DBRows...)
	}
	return rows
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
