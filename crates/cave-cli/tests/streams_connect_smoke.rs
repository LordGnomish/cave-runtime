//! Smoke tests for `cavectl streams connect …`.
//!
//! These call the compiled `cavectl` binary as a subprocess
//! and assert that `--help` accepts the new subcommand strings.
//! The internal parse-table tests (which need access to the
//! private `Cli` / `Commands` / `ConnectCmd` enums in
//! `main.rs`) stay inline in `main.rs`; this file proves the
//! externally observable surface.

use std::process::{Command, Stdio};

fn cavectl_bin() -> &'static str {
    // Set by Cargo for integration tests against a binary target.
    env!("CARGO_BIN_EXE_cavectl")
}

fn run_help(args: &[&str]) -> std::process::Output {
    Command::new(cavectl_bin())
        .args(args)
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn cavectl")
}

#[test]
fn streams_connect_top_level_help_lists_subcommands() {
    let out = run_help(&["streams", "connect"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("worker"), "missing worker subcommand");
    assert!(s.contains("connector"), "missing connector subcommand");
    assert!(s.contains("task"), "missing task subcommand");
}

#[test]
fn streams_connect_worker_help_lists_list_and_status() {
    let out = run_help(&["streams", "connect", "worker"]);
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("list"));
    assert!(s.contains("status"));
}

#[test]
fn streams_connect_connector_help_lists_lifecycle_subcommands() {
    let out = run_help(&["streams", "connect", "connector"]);
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    for sub in &[
        "list", "get", "offsets", "create", "delete", "pause", "resume", "restart",
    ] {
        assert!(s.contains(sub), "missing connector subcommand {sub}");
    }
}

#[test]
fn streams_connect_task_help_lists_subcommands() {
    let out = run_help(&["streams", "connect", "task"]);
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    for sub in &["list", "status", "restart"] {
        assert!(s.contains(sub), "missing task subcommand {sub}");
    }
}

#[test]
fn streams_connect_connector_create_takes_repeatable_config() {
    let out = run_help(&["streams", "connect", "connector", "create"]);
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("--config"), "missing --config flag");
    // Repeatable flags carry the "value_name = k=v" hint or
    // appear with brackets in clap's help output.
    assert!(s.contains("k=v") || s.contains("CONFIG"));
}
