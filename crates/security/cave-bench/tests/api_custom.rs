// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 3 — HTTP backend surface for custom frameworks + security checks.
//!
//! Mirrors kubescape's REST API that lists frameworks and accepts a custom
//! framework for evaluation (`core/cautils/getter/restapi.go`).

use cave_bench::State;
use cave_bench::api::{list_custom_frameworks, register_custom_framework};

fn sample_json() -> &'static str {
    r#"{"id":"acme","name":"Acme","version":"1.0","controls":[
        {"control_id":"A-1","name":"manual review","severity":"low","node_type":"policies","rule":{"items":[],"logic":"And","manual":true}}
    ]}"#
}

#[test]
fn test_state_has_custom_registry() {
    let s = State::default();
    assert_eq!(s.custom.count(), 0);
}

#[test]
fn test_register_custom_framework_backend() {
    let s = State::default();
    let n = register_custom_framework(&s, sample_json()).expect("register ok");
    assert_eq!(n, 1); // one control registered
    assert_eq!(s.custom.count(), 1);
    let ids = list_custom_frameworks(&s);
    assert_eq!(ids, vec!["acme".to_string()]);
}

#[test]
fn test_register_invalid_framework_errors() {
    let s = State::default();
    let bad = r#"{"id":"empty","name":"Empty","controls":[]}"#;
    assert!(register_custom_framework(&s, bad).is_err());
    assert_eq!(s.custom.count(), 0);
}

#[test]
fn test_register_duplicate_errors() {
    let s = State::default();
    register_custom_framework(&s, sample_json()).unwrap();
    assert!(register_custom_framework(&s, sample_json()).is_err());
    assert_eq!(s.custom.count(), 1);
}

#[test]
fn test_router_constructs_with_custom_routes() {
    use std::sync::Arc;
    let _r = cave_bench::router(Arc::new(State::default()));
}
