// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement step 4 — hot-patch ingestion: checksum-validated staging,
//! apply with rollback snapshot, and an audit trail.

use cave_agent::hotpatch::{sha256_of, Patch, PatchRegistry};
use cave_agent::AgentError;
use serde_json::json;

fn seeded() -> PatchRegistry {
    let mut r = PatchRegistry::new();
    r.set("temperature", json!(0.7));
    r
}

#[test]
fn checksum_is_deterministic() {
    assert_eq!(sha256_of(&json!({"a":1,"b":2})), sha256_of(&json!({"b":2,"a":1})));
    assert_ne!(sha256_of(&json!(1)), sha256_of(&json!(2)));
}

#[test]
fn stage_valid_patch_does_not_apply_yet() {
    let mut reg = seeded();
    let p = Patch::create("p1", "temperature", json!(0.5));
    reg.stage(p).unwrap();
    // active value unchanged until apply
    assert_eq!(reg.get("temperature"), Some(&json!(0.7)));
    assert!(reg.is_staged("p1"));
}

#[test]
fn tampered_patch_is_rejected_on_stage() {
    let mut reg = seeded();
    let mut p = Patch::create("p2", "temperature", json!(0.5));
    p.value = json!(9.9); // mutate payload but leave the stale checksum
    let err = reg.stage(p).unwrap_err();
    assert!(matches!(err, AgentError::PatchRejected(_)));
}

#[test]
fn apply_promotes_staged_value() {
    let mut reg = seeded();
    reg.stage(Patch::create("p3", "temperature", json!(0.4))).unwrap();
    reg.apply("p3").unwrap();
    assert_eq!(reg.get("temperature"), Some(&json!(0.4)));
    assert!(!reg.is_staged("p3"));
}

#[test]
fn apply_unknown_id_is_rejected() {
    let mut reg = seeded();
    assert!(matches!(reg.apply("ghost"), Err(AgentError::PatchRejected(_))));
}

#[test]
fn rollback_restores_previous_value() {
    let mut reg = seeded();
    reg.stage(Patch::create("p4", "temperature", json!(0.2))).unwrap();
    reg.apply("p4").unwrap();
    assert_eq!(reg.get("temperature"), Some(&json!(0.2)));
    reg.rollback("temperature").unwrap();
    assert_eq!(reg.get("temperature"), Some(&json!(0.7)));
}

#[test]
fn rollback_without_history_is_rejected() {
    let mut reg = seeded();
    assert!(matches!(reg.rollback("temperature"), Err(AgentError::PatchRejected(_))));
}

#[test]
fn audit_trail_records_each_action() {
    let mut reg = seeded();
    reg.stage(Patch::create("p5", "temperature", json!(0.3))).unwrap();
    reg.apply("p5").unwrap();
    reg.rollback("temperature").unwrap();
    let actions: Vec<&str> = reg.audit().iter().map(|a| a.action.as_str()).collect();
    assert_eq!(actions, ["stage", "apply", "rollback"]);
    assert_eq!(reg.audit()[1].key, "temperature");
}

#[test]
fn new_key_patch_applies_then_rolls_back_to_absent() {
    let mut reg = PatchRegistry::new();
    reg.stage(Patch::create("n1", "new_knob", json!(42))).unwrap();
    reg.apply("n1").unwrap();
    assert_eq!(reg.get("new_knob"), Some(&json!(42)));
    reg.rollback("new_knob").unwrap();
    assert_eq!(reg.get("new_knob"), None);
}
