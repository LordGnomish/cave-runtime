// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 5 (RED→GREEN): invocation history + tamper-evident audit log.

use cave_tools::audit::{AuditLog, RecordOutcome};
use cave_tools::tool::ToolResult;
use cave_tools::ToolError;
use serde_json::json;

#[test]
fn records_accumulate_in_order() {
    let mut log = AuditLog::new();
    log.record("alice", "file_read", &json!({"path": "a"}), RecordOutcome::Success);
    log.record("bob", "code_exec", &json!({"src": "1+1"}), RecordOutcome::Success);
    assert_eq!(log.len(), 2);
    assert_eq!(log.entries()[0].tool, "file_read");
    assert_eq!(log.entries()[0].user, "alice");
    assert_eq!(log.entries()[1].user, "bob");
}

#[test]
fn args_are_hashed_not_stored_verbatim() {
    let mut log = AuditLog::new();
    let rec = log.record("alice", "secret_tool", &json!({"password": "hunter2"}), RecordOutcome::Success);
    // The raw secret must not appear; a stable hash is stored instead.
    assert!(!rec.args_hash.contains("hunter2"));
    assert_eq!(rec.args_hash.len(), 64); // sha256 hex
}

#[test]
fn outcome_from_result_classifies() {
    let ok: cave_tools::Result<ToolResult> = Ok(ToolResult::text("hi"));
    assert!(matches!(RecordOutcome::from_result(&ok), RecordOutcome::Success));

    let tool_err: cave_tools::Result<ToolResult> = Ok(ToolResult::error("bad"));
    assert!(matches!(RecordOutcome::from_result(&tool_err), RecordOutcome::ToolError(_)));

    let denied: cave_tools::Result<ToolResult> =
        Err(ToolError::PermissionDenied { tool: "x".into(), reason: "no".into() });
    assert!(matches!(RecordOutcome::from_result(&denied), RecordOutcome::Rejected(_)));
}

#[test]
fn query_by_tool_and_user() {
    let mut log = AuditLog::new();
    log.record("alice", "file_read", &json!({}), RecordOutcome::Success);
    log.record("alice", "file_read", &json!({}), RecordOutcome::Success);
    log.record("bob", "web_search", &json!({}), RecordOutcome::Success);
    assert_eq!(log.by_tool("file_read").count(), 2);
    assert_eq!(log.by_user("bob").count(), 1);
    assert_eq!(log.by_user("carol").count(), 0);
}

#[test]
fn hash_chain_verifies_clean_log() {
    let mut log = AuditLog::new();
    log.record("alice", "a", &json!({}), RecordOutcome::Success);
    log.record("alice", "b", &json!({}), RecordOutcome::Success);
    log.record("alice", "c", &json!({}), RecordOutcome::Success);
    assert!(log.verify().is_ok());
    // Each entry links to its predecessor.
    assert_eq!(log.entries()[1].prev_hash, log.entries()[0].entry_hash);
}

#[test]
fn tampering_breaks_the_chain() {
    let mut log = AuditLog::new();
    log.record("alice", "a", &json!({}), RecordOutcome::Success);
    log.record("alice", "b", &json!({}), RecordOutcome::Success);
    // Mutate a recorded field after the fact.
    log.entries_mut()[0].tool = "evil".into();
    let bad = log.verify().unwrap_err();
    assert_eq!(bad, 0); // first entry is where verification fails
}
