use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn run_runner(args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_parity_fixture_runner"))
        .args(args)
        .output()
        .expect("spawn parity fixture runner");

    if !output.status.success() {
        panic!(
            "runner failed: status={:?}\nstdout:\n{}\nstderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    serde_json::from_slice(&output.stdout).expect("runner outputs JSON")
}

#[test]
fn high_risk_fixture_succeeds() {
    let fixture = fixture_path("docs/parity/fixtures/high-risk.json");
    let value = run_runner(&[fixture.to_str().expect("fixture path utf8")]);

    let config_results = value["config_results"]
        .as_array()
        .expect("config_results array");
    assert_eq!(config_results.len(), 2);
    assert_eq!(config_results[0]["ok"], Value::Bool(true));
    assert_eq!(config_results[1]["ok"], Value::Bool(false));

    let operation_results = value["operation_results"]
        .as_array()
        .expect("operation_results array");
    assert_eq!(operation_results.len(), 1);

    let outputs = operation_results[0]["outputs"].as_array().expect("outputs array");
    assert_eq!(outputs.len(), 11);
    assert_eq!(outputs[5]["op"], Value::String("route_reaction_slack".to_string()));
    assert_eq!(outputs[10]["op"], Value::String("route_reaction_discord".to_string()));
}

#[test]
fn integration_fixture_local_scenario_succeeds() {
    let fixture = fixture_path("docs/parity/fixtures/integration.json");
    let value = run_runner(&[
        "--mode",
        "integration",
        fixture.to_str().expect("fixture path utf8"),
    ]);

    let scenarios = value.as_array().expect("scenario results array");
    assert_eq!(scenarios.len(), 1);
    assert_eq!(
        scenarios[0]["name"],
        Value::String("local-slack-discord-routing".to_string())
    );

    let actions = scenarios[0]["action_results"]
        .as_array()
        .expect("action_results array");
    assert_eq!(actions.len(), 4);
    assert!(actions.iter().all(|action| action["ok"] == Value::Bool(true)));

    let db_rows = scenarios[0]["db_rows"].as_array().expect("db_rows array");
    assert_eq!(db_rows.len(), 1);
    assert_eq!(db_rows[0]["discord_id"], Value::Number(9911223344_u64.into()));
}
