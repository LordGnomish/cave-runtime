// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral TDD coverage fills for `cave-ai-obs`, the in-memory LLM
//! observability backend mirroring Langfuse (https://github.com/langfuse/langfuse)
//! at v3.75.1.
//!
//! These tests target public, already-implemented cave functions whose branch
//! behavior was previously under-covered:
//!   - `trace_store::TraceStore::list_traces`        — combined AND filter + tag membership
//!   - `trace_store::TraceStore::get_active_prompt`  — is_active drives selection, not max version
//!   - `analytics::compute_cost_window`              — per-model breakdown + generation_count
//!   - `engine::cost_per_thousand_tokens`            — non-zero (Some) arithmetic path
//!
//! Every expected value is derived directly from the implementation logic in
//! `src/trace_store.rs`, `src/analytics.rs`, and `src/engine.rs`.

use cave_ai_obs::analytics::compute_cost_window;
use cave_ai_obs::engine::cost_per_thousand_tokens;
use cave_ai_obs::models::LlmTrace;
use cave_ai_obs::trace_models::{Generation, PromptTemplate, Trace, TraceStatus};
use cave_ai_obs::trace_store::TraceStore;
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Test fixtures ──────────────────────────────────────────────────────────

fn make_trace(user_id: Option<&str>, session_id: Option<&str>, tags: Vec<&str>) -> Trace {
    Trace {
        id: Uuid::new_v4(),
        name: "t".to_string(),
        user_id: user_id.map(|s| s.to_string()),
        session_id: session_id.map(|s| s.to_string()),
        metadata: serde_json::Value::Null,
        input: None,
        output: None,
        status: TraceStatus::Success,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tags: tags.into_iter().map(|s| s.to_string()).collect(),
    }
}

fn make_prompt(name: &str, version: u32, is_active: bool) -> PromptTemplate {
    PromptTemplate {
        id: Uuid::new_v4(),
        name: name.to_string(),
        version,
        content: "hello {{name}}".to_string(),
        variables: vec!["name".to_string()],
        is_active,
        created_at: Utc::now(),
    }
}

fn make_generation(trace_id: Uuid, model: &str, cost: f64) -> Generation {
    Generation {
        id: Uuid::new_v4(),
        trace_id,
        parent_span_id: None,
        name: "g".to_string(),
        model: model.to_string(),
        model_parameters: serde_json::Value::Null,
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        input: serde_json::Value::Null,
        output: serde_json::Value::Null,
        cost_usd: cost,
        latency_ms: 100,
        start_time: Utc::now(),
        end_time: None,
        metadata: serde_json::Value::Null,
    }
}

fn make_llm_trace(prompt: u32, completion: u32, cost: f64) -> LlmTrace {
    LlmTrace {
        id: Uuid::new_v4(),
        model: "gpt-4".to_string(),
        prompt_tokens: prompt,
        completion_tokens: completion,
        latency_ms: 100,
        cost_usd: cost,
        success: true,
        created_at: Utc::now(),
        tags: HashMap::new(),
    }
}

// ─── list_traces: combined AND filter + tag membership ──────────────────────

#[test]
fn test_list_traces_combined_and_filter() {
    // list_traces applies user_id AND session_id AND tag together; a trace must
    // satisfy *every* Some(..) filter to be returned (src/trace_store.rs:53).
    let store = TraceStore::new();
    // Matches both user=alice AND session=s1.
    store.upsert_trace(make_trace(Some("alice"), Some("s1"), vec![]));
    // Right user, wrong session.
    store.upsert_trace(make_trace(Some("alice"), Some("s2"), vec![]));
    // Wrong user, right session.
    store.upsert_trace(make_trace(Some("bob"), Some("s1"), vec![]));

    let combined = store.list_traces(Some("alice"), Some("s1"), None, 100);
    assert_eq!(
        combined.len(),
        1,
        "only the trace matching BOTH user_id AND session_id should pass"
    );
    assert_eq!(combined[0].user_id.as_deref(), Some("alice"));
    assert_eq!(combined[0].session_id.as_deref(), Some("s1"));
}

