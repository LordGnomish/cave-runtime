// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement step 2 — self-tuning policy: turn an observation into
//! concrete knob-change proposals and apply them.

use cave_agent::tune::{apply, propose, Knobs, Observation, Targets};

fn knobs() -> Knobs {
    Knobs { max_tokens: 2000, temperature: 0.7, concurrency: 8 }
}

fn targets() -> Targets {
    Targets { latency_budget_ms: 1000.0, min_accuracy: 0.85, cost_budget_usd: 0.10 }
}

#[test]
fn latency_over_budget_reduces_max_tokens() {
    let obs = Observation { latency_p95_ms: 2500.0, accuracy: 0.9, cost_usd: 0.02, error_rate: 0.0 };
    let props = propose(&obs, &knobs(), &targets());
    let p = props.iter().find(|p| p.knob == "max_tokens").expect("max_tokens proposal");
    assert!(p.to < p.from);
    assert_eq!(p.to, 1600.0); // round(2000 * 0.8)
}

#[test]
fn low_accuracy_lowers_temperature() {
    let obs = Observation { latency_p95_ms: 500.0, accuracy: 0.6, cost_usd: 0.02, error_rate: 0.0 };
    let props = propose(&obs, &knobs(), &targets());
    let p = props.iter().find(|p| p.knob == "temperature").expect("temperature proposal");
    assert!((p.to - 0.6).abs() < 1e-9); // 0.7 - 0.1
}

#[test]
fn high_error_rate_lowers_concurrency() {
    let obs = Observation { latency_p95_ms: 500.0, accuracy: 0.9, cost_usd: 0.02, error_rate: 0.25 };
    let props = propose(&obs, &knobs(), &targets());
    let p = props.iter().find(|p| p.knob == "concurrency").expect("concurrency proposal");
    assert!(p.to < p.from);
}

#[test]
fn cost_over_budget_reduces_max_tokens_once() {
    let obs = Observation { latency_p95_ms: 500.0, accuracy: 0.9, cost_usd: 0.50, error_rate: 0.0 };
    let props = propose(&obs, &knobs(), &targets());
    let mt: Vec<_> = props.iter().filter(|p| p.knob == "max_tokens").collect();
    assert_eq!(mt.len(), 1, "exactly one max_tokens proposal even if latency+cost both fire");
    assert!(mt[0].to < mt[0].from);
}

#[test]
fn healthy_with_headroom_increases_max_tokens() {
    let obs = Observation { latency_p95_ms: 200.0, accuracy: 0.95, cost_usd: 0.01, error_rate: 0.0 };
    let props = propose(&obs, &knobs(), &targets());
    let p = props.iter().find(|p| p.knob == "max_tokens").expect("opportunistic raise");
    assert!(p.to > p.from);
}

#[test]
fn borderline_within_targets_proposes_nothing() {
    // within budget but no big headroom (latency just under, accuracy just at target)
    let obs = Observation { latency_p95_ms: 900.0, accuracy: 0.85, cost_usd: 0.08, error_rate: 0.02 };
    let props = propose(&obs, &knobs(), &targets());
    assert!(props.is_empty(), "got {props:?}");
}

#[test]
fn apply_folds_proposals_onto_knobs() {
    let obs = Observation { latency_p95_ms: 2500.0, accuracy: 0.6, cost_usd: 0.02, error_rate: 0.0 };
    let k = knobs();
    let props = propose(&obs, &k, &targets());
    let next = apply(&k, &props);
    assert_eq!(next.max_tokens, 1600);
    assert!((next.temperature - 0.6).abs() < 1e-9);
}

#[test]
fn max_tokens_never_drops_below_floor() {
    let small = Knobs { max_tokens: 256, temperature: 0.7, concurrency: 8 };
    let obs = Observation { latency_p95_ms: 9999.0, accuracy: 0.9, cost_usd: 0.0, error_rate: 0.0 };
    let props = propose(&obs, &small, &targets());
    let next = apply(&small, &props);
    assert!(next.max_tokens >= 256);
}
