// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-llm-tracker bench-runner wire.
//!
//! cave-llm-tracker publishes a deterministic 5-prompt cave-specific
//! eval bench and a `run_bench(cfg, model_id)` helper that talks to a
//! local Ollama endpoint. This module owns the *gateway-side* contract:
//! [`BenchTarget`] (which gateway URL + model id to evaluate) and
//! [`BenchPrompt`] (the 5 prompt-IDs the tracker mandates). Kept in
//! lockstep with the upstream prompt list by the integration test
//! below.
//!
//! No live HTTP here — the actual runner is in cave-llm-tracker so we
//! never pull tracker as a dependency. cave-llm-gateway only declares
//! the wire format and provides a stable, hash-stable seed set the
//! tracker can call back into.

use serde::{Deserialize, Serialize};

/// The 5 prompt IDs the cave-llm-tracker bench cycles through. Kept in
/// the exact upstream order so daily reports diff cleanly.
pub const BENCH_PROMPT_IDS: [&str; 5] = [
    "charter_v2_close",
    "parity_manifest_toml",
    "rust_refactor",
    "bilingual_reply",
    "conventional_commit",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchTarget {
    /// Gateway base URL (e.g. `http://127.0.0.1:8080`).
    pub gateway_url: String,
    /// Model id as the gateway exposes it (`ollama/llama3.1`, `claude-3-5-sonnet`, ...).
    pub model_id: String,
    /// Optional consumer/tenant tag used for rate-limit + spend accounting.
    pub consumer: Option<String>,
}

impl BenchTarget {
    pub fn new(gateway_url: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            gateway_url: gateway_url.into(),
            model_id: model_id.into(),
            consumer: None,
        }
    }

    pub fn with_consumer(mut self, consumer: impl Into<String>) -> Self {
        self.consumer = Some(consumer.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchPrompt {
    pub id: String,
    pub category: String,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchOutcome {
    pub prompt_id: String,
    pub model_id: String,
    pub elapsed_ms: u64,
    pub response_bytes: usize,
    pub quality: f32,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchSummary {
    pub target: BenchTarget,
    pub outcomes: Vec<BenchOutcome>,
    pub aggregate_quality: f32,
    pub passed: bool,
}

impl BenchSummary {
    /// Quality threshold used by `cavectl llm-gateway bench` and the
    /// `cave-llm-tracker` daily report.
    pub const QUALITY_PASS_THRESHOLD: f32 = 0.4;

    pub fn from_outcomes(target: BenchTarget, outcomes: Vec<BenchOutcome>) -> Self {
        let agg = if outcomes.is_empty() {
            0.0
        } else {
            outcomes.iter().map(|o| o.quality).sum::<f32>() / outcomes.len() as f32
        };
        let passed = agg >= Self::QUALITY_PASS_THRESHOLD
            && outcomes.iter().all(|o| !o.timed_out);
        Self {
            target,
            outcomes,
            aggregate_quality: agg,
            passed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_prompt_ids_match_cave_llm_tracker_order() {
        // Hard-pins the prompt order. If cave-llm-tracker shuffles its
        // cave_prompts() vec this test breaks loudly and the daily-
        // report diff stays stable.
        assert_eq!(
            BENCH_PROMPT_IDS,
            [
                "charter_v2_close",
                "parity_manifest_toml",
                "rust_refactor",
                "bilingual_reply",
                "conventional_commit",
            ]
        );
    }

    #[test]
    fn bench_target_builder_threads_consumer() {
        let t = BenchTarget::new("http://x:8080", "ollama/llama3.1")
            .with_consumer("ci-bot");
        assert_eq!(t.gateway_url, "http://x:8080");
        assert_eq!(t.model_id, "ollama/llama3.1");
        assert_eq!(t.consumer, Some("ci-bot".to_string()));
    }

    #[test]
    fn summary_marks_passed_when_quality_high_and_no_timeouts() {
        let t = BenchTarget::new("u", "m");
        let outcomes = (0..5)
            .map(|i| BenchOutcome {
                prompt_id: BENCH_PROMPT_IDS[i].to_string(),
                model_id: "m".into(),
                elapsed_ms: 100,
                response_bytes: 500,
                quality: 0.8,
                timed_out: false,
            })
            .collect();
        let s = BenchSummary::from_outcomes(t, outcomes);
        assert!((s.aggregate_quality - 0.8).abs() < 1e-3);
        assert!(s.passed);
    }

    #[test]
    fn summary_fails_when_any_prompt_timed_out() {
        let t = BenchTarget::new("u", "m");
        let mut outcomes: Vec<_> = (0..5)
            .map(|i| BenchOutcome {
                prompt_id: BENCH_PROMPT_IDS[i].to_string(),
                model_id: "m".into(),
                elapsed_ms: 100,
                response_bytes: 500,
                quality: 0.9,
                timed_out: false,
            })
            .collect();
        outcomes[2].timed_out = true;
        let s = BenchSummary::from_outcomes(t, outcomes);
        assert!(!s.passed);
    }

    #[test]
    fn summary_empty_aggregates_to_zero() {
        let s = BenchSummary::from_outcomes(BenchTarget::new("u", "m"), vec![]);
        assert_eq!(s.aggregate_quality, 0.0);
        assert!(!s.passed);
    }
}
