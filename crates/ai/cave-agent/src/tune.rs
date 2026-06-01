// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement, step 2: self-tuning. Given an [`Observation`] of how the
//! runtime is behaving and the operator's [`Targets`], the policy emits
//! [`Proposal`]s that nudge the tunable [`Knobs`] back toward the targets — and
//! opportunistically spends headroom when everything is healthy.
//!
//! OpenJarvis upstream: `jarvis/improve/autotune.py`. Upstream consults a small
//! tuning LLM; the prompt+model call is scope-cut (no live model in this crate).
//! This is the deterministic guard-rail policy that bounds and sanity-checks
//! whatever a tuner proposes — and stands alone when no model is configured.

use serde::{Deserialize, Serialize};

const MAX_TOKENS_FLOOR: u32 = 256;
const MAX_TOKENS_CEIL: u32 = 8192;
const TEMP_FLOOR: f64 = 0.0;
const CONCURRENCY_FLOOR: u32 = 1;

/// The runtime knobs the policy may adjust.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Knobs {
    pub max_tokens: u32,
    pub temperature: f64,
    pub concurrency: u32,
}

/// A snapshot of observed behaviour (from [`crate::observe`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Observation {
    pub latency_p95_ms: f64,
    pub accuracy: f64,
    pub cost_usd: f64,
    pub error_rate: f64,
}

/// Operator targets the policy steers toward.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Targets {
    pub latency_budget_ms: f64,
    pub min_accuracy: f64,
    pub cost_budget_usd: f64,
}

/// A single proposed knob change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    /// Which knob (`"max_tokens"`, `"temperature"`, `"concurrency"`).
    pub knob: String,
    /// Current value (as f64 for a uniform shape).
    pub from: f64,
    /// Proposed value.
    pub to: f64,
    /// Why the policy is proposing it.
    pub rationale: String,
}

/// Derive knob-change proposals from an observation. Each knob is proposed at
/// most once. Conditions:
/// - latency or cost over budget ⇒ reduce `max_tokens` by 20% (floored);
/// - all axes healthy with comfortable headroom ⇒ raise `max_tokens` by 20%
///   (ceiled);
/// - accuracy below target ⇒ lower `temperature` by 0.1 (floored);
/// - error rate above 10% ⇒ halve `concurrency` (floored).
pub fn propose(obs: &Observation, knobs: &Knobs, targets: &Targets) -> Vec<Proposal> {
    let mut out = Vec::new();

    let latency_over = obs.latency_p95_ms > targets.latency_budget_ms;
    let cost_over = obs.cost_usd > targets.cost_budget_usd;
    let accuracy_low = obs.accuracy < targets.min_accuracy;
    let errors_high = obs.error_rate > 0.10;

    let headroom = obs.latency_p95_ms < 0.5 * targets.latency_budget_ms
        && obs.accuracy >= targets.min_accuracy
        && obs.cost_usd < 0.5 * targets.cost_budget_usd
        && obs.error_rate < 0.05;

    // ── max_tokens: at most one proposal ────────────────────────────────────
    if latency_over || cost_over {
        let to = ((knobs.max_tokens as f64 * 0.8).round() as u32)
            .max(MAX_TOKENS_FLOOR);
        if to != knobs.max_tokens {
            let why = match (latency_over, cost_over) {
                (true, true) => "latency and cost both over budget",
                (true, false) => "p95 latency over budget",
                _ => "cost over budget",
            };
            out.push(Proposal {
                knob: "max_tokens".into(),
                from: knobs.max_tokens as f64,
                to: to as f64,
                rationale: format!("{why}: shrink generation budget"),
            });
        }
    } else if headroom {
        let to = ((knobs.max_tokens as f64 * 1.2).round() as u32).min(MAX_TOKENS_CEIL);
        if to != knobs.max_tokens {
            out.push(Proposal {
                knob: "max_tokens".into(),
                from: knobs.max_tokens as f64,
                to: to as f64,
                rationale: "healthy headroom: invest in longer reasoning".into(),
            });
        }
    }

    // ── temperature ─────────────────────────────────────────────────────────
    if accuracy_low {
        let to = (knobs.temperature - 0.1).max(TEMP_FLOOR);
        if (to - knobs.temperature).abs() > f64::EPSILON {
            out.push(Proposal {
                knob: "temperature".into(),
                from: knobs.temperature,
                to,
                rationale: "accuracy below target: reduce sampling noise".into(),
            });
        }
    }

    // ── concurrency ─────────────────────────────────────────────────────────
    if errors_high {
        let to = (knobs.concurrency / 2).max(CONCURRENCY_FLOOR);
        if to != knobs.concurrency {
            out.push(Proposal {
                knob: "concurrency".into(),
                from: knobs.concurrency as f64,
                to: to as f64,
                rationale: "error rate high: ease back-pressure".into(),
            });
        }
    }

    out
}

/// Fold a set of proposals onto knobs, producing the next configuration. Values
/// are re-clamped to their hard bounds defensively.
pub fn apply(knobs: &Knobs, proposals: &[Proposal]) -> Knobs {
    let mut next = *knobs;
    for p in proposals {
        match p.knob.as_str() {
            "max_tokens" => {
                next.max_tokens = (p.to.round() as i64)
                    .clamp(MAX_TOKENS_FLOOR as i64, MAX_TOKENS_CEIL as i64)
                    as u32
            }
            "temperature" => next.temperature = p.to.max(TEMP_FLOOR),
            "concurrency" => {
                next.concurrency = (p.to.round() as i64).max(CONCURRENCY_FLOOR as i64) as u32
            }
            _ => {}
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proposal_when_reduction_would_be_noop_at_floor() {
        let k = Knobs { max_tokens: MAX_TOKENS_FLOOR, temperature: 0.7, concurrency: 8 };
        let obs = Observation { latency_p95_ms: 9999.0, accuracy: 0.9, cost_usd: 0.0, error_rate: 0.0 };
        let t = Targets { latency_budget_ms: 1000.0, min_accuracy: 0.85, cost_budget_usd: 0.10 };
        // already at floor → 0.8*256 rounds to 205 < floor → clamps to 256 == from → no proposal
        assert!(propose(&obs, &k, &t).iter().all(|p| p.knob != "max_tokens"));
    }
}
