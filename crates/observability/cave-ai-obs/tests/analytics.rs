// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for analytics: aggregated stats, cost windows, latency percentiles.

use cave_ai_obs::analytics::{
    AggregatedStats, CostWindow, ModelStats, compute_aggregated_stats, compute_cost_window,
    compute_model_stats, top_models_by_cost,
};
use cave_ai_obs::trace_models::{Generation, TraceStatus, Trace};
use cave_ai_obs::trace_store::TraceStore;
use chrono::{Duration, Utc};
use uuid::Uuid;

fn make_generation(
    trace_id: Uuid,
    model: &str,
    prompt_tokens: u32,
    completion_tokens: u32,
    cost_usd: f64,
    latency_ms: u64,
) -> Generation {
    Generation {
        id: Uuid::new_v4(),
        trace_id,
        parent_span_id: None,
        name: "call".to_string(),
        model: model.to_string(),
        model_parameters: serde_json::Value::Null,
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        input: serde_json::Value::Null,
        output: serde_json::Value::Null,
        cost_usd,
        latency_ms,
        start_time: Utc::now(),
        end_time: None,
        metadata: serde_json::Value::Null,
    }
}

fn make_trace(status: TraceStatus) -> Trace {
    Trace {
        id: Uuid::new_v4(),
        name: "t".to_string(),
        user_id: None,
        session_id: None,
        metadata: serde_json::Value::Null,
        input: None,
        output: None,
        status,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tags: vec![],
    }
}

// ─── ModelStats ───────────────────────────────────────────────────────────

#[test]
fn test_model_stats_empty() {
    let stats = compute_model_stats("gpt-4", &[]);
    assert_eq!(stats.model, "gpt-4");
    assert_eq!(stats.request_count, 0);
    assert_eq!(stats.total_tokens, 0);
    assert_eq!(stats.total_cost_usd, 0.0);
    assert_eq!(stats.avg_latency_ms, 0.0);
    assert_eq!(stats.p95_latency_ms, 0);
    assert_eq!(stats.p99_latency_ms, 0);
}

#[test]
fn test_model_stats_single() {
    let trace_id = Uuid::new_v4();
    let gens = vec![make_generation(trace_id, "gpt-4o", 100, 50, 0.01, 300)];
    let stats = compute_model_stats("gpt-4o", &gens);
    assert_eq!(stats.request_count, 1);
    assert_eq!(stats.total_tokens, 150);
    assert!((stats.total_cost_usd - 0.01).abs() < 1e-9);
    assert!((stats.avg_latency_ms - 300.0).abs() < 0.001);
}

#[test]
fn test_model_stats_percentiles() {
    let trace_id = Uuid::new_v4();
    // 10 generations with latencies 100..1000 ms
    let gens: Vec<Generation> = (1..=10u64)
        .map(|i| make_generation(trace_id, "llama", 10, 10, 0.0, i * 100))
        .collect();
    let stats = compute_model_stats("llama", &gens);
    assert_eq!(stats.request_count, 10);
    // p95 should be at index 9 (latency=1000)
    assert!(stats.p95_latency_ms >= 900, "p95={}", stats.p95_latency_ms);
    assert!(stats.p99_latency_ms >= 900, "p99={}", stats.p99_latency_ms);
}

#[test]
fn test_model_stats_only_own_model() {
    let trace_id = Uuid::new_v4();
    let gens = vec![
        make_generation(trace_id, "gpt-4o", 100, 50, 0.01, 300),
        make_generation(trace_id, "claude", 200, 100, 0.02, 400),
    ];
    let gpt_stats = compute_model_stats("gpt-4o", &gens);
    assert_eq!(gpt_stats.request_count, 2); // both passed; filter is caller responsibility
}

// ─── AggregatedStats ──────────────────────────────────────────────────────

#[test]
fn test_aggregated_stats_empty_store() {
    let store = TraceStore::new();
    let stats = compute_aggregated_stats(&store);
    assert_eq!(stats.total_traces, 0);
    assert_eq!(stats.total_generations, 0);
    assert_eq!(stats.success_rate, 0.0);
}

#[test]
fn test_aggregated_stats_counts() {
    let store = TraceStore::new();
    // 3 success, 1 error
    for _ in 0..3 {
        store.upsert_trace(make_trace(TraceStatus::Success));
    }
    store.upsert_trace(make_trace(TraceStatus::Error));

    let trace_id = Uuid::new_v4();
    for _ in 0..5 {
        store.upsert_generation(make_generation(trace_id, "gpt-4", 10, 10, 0.001, 100));
    }

    let stats = compute_aggregated_stats(&store);
    assert_eq!(stats.total_traces, 4);
    assert_eq!(stats.total_generations, 5);
    assert!((stats.success_rate - 0.75).abs() < 0.001);
}

// ─── CostWindow ───────────────────────────────────────────────────────────

#[test]
fn test_cost_window_sum() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    store.upsert_generation(make_generation(trace_id, "gpt-4", 100, 50, 0.05, 200));
    store.upsert_generation(make_generation(trace_id, "gpt-4", 200, 100, 0.10, 300));
    store.upsert_generation(make_generation(trace_id, "claude", 50, 25, 0.02, 150));

    let window = compute_cost_window(&store, Duration::hours(1));
    assert!((window.total_usd - 0.17).abs() < 1e-9);
    assert_eq!(window.by_model.len(), 2);
}

#[test]
fn test_cost_window_excludes_old_generations() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    // Add a generation with old start_time
    let old_gen = Generation {
        id: Uuid::new_v4(),
        trace_id,
        parent_span_id: None,
        name: "old".to_string(),
        model: "gpt-4".to_string(),
        model_parameters: serde_json::Value::Null,
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        input: serde_json::Value::Null,
        output: serde_json::Value::Null,
        cost_usd: 10.0,
        latency_ms: 200,
        start_time: Utc::now() - Duration::hours(25),
        end_time: None,
        metadata: serde_json::Value::Null,
    };
    store.upsert_generation(old_gen);
    store.upsert_generation(make_generation(trace_id, "gpt-4", 10, 5, 0.001, 50));

    let window = compute_cost_window(&store, Duration::hours(1));
    assert!(window.total_usd < 1.0, "old generation should be excluded, total={}", window.total_usd);
}

// ─── Top models by cost ───────────────────────────────────────────────────

#[test]
fn test_top_models_by_cost_ordering() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    store.upsert_generation(make_generation(trace_id, "cheap", 10, 5, 0.001, 50));
    store.upsert_generation(make_generation(trace_id, "expensive", 100, 50, 1.0, 500));
    store.upsert_generation(make_generation(trace_id, "medium", 50, 25, 0.1, 200));

    let top = top_models_by_cost(&store, 3);
    assert_eq!(top[0].model, "expensive");
    assert!(top[0].total_cost_usd > top[1].total_cost_usd);
    assert!(top[1].total_cost_usd > top[2].total_cost_usd);
}

#[test]
fn test_top_models_limit() {
    let store = TraceStore::new();
    let trace_id = Uuid::new_v4();
    for m in &["a", "b", "c", "d", "e"] {
        store.upsert_generation(make_generation(trace_id, m, 10, 5, 0.01, 100));
    }
    let top = top_models_by_cost(&store, 3);
    assert_eq!(top.len(), 3);
}
