// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{LlmStats, LlmTrace};

pub fn calculate_stats(model: &str, traces: &[LlmTrace]) -> LlmStats {
    let model_traces: Vec<&LlmTrace> = traces.iter().filter(|t| t.model == model).collect();
    let total = model_traces.len() as u64;
    if total == 0 {
        return LlmStats {
            model: model.to_string(),
            total_requests: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            avg_latency_ms: 0.0,
            error_rate: 0.0,
        };
    }
    let total_tokens: u64 = model_traces
        .iter()
        .map(|t| (t.prompt_tokens + t.completion_tokens) as u64)
        .sum();
    let total_cost: f64 = model_traces.iter().map(|t| t.cost_usd).sum();
    let avg_latency =
        model_traces.iter().map(|t| t.latency_ms as f64).sum::<f64>() / total as f64;
    let errors = model_traces.iter().filter(|t| !t.success).count() as f64;
    LlmStats {
        model: model.to_string(),
        total_requests: total,
        total_tokens,
        total_cost_usd: total_cost,
        avg_latency_ms: avg_latency,
        error_rate: errors / total as f64,
    }
}

pub fn total_tokens(trace: &LlmTrace) -> u32 {
    trace.prompt_tokens + trace.completion_tokens
}

pub fn filter_by_model<'a>(traces: &'a [LlmTrace], model: &str) -> Vec<&'a LlmTrace> {
    traces.iter().filter(|t| t.model == model).collect()
}

pub fn cost_per_thousand_tokens(trace: &LlmTrace) -> Option<f64> {
    let total = total_tokens(trace) as f64;
    if total == 0.0 {
        None
    } else {
        Some(trace.cost_usd / total * 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_trace(model: &str, prompt: u32, completion: u32, latency: u64, cost: f64, success: bool) -> LlmTrace {
        LlmTrace {
            id: Uuid::new_v4(),
            model: model.to_string(),
            prompt_tokens: prompt,
            completion_tokens: completion,
            latency_ms: latency,
            cost_usd: cost,
            success,
            created_at: Utc::now(),
            tags: HashMap::new(),
        }
    }

    #[test]
    fn test_calculate_stats_empty() {
        let traces: Vec<LlmTrace> = vec![];
        let stats = calculate_stats("gpt-4", &traces);
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.total_tokens, 0);
        assert_eq!(stats.error_rate, 0.0);
    }

    #[test]
    fn test_calculate_stats_basic() {
        let traces = vec![
            make_trace("gpt-4", 100, 50, 200, 0.01, true),
            make_trace("gpt-4", 200, 100, 400, 0.02, true),
            make_trace("gpt-3.5", 50, 25, 100, 0.001, true),
        ];
        let stats = calculate_stats("gpt-4", &traces);
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.total_tokens, 450);
        assert!((stats.total_cost_usd - 0.03).abs() < 0.0001);
        assert!((stats.avg_latency_ms - 300.0).abs() < 0.001);
    }

    #[test]
    fn test_total_tokens_sum() {
        let trace = make_trace("gpt-4", 100, 50, 200, 0.01, true);
        assert_eq!(total_tokens(&trace), 150);
    }

    #[test]
    fn test_filter_by_model() {
        let traces = vec![
            make_trace("gpt-4", 100, 50, 200, 0.01, true),
            make_trace("gpt-3.5", 50, 25, 100, 0.001, true),
            make_trace("gpt-4", 200, 100, 400, 0.02, true),
        ];
        let filtered = filter_by_model(&traces, "gpt-4");
        assert_eq!(filtered.len(), 2);
        for t in &filtered {
            assert_eq!(t.model, "gpt-4");
        }
    }

    #[test]
    fn test_cost_per_thousand_tokens_zero_tokens() {
        let trace = make_trace("gpt-4", 0, 0, 200, 0.01, true);
        assert_eq!(cost_per_thousand_tokens(&trace), None);
    }

    #[test]
    fn test_error_rate_calculation() {
        let traces = vec![
            make_trace("gpt-4", 100, 50, 200, 0.01, true),
            make_trace("gpt-4", 100, 50, 200, 0.01, false),
            make_trace("gpt-4", 100, 50, 200, 0.01, true),
        ];
        let stats = calculate_stats("gpt-4", &traces);
        assert!((stats.error_rate - (1.0 / 3.0)).abs() < 0.001);
    }
}
