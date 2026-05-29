// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Analytics engine: aggregated trace/generation statistics, cost windows,
//! per-model metrics, and top-N cost rankings.

use crate::trace_models::TraceStatus;
use crate::trace_store::TraceStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-model statistics aggregated from all generations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStats {
    pub model: String,
    pub request_count: u64,
    pub total_tokens: u64,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_cost_usd: f64,
    pub avg_latency_ms: f64,
    pub p50_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub p99_latency_ms: u64,
    pub max_latency_ms: u64,
}

/// Workspace-wide aggregated statistics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AggregatedStats {
    pub total_traces: u64,
    pub total_generations: u64,
    pub total_spans: u64,
    pub total_scores: u64,
    pub success_rate: f64,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub model_breakdown: HashMap<String, ModelStats>,
}

/// Cost totals over a time window.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostWindow {
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
    pub total_usd: f64,
    pub by_model: HashMap<String, f64>,
    pub by_user: HashMap<String, f64>,
    pub generation_count: u64,
}

/// Compute per-model stats from a slice of generations.
/// All generations in the slice are treated as belonging to the requested model.
pub fn compute_model_stats(model: &str, generations: &[crate::trace_models::Generation]) -> ModelStats {
    let count = generations.len() as u64;
    if count == 0 {
        return ModelStats {
            model: model.to_string(),
            request_count: 0,
            total_tokens: 0,
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            total_cost_usd: 0.0,
            avg_latency_ms: 0.0,
            p50_latency_ms: 0,
            p95_latency_ms: 0,
            p99_latency_ms: 0,
            max_latency_ms: 0,
        };
    }

    let total_prompt: u64 = generations.iter().map(|g| g.prompt_tokens as u64).sum();
    let total_completion: u64 = generations.iter().map(|g| g.completion_tokens as u64).sum();
    let total_cost: f64 = generations.iter().map(|g| g.cost_usd).sum();

    let mut latencies: Vec<u64> = generations.iter().map(|g| g.latency_ms).collect();
    latencies.sort_unstable();
    let len = latencies.len();
    let avg_ms = latencies.iter().sum::<u64>() as f64 / len as f64;
    let p50 = latencies[len / 2];
    let p95 = latencies[((len as f64 * 0.95) as usize).min(len - 1)];
    let p99 = latencies[((len as f64 * 0.99) as usize).min(len - 1)];
    let max = *latencies.last().unwrap();

    ModelStats {
        model: model.to_string(),
        request_count: count,
        total_tokens: total_prompt + total_completion,
        total_prompt_tokens: total_prompt,
        total_completion_tokens: total_completion,
        total_cost_usd: total_cost,
        avg_latency_ms: avg_ms,
        p50_latency_ms: p50,
        p95_latency_ms: p95,
        p99_latency_ms: p99,
        max_latency_ms: max,
    }
}

/// Compute workspace-wide aggregated stats from a TraceStore.
pub fn compute_aggregated_stats(store: &TraceStore) -> AggregatedStats {
    let traces = store.list_traces(None, None, None, usize::MAX);
    let total_traces = traces.len() as u64;
    let success_count = traces
        .iter()
        .filter(|t| matches!(t.status, TraceStatus::Success))
        .count() as f64;
    let success_rate = if total_traces == 0 {
        0.0
    } else {
        success_count / total_traces as f64
    };

    // Gather ALL generations directly from store (not filtered by trace).
    let all_generations = store.all_generations();
    let total_generations = all_generations.len() as u64;

    // Aggregate by model.
    let mut by_model: HashMap<String, Vec<crate::trace_models::Generation>> = HashMap::new();
    for generation in &all_generations {
        by_model
            .entry(generation.model.clone())
            .or_default()
            .push(generation.clone());
    }

    let total_cost: f64 = all_generations.iter().map(|g| g.cost_usd).sum();
    let total_tokens: u64 = all_generations.iter().map(|g| g.total_tokens as u64).sum();

    let model_breakdown: HashMap<String, ModelStats> = by_model
        .iter()
        .map(|(model, gens)| {
            let stats = compute_model_stats(model, gens);
            (model.clone(), stats)
        })
        .collect();

    AggregatedStats {
        total_traces,
        total_generations,
        total_spans: 0, // spans not counted for simplicity (store doesn't expose total count)
        total_scores: 0,
        success_rate,
        total_cost_usd: total_cost,
        total_tokens,
        model_breakdown,
    }
}

/// Compute cost totals for generations within the given time window (from now - duration to now).
pub fn compute_cost_window(store: &TraceStore, window: Duration) -> CostWindow {
    let now = Utc::now();
    let window_start = now - window;

    let all_generations = store.all_generations();
    let mut total_usd = 0.0;
    let mut by_model: HashMap<String, f64> = HashMap::new();
    let by_user: HashMap<String, f64> = HashMap::new();
    let mut count = 0u64;

    for generation in &all_generations {
        if generation.start_time >= window_start {
            total_usd += generation.cost_usd;
            *by_model.entry(generation.model.clone()).or_default() += generation.cost_usd;
            count += 1;
        }
    }

    CostWindow {
        window_start: Some(window_start),
        window_end: Some(now),
        total_usd,
        by_model,
        by_user,
        generation_count: count,
    }
}

/// Return the top N models ranked by total cost descending.
pub fn top_models_by_cost(store: &TraceStore, n: usize) -> Vec<ModelStats> {
    let stats = compute_aggregated_stats(store);
    let mut models: Vec<ModelStats> = stats.model_breakdown.into_values().collect();
    models.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    models.truncate(n);
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_models::{Generation, TraceStatus, Trace};
    use crate::trace_store::TraceStore;
    use chrono::Utc;
    use uuid::Uuid;

    fn quick_trace(status: TraceStatus) -> Trace {
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

    fn quick_gen(trace_id: Uuid, model: &str, cost: f64, latency: u64) -> Generation {
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
            latency_ms: latency,
            start_time: Utc::now(),
            end_time: None,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn test_cost_window_empty() {
        let store = TraceStore::new();
        let cw = compute_cost_window(&store, Duration::hours(24));
        assert_eq!(cw.total_usd, 0.0);
        assert_eq!(cw.generation_count, 0);
    }

    #[test]
    fn test_aggregated_all_success() {
        let store = TraceStore::new();
        for _ in 0..4 {
            store.upsert_trace(quick_trace(TraceStatus::Success));
        }
        let stats = compute_aggregated_stats(&store);
        assert!((stats.success_rate - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_top_models_empty() {
        let store = TraceStore::new();
        let top = top_models_by_cost(&store, 5);
        assert!(top.is_empty());
    }
}
