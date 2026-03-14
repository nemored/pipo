use std::path::Path;
use std::process::Command;

/// These tests are intentionally ignored until the runtime supervisor is wired.
/// They codify expected behavior so runtime logic can be implemented against them.

#[test]
#[ignore = "pending runtime protocol handshake implementation"]
fn protocol_ready_requires_fields_and_rfc3339_timestamp() {
    let line = r#"{\"event\":\"ready\",\"transport\":\"irc\",\"protocol_version\":\"v1\",\"timestamp\":\"2026-01-31T12:34:56Z\"}"#;
    let status = run_runtime_check("protocol-ready", line);
    assert!(status.success(), "ready payload should be accepted");
}

#[test]
#[ignore = "pending runtime protocol handshake implementation"]
fn protocol_version_mismatch_is_rejected() {
    let line = r#"{\"event\":\"ready\",\"transport\":\"irc\",\"protocol_version\":\"v999\",\"timestamp\":\"2026-01-31T12:34:56Z\"}"#;
    let status = run_runtime_check("protocol-version-mismatch", line);
    assert_eq!(status.code(), Some(1), "version mismatch should be rejected");
}

#[test]
#[ignore = "pending supervisor stream parser implementation"]
fn malformed_json_line_does_not_crash_supervisor() {
    let line = "{not valid json}";
    let status = run_runtime_check("malformed-json", line);
    assert!(status.success(), "supervisor should survive malformed input lines");
}

#[test]
#[ignore = "pending control-plane command handling"]
fn reload_config_emits_unimplemented_warn_and_worker_continues() {
    let line = r#"{\"cmd\":\"reload_config\"}"#;
    let status = run_runtime_check("reload-config", line);
    assert!(status.success(), "reload_config should warn and continue running");
}

#[test]
#[ignore = "pending supervisor restart policy implementation"]
fn child_restart_policy_is_enforced() {
    let status = run_runtime_check("restart-policy", "");
    assert!(status.success(), "child should restart according to configured policy");
}

#[test]
#[ignore = "pending supervisor restart intensity/window implementation"]
fn restart_intensity_window_escalates_after_threshold() {
    let status = run_runtime_check("restart-intensity-window", "");
    assert_eq!(status.code(), Some(1), "restart storm should escalate supervisor");
}

#[test]
#[ignore = "pending graceful shutdown implementation"]
fn graceful_shutdown_timeout_forces_terminate() {
    let status = run_runtime_check("shutdown-timeout", "");
    assert!(status.success(), "stuck worker should be force-terminated after timeout");
}

#[test]
#[ignore = "pending router health/degraded implementation"]
fn router_degraded_threshold_behavior() {
    let status = run_runtime_check("router-degraded-threshold", "");
    assert!(status.success(), "router should enter degraded mode only after threshold");
}

#[test]
#[ignore = "pending crash isolation implementation"]
fn router_restart_does_not_restart_transport_workers() {
    let status = run_runtime_check("router-crash-isolation", "");
    assert!(status.success(), "router restart must not bounce transport workers");
}

#[test]
#[ignore = "pending transport CLI runtime selection"]
fn transport_flag_dispatches_to_named_transport() {
    let status = run_transport_cli(["--transport", "irc"]);
    assert!(status.success(), "named transport should dispatch correctly");
}

#[test]
#[ignore = "pending transport CLI runtime selection"]
fn invalid_transport_exits_code_2() {
    let status = run_transport_cli(["--transport", "not-a-transport"]);
    assert_eq!(status.code(), Some(2));
}

#[test]
#[ignore = "pending transport auth failure exit mapping"]
fn auth_failure_exits_code_10() {
    let status = run_runtime_check("auth-failure", "");
    assert_eq!(status.code(), Some(10));
}

#[test]
#[ignore = "pending release packaging pipeline"]
fn packaging_tarball_contains_required_paths() {
    let status = run_packaging_check(["required-contents"]);
    assert!(status.success(), "tarball should include required directories/files");
}

#[test]
#[ignore = "pending release packaging pipeline"]
fn extracted_runtime_artifacts_are_executable() {
    let status = run_packaging_check(["executability"]);
    assert!(status.success(), "runtime binaries should be executable after extract");
}

#[test]
#[ignore = "pending release packaging pipeline"]
fn packaging_smoke_boots_with_require_all_ready() {
    let status = run_packaging_check(["smoke-boot-require-all-ready"]);
    assert!(status.success(), "smoke boot should wait for at least one ready event");
}

fn run_transport_cli<const N: usize>(args: [&str; N]) -> std::process::ExitStatus {
    let bin = std::env::var("PIPO_TRANSPORT_BIN").unwrap_or_else(|_| "pipo-transport".to_string());
    Command::new(bin)
        .args(args)
        .status()
        .expect("failed to run transport runtime CLI")
}

fn run_runtime_check(mode: &str, input_line: &str) -> std::process::ExitStatus {
    let harness = std::env::var("PIPO_RUNTIME_TEST_HARNESS")
        .unwrap_or_else(|_| "scripts/runtime_harness.sh".to_string());
    assert!(Path::new(&harness).exists(), "missing runtime harness: {harness}");
    Command::new(harness)
        .arg(mode)
        .arg(input_line)
        .status()
        .expect("failed to run runtime harness")
}

fn run_packaging_check<const N: usize>(args: [&str; N]) -> std::process::ExitStatus {
    let harness = std::env::var("PIPO_PACKAGING_TEST_HARNESS")
        .unwrap_or_else(|_| "scripts/packaging_harness.sh".to_string());
    assert!(Path::new(&harness).exists(), "missing packaging harness: {harness}");
    Command::new(harness)
        .args(args)
        .status()
        .expect("failed to run packaging harness")
}
