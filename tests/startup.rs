//! Always-on integration tests that exercise pipo's real startup path end to
//! end through the binary: argument/env parsing, config file read + comment
//! stripping, JSON deserialization, sqlite pool creation, schema bootstrap and
//! the bus/transport wiring loop. These need no external services.

mod common;

use common::{run_pipo_to_completion, unique_tmp, write_config};
use rusqlite::Connection;

const EMPTY_CONFIG: &str = r#"{"buses":[{"id":"main"}],"transports":[]}"#;

fn messages_table_exists(db: &std::path::Path) -> bool {
    let conn = Connection::open(db).expect("failed to open sqlite db");
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'",
            [],
            |row| row.get(0),
        )
        .expect("failed to query sqlite_master");
    count >= 1
}

#[test]
fn usage_printed_when_args_missing() {
    let out = run_pipo_to_completion(&[], &[]);
    assert!(out.status.success(), "pipo should exit successfully with no args");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Usage:"),
        "expected a usage message on stdout, got: {stdout}"
    );
}

#[test]
fn empty_transports_config_bootstraps_messages_table() {
    let config = write_config(EMPTY_CONFIG);
    let db = unique_tmp("db.sqlite3");
    let out = run_pipo_to_completion(
        &[config.to_str().unwrap(), db.to_str().unwrap()],
        &[],
    );
    assert!(
        out.status.success(),
        "pipo should exit cleanly for a no-transport config; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        messages_table_exists(&db),
        "pipo should have created the `messages` table during startup"
    );
}

#[test]
fn config_and_db_read_from_env_vars() {
    let config = write_config(EMPTY_CONFIG);
    let db = unique_tmp("db.sqlite3");
    let out = run_pipo_to_completion(
        &[],
        &[
            ("CONFIG_PATH", config.to_str().unwrap()),
            ("DB_PATH", db.to_str().unwrap()),
        ],
    );
    assert!(
        out.status.success(),
        "pipo should honor CONFIG_PATH/DB_PATH env vars; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        messages_table_exists(&db),
        "pipo should have created the `messages` table via env-var config"
    );
}

#[test]
fn invalid_config_json_reports_parse_error() {
    let config = write_config("this is definitely not json");
    let db = unique_tmp("db.sqlite3");
    let out = run_pipo_to_completion(
        &[config.to_str().unwrap(), db.to_str().unwrap()],
        &[],
    );
    // pipo's main swallows the error and still exits 0, so assert on stderr.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Couldn't parse the JSON"),
        "expected a config parse error on stderr, got: {stderr}"
    );
}
