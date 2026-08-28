//! Shared helpers for pipo's integration tests.
//!
//! Because pipo's internals (the `Message` enum, config types, buses) are all
//! private, integration tests drive the system through the compiled binary
//! (`env!("CARGO_BIN_EXE_pipo")`) with real temp config + database files.

#![allow(dead_code)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Absolute path to the compiled `pipo` binary for this test run.
pub const PIPO_BIN: &str = env!("CARGO_BIN_EXE_pipo");

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique path under the system temp dir, safe for parallel tests.
pub fn unique_tmp(suffix: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!("pipo-it-{}-{}-{}-{}", std::process::id(), n, nanos, suffix));
    path
}

/// Write `json` to a fresh temp config file and return its path.
pub fn write_config(json: &str) -> PathBuf {
    let path = unique_tmp("config.json");
    std::fs::write(&path, json).expect("failed to write temp config");
    path
}

/// Run pipo to completion (for configs where the process exits on its own,
/// e.g. no transports or a bad config) and capture its output.
pub fn run_pipo_to_completion(args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PIPO_BIN);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("failed to run pipo")
}

/// A running pipo process. Its stdout/stderr are redirected to temp files
/// (avoiding pipe-buffer deadlock for long-lived processes) and it is killed
/// when this guard is dropped, so cleanup survives test panics.
pub struct PipoProcess {
    child: Child,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl PipoProcess {
    /// True while the process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn stderr(&self) -> String {
        std::fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }

    pub fn stdout(&self) -> String {
        std::fs::read_to_string(&self.stdout_path).unwrap_or_default()
    }
}

impl Drop for PipoProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn a long-lived pipo process against `config_path` and `db_path`.
pub fn spawn_pipo(config_path: &Path, db_path: &Path) -> PipoProcess {
    let stdout_path = unique_tmp("stdout.log");
    let stderr_path = unique_tmp("stderr.log");
    let child = Command::new(PIPO_BIN)
        .arg(config_path)
        .arg(db_path)
        .stdout(Stdio::from(File::create(&stdout_path).expect("create stdout log")))
        .stderr(Stdio::from(File::create(&stderr_path).expect("create stderr log")))
        .spawn()
        .expect("failed to spawn pipo");
    PipoProcess {
        child,
        stdout_path,
        stderr_path,
    }
}

/// Return the values of the given env vars, or `None` (printing a SKIP notice)
/// if any is unset or empty. Lets live-service tests self-skip locally and on
/// fork PRs where the corresponding secrets are unavailable.
pub fn env_or_skip(vars: &[&str]) -> Option<Vec<String>> {
    let mut values = Vec::with_capacity(vars.len());
    for var in vars {
        match std::env::var(var) {
            Ok(val) if !val.is_empty() => values.push(val),
            _ => {
                eprintln!("SKIP: env var {var} is not set; skipping live-service test");
                return None;
            }
        }
    }
    Some(values)
}