#[test]
fn test_list_traces_tag_membership() {
    // The tag filter uses tags.iter().any(|x| x == tg): a trace passes iff its
    // tags vector *contains* the requested tag (src/trace_store.rs:74).
    let store = TraceStore::new();
    store.upsert_trace(make_trace(None, None, vec!["prod", "eu"]));
    store.upsert_trace(make_trace(None, None, vec!["dev"]));

    let prod = store.list_traces(None, None, Some("prod"), 100);
    assert_eq!(prod.len(), 1, "only the trace whose tags contain 'prod' matches");
    assert!(prod[0].tags.iter().any(|t| t == "prod"));

    // A tag present on no trace yields nothing.
    let staging = store.list_traces(None, None, Some("staging"), 100);
    assert_eq!(staging.len(), 0, "no trace carries the 'staging' tag");
}

// ─── get_active_prompt: is_active drives selection, not max version ─────────

#[test]
fn test_get_active_prompt_lower_active_version_wins() {
    // get_active_prompt filters is_active THEN max_by_key(version). If only a
    // lower version is active, it must win over a higher INACTIVE version,
    // proving the is_active filter (not max version) drives selection
    // (src/trace_store.rs:231).
    let store = TraceStore::new();
    store.upsert_prompt_template(make_prompt("greeting", 1, true));
    store.upsert_prompt_template(make_prompt("greeting", 2, false));
    store.upsert_prompt_template(make_prompt("greeting", 3, false));

    let active = store
        .get_active_prompt("greeting")
        .expect("an active version exists");
    assert_eq!(
        active.version, 1,
        "version 1 is the only active one; max-version v3 is inactive"
    );
    assert!(active.is_active);
}

#[test]
fn test_get_active_prompt_max_among_active() {
    // When multiple versions are active, max_by_key(version) selects the highest
    // active one; an even-higher inactive version is ignored.
    let store = TraceStore::new();
    store.upsert_prompt_template(make_prompt("sys", 1, true));
    store.upsert_prompt_template(make_prompt("sys", 2, true));
    store.upsert_prompt_template(make_prompt("sys", 3, false));

    let active = store.get_active_prompt("sys").expect("active versions exist");
    assert_eq!(
        active.version, 2,
        "v2 is the highest ACTIVE version; inactive v3 is excluded"
    );
}

// ─── compute_cost_window: by_model breakdown + generation_count ─────────────

#[test]
fn test_cost_window_by_model_breakdown() {
    // compute_cost_window accumulates cost per model into by_model and counts
    // each in-window generation (src/analytics.rs:151). All generations here use
    // start_time = now, so all fall inside a 1-hour window.
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    store.upsert_generation(make_generation(trace_id, "gpt-4", 0.05));
    store.upsert_generation(make_generation(trace_id, "gpt-4", 0.10));
    store.upsert_generation(make_generation(trace_id, "claude", 0.02));

    let window = compute_cost_window(&store, Duration::hours(1));

    assert_eq!(window.generation_count, 3, "all three generations are in-window");
    assert_eq!(window.by_model.len(), 2, "two distinct models");
    assert!(
        (window.by_model["gpt-4"] - 0.15).abs() < 1e-9,
        "gpt-4 cost = 0.05 + 0.10"
    );
    assert!(
        (window.by_model["claude"] - 0.02).abs() < 1e-9,
        "claude cost = 0.02"
    );
    assert!(
        (window.total_usd - 0.17).abs() < 1e-9,
        "total = 0.15 + 0.02"
    );
}

// ─── cost_per_thousand_tokens: non-zero (Some) arithmetic path ──────────────

#[test]
fn test_cost_per_thousand_tokens_nonzero() {
    // total_tokens = prompt + completion = 100 + 50 = 150.
    // cost_per_thousand = cost_usd / total * 1000 = 0.0015 / 150 * 1000 = 0.01
    // (src/engine.rs:47).
    let trace = make_llm_trace(100, 50, 0.0015);
    let cptt = cost_per_thousand_tokens(&trace).expect("non-zero tokens yield Some");
    assert!(
        (cptt - 0.01).abs() < 1e-9,
        "0.0015 / 150 * 1000 == 0.01, got {cptt}"
    );
}
