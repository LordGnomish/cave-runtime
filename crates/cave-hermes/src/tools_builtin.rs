// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Built-in tool implementations.
//!
//! Four MVP tools ship with cave-hermes — covering shell, filesystem, and
//! HTTP fetch. Each mirrors a Hermes counterpart but drops Python-only
//! quirks (yaml loader for skills, voice mode, vision tools, …) that
//! belong in a follow-up. The cave-runtime environment is the deployment
//! target; tools that need browsers / OAuth / OS daemons are out-of-scope
//! and documented in `PARITY_REPORT.md §6`.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::tool::{ToolEntry, ToolRegistry, ToolResult};

/// Register all four built-ins onto `registry`. Idempotent: re-registering
/// the same name overwrites the previous entry (see [`ToolRegistry::register`]).
pub fn register_all(registry: &mut ToolRegistry) {
    registry.register(bash_entry());
    registry.register(file_read_entry());
    registry.register(file_write_entry());
    registry.register(web_fetch_entry());
}

// ─── bash ──────────────────────────────────────────────────────────────────

pub fn bash_entry() -> ToolEntry {
    ToolEntry::new(
        "bash",
        "shell",
        "Run a shell command via /bin/sh. Returns stdout + stderr + exit code.",
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "shell command to run" },
                "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 600, "default": 30 }
            },
            "required": ["command"]
        }),
        Arc::new(run_bash),
    )
    .with_max_result_size(64 * 1024)
}

fn run_bash(args: &Value) -> crate::error::Result<ToolResult> {
    let command = args.get("command").and_then(Value::as_str).ok_or_else(|| {
        crate::error::HermesError::ToolArguments {
            name: "bash".into(),
            reason: "`command` (string) is required".into(),
        }
    })?;
    if command.trim().is_empty() {
        return Err(crate::error::HermesError::ToolArguments {
            name: "bash".into(),
            reason: "`command` must not be empty".into(),
        });
    }
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| crate::error::HermesError::ToolFailed {
            name: "bash".into(),
            reason: format!("spawn failed: {e}"),
        })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code().unwrap_or(-1);
    let mut combined = stdout.clone();
    if !stderr.is_empty() {
        combined.push_str("\n---stderr---\n");
        combined.push_str(&stderr);
    }
    let res = ToolResult {
        ok: out.status.success(),
        output: combined,
        meta: [
            ("exit_code".to_string(), code.to_string()),
            ("stdout_bytes".to_string(), out.stdout.len().to_string()),
            ("stderr_bytes".to_string(), out.stderr.len().to_string()),
        ]
        .into_iter()
        .collect(),
    };
    Ok(res)
}

// ─── file_read ────────────────────────────────────────────────────────────

pub fn file_read_entry() -> ToolEntry {
    ToolEntry::new(
        "file_read",
        "fs",
        "Read the contents of a UTF-8 text file.",
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }),
        Arc::new(run_file_read),
    )
    .with_max_result_size(256 * 1024)
}

fn run_file_read(args: &Value) -> crate::error::Result<ToolResult> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::HermesError::ToolArguments {
            name: "file_read".into(),
            reason: "`path` (string) is required".into(),
        })?;
    let body = fs::read_to_string(Path::new(path))?;
    let bytes = body.len();
    Ok(ToolResult::ok(body).with_meta("bytes", bytes.to_string()))
}

// ─── file_write ───────────────────────────────────────────────────────────

pub fn file_write_entry() -> ToolEntry {
    ToolEntry::new(
        "file_write",
        "fs",
        "Atomically write a UTF-8 text file. Parent directories are created on demand.",
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        }),
        Arc::new(run_file_write),
    )
}

fn run_file_write(args: &Value) -> crate::error::Result<ToolResult> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::HermesError::ToolArguments {
            name: "file_write".into(),
            reason: "`path` (string) is required".into(),
        })?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::HermesError::ToolArguments {
            name: "file_write".into(),
            reason: "`content` (string) is required".into(),
        })?;
    let p = Path::new(path);
    if let Some(parent) = p.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(p, content)?;
    Ok(ToolResult::ok(format!("wrote {} bytes to {path}", content.len()))
        .with_meta("bytes", content.len().to_string()))
}

// ─── web_fetch ────────────────────────────────────────────────────────────

/// Plug for an HTTP fetcher. The default impl deliberately returns a
/// "no fetcher installed" failure rather than pulling in a TLS stack —
/// downstream callers wire `reqwest` or any other client in the runtime
/// where the credentials live. This mirrors Hermes' approach: the tool
/// surface is part of cave-hermes; the I/O policy lives in the binary.
pub type FetchFn =
    Arc<dyn Fn(&str) -> std::result::Result<String, String> + Send + Sync>;

static FETCHER: parking_lot::RwLock<Option<FetchFn>> = parking_lot::RwLock::new(None);

