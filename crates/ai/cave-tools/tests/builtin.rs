// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 6 (RED→GREEN): sandboxed built-in tools.

use std::sync::Arc;

use cave_tools::builtin::{
    self, Calendar, FileSandbox, Mailbox, WebResult, WebSearchProvider,
};
use cave_tools::tool::ToolRegistry;
use serde_json::json;

// ── File sandbox jail ────────────────────────────────────────────────────

#[test]
fn file_write_then_read_within_jail() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Arc::new(FileSandbox::new(dir.path()));
    let mut reg = ToolRegistry::new();
    reg.register(builtin::file_write_tool(sb.clone()));
    reg.register(builtin::file_read_tool(sb.clone()));

    let w = reg
        .invoke_validated("file_write", &json!({"path": "notes/hello.txt", "content": "hi"}))
        .unwrap();
    assert!(!w.is_error);
    let r = reg
        .invoke_validated("file_read", &json!({"path": "notes/hello.txt"}))
        .unwrap();
    assert_eq!(r.text_output(), "hi");
}

#[test]
fn file_read_rejects_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Arc::new(FileSandbox::new(dir.path()));
    let mut reg = ToolRegistry::new();
    reg.register(builtin::file_read_tool(sb));
    let err = reg
        .invoke_validated("file_read", &json!({"path": "../../etc/passwd"}))
        .unwrap_err();
    assert_eq!(err.code(), "sandbox_violation");
}

#[test]
fn file_read_rejects_absolute_path_outside_root() {
    let dir = tempfile::tempdir().unwrap();
    let sb = Arc::new(FileSandbox::new(dir.path()));
    let mut reg = ToolRegistry::new();
    reg.register(builtin::file_read_tool(sb));
    let err = reg
        .invoke_validated("file_read", &json!({"path": "/etc/hosts"}))
        .unwrap_err();
    assert_eq!(err.code(), "sandbox_violation");
}

// ── Web search (injected provider) ──────────────────────────────────────

struct FakeWeb;
impl WebSearchProvider for FakeWeb {
    fn search(&self, query: &str, limit: usize) -> Vec<WebResult> {
        (0..limit)
            .map(|i| WebResult {
                title: format!("{query} #{i}"),
                url: format!("https://example.com/{i}"),
                snippet: "…".into(),
            })
            .collect()
    }
}

#[test]
fn web_search_uses_injected_provider() {
    let mut reg = ToolRegistry::new();
    reg.register(builtin::web_search_tool(Arc::new(FakeWeb)));
    let out = reg
        .invoke_validated("web_search", &json!({"query": "rust mcp", "limit": 2}))
        .unwrap();
    let v = out.structured.unwrap();
    assert_eq!(v["results"].as_array().unwrap().len(), 2);
    assert_eq!(v["results"][0]["title"], "rust mcp #0");
}

// ── Sandboxed code execution (arithmetic evaluator) ─────────────────────

#[test]
fn code_exec_evaluates_arithmetic() {
    let mut reg = ToolRegistry::new();
    reg.register(builtin::code_exec_tool());
    let out = reg
        .invoke_validated("code_exec", &json!({"expression": "2 + 3 * (4 - 1)"}))
        .unwrap();
    assert_eq!(out.text_output(), "11");
}

#[test]
fn code_exec_rejects_non_arithmetic() {
    let mut reg = ToolRegistry::new();
    reg.register(builtin::code_exec_tool());
    // No system access — only arithmetic is permitted in the sandbox.
    let err = reg
        .invoke_validated("code_exec", &json!({"expression": "open('/etc/passwd')"}))
        .unwrap_err();
    assert_eq!(err.code(), "execution_error");
}

#[test]
fn code_exec_division_by_zero_is_error() {
    let mut reg = ToolRegistry::new();
    reg.register(builtin::code_exec_tool());
    let err = reg
        .invoke_validated("code_exec", &json!({"expression": "1 / 0"}))
        .unwrap_err();
    assert_eq!(err.code(), "execution_error");
}

// ── Calendar (in-memory sandboxed store) ────────────────────────────────

#[test]
fn calendar_add_and_list() {
    let cal = Arc::new(Calendar::new());
    let mut reg = ToolRegistry::new();
    reg.register(builtin::calendar_add_tool(cal.clone()));
    reg.register(builtin::calendar_list_tool(cal.clone()));
    reg.invoke_validated(
        "calendar_add",
        &json!({"title": "standup", "start": "2026-06-01T09:00:00Z"}),
    )
    .unwrap();
    let out = reg.invoke_validated("calendar_list", &json!({})).unwrap();
    let v = out.structured.unwrap();
    assert_eq!(v["events"].as_array().unwrap().len(), 1);
    assert_eq!(v["events"][0]["title"], "standup");
}

// ── Email (in-memory sandboxed outbox) ──────────────────────────────────

#[test]
fn email_send_queues_to_outbox_not_network() {
    let mb = Arc::new(Mailbox::new());
    let mut reg = ToolRegistry::new();
    reg.register(builtin::email_send_tool(mb.clone()));
    reg.invoke_validated(
        "email_send",
        &json!({"to": "a@example.com", "subject": "hi", "body": "yo"}),
    )
    .unwrap();
    // Nothing leaves the process: it lands in the inspectable outbox.
    assert_eq!(mb.outbox().len(), 1);
    assert_eq!(mb.outbox()[0].to, "a@example.com");
}

// ── Default registration ────────────────────────────────────────────────

#[test]
fn register_builtins_wires_the_full_set() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = builtin::BuiltinConfig {
        file_sandbox: Arc::new(FileSandbox::new(dir.path())),
        web: Arc::new(FakeWeb),
        calendar: Arc::new(Calendar::new()),
        mailbox: Arc::new(Mailbox::new()),
    };
    let mut reg = ToolRegistry::new();
    builtin::register_builtins(&mut reg, &cfg);
    for name in [
        "file_read", "file_write", "file_list", "web_search", "code_exec",
        "calendar_add", "calendar_list", "email_send",
    ] {
        assert!(reg.get(name).is_some(), "missing builtin {name}");
    }
}