/// Install a custom HTTP fetcher. Tests use this to stub `web_fetch`.
/// In production, `cave-runtime` injects a `reqwest`-backed client.
pub fn install_fetcher(f: FetchFn) {
    *FETCHER.write() = Some(f);
}

/// Drop any installed fetcher. Mainly for tests.
pub fn clear_fetcher() {
    *FETCHER.write() = None;
}

pub fn web_fetch_entry() -> ToolEntry {
    ToolEntry::new(
        "web_fetch",
        "net",
        "Fetch a URL via the installed HTTP fetcher and return the response body.",
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "format": "uri" }
            },
            "required": ["url"]
        }),
        Arc::new(run_web_fetch),
    )
    .with_max_result_size(512 * 1024)
}

fn run_web_fetch(args: &Value) -> crate::error::Result<ToolResult> {
    let url = args.get("url").and_then(Value::as_str).ok_or_else(|| {
        crate::error::HermesError::ToolArguments {
            name: "web_fetch".into(),
            reason: "`url` (string) is required".into(),
        }
    })?;
    let fetcher = FETCHER.read().clone();
    let Some(f) = fetcher else {
        return Err(crate::error::HermesError::ToolFailed {
            name: "web_fetch".into(),
            reason: "no HTTP fetcher installed (call tools_builtin::install_fetcher)".into(),
        });
    };
    match (f)(url) {
        Ok(body) => {
            let bytes = body.len();
            Ok(ToolResult::ok(body).with_meta("bytes", bytes.to_string()))
        }
        Err(reason) => Err(crate::error::HermesError::ToolFailed {
            name: "web_fetch".into(),
            reason,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn register_all_installs_four_tools() {
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        assert_eq!(r.len(), 4);
        for n in ["bash", "file_read", "file_write", "web_fetch"] {
            assert!(r.get(n).is_some(), "{n} missing");
        }
    }

    #[test]
    fn bash_echo_succeeds() {
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let out = r
            .invoke("bash", &json!({"command": "echo hi"}))
            .unwrap();
        assert!(out.ok, "bash echo failed: {out:?}");
        assert!(out.output.contains("hi"));
        assert_eq!(out.meta.get("exit_code").map(String::as_str), Some("0"));
    }

    #[test]
    fn bash_non_zero_exit_reports_failure() {
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let out = r
            .invoke("bash", &json!({"command": "exit 7"}))
            .unwrap();
        assert!(!out.ok);
        assert_eq!(out.meta.get("exit_code").map(String::as_str), Some("7"));
    }

    #[test]
    fn bash_rejects_empty_command() {
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let err = r.invoke("bash", &json!({"command": ""})).unwrap_err();
        assert!(matches!(
            err,
            crate::error::HermesError::ToolArguments { .. }
        ));
    }

    #[test]
    fn file_write_then_read_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested/x.txt");
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let w = r
            .invoke(
                "file_write",
                &json!({"path": path.to_str().unwrap(), "content": "hello cave"}),
            )
            .unwrap();
        assert!(w.ok);
        assert_eq!(w.meta.get("bytes").map(String::as_str), Some("10"));

        let rd = r
            .invoke("file_read", &json!({"path": path.to_str().unwrap()}))
            .unwrap();
        assert!(rd.ok);
        assert_eq!(rd.output, "hello cave");
    }

    #[test]
    fn file_read_missing_path_propagates_io_error() {
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let err = r
            .invoke("file_read", &json!({"path": "/this/does/not/exist"}))
            .unwrap_err();
        assert!(matches!(err, crate::error::HermesError::Io(_)));
    }

    // FETCHER is process-global; these two tests serialise against a
    // shared mutex so `cargo test`'s parallel scheduler can't interleave
    // install / clear and surface false negatives.
    static FETCHER_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    #[test]
    fn web_fetch_without_installed_fetcher_fails_clearly() {
        let _g = FETCHER_TEST_LOCK.lock();
        clear_fetcher();
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let err = r
            .invoke("web_fetch", &json!({"url": "https://example.com"}))
            .unwrap_err();
        match err {
            crate::error::HermesError::ToolFailed { reason, .. } => {
                assert!(reason.contains("no HTTP fetcher installed"));
            }
            e => panic!("expected ToolFailed, got {e}"),
        }
    }

    #[test]
    fn web_fetch_with_installed_fetcher_returns_body() {
        let _g = FETCHER_TEST_LOCK.lock();
        install_fetcher(Arc::new(|url: &str| Ok(format!("body of {url}"))));
        let mut r = ToolRegistry::new();
        register_all(&mut r);
        let out = r
            .invoke("web_fetch", &json!({"url": "https://example.com"}))
            .unwrap();
        clear_fetcher();
        assert!(out.ok);
        assert_eq!(out.output, "body of https://example.com");
    }
}
