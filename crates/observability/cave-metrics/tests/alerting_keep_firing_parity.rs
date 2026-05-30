// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for Prometheus `keep_firing_for` alert semantics.
//!
//! Upstream: prometheus/prometheus `rules/alerting.go` — `KeepFiringFor` /
//! `KeepFiringSince` (PR #11827, Prometheus 2.42). After an alert's condition
//! resolves, the alert is *retained in the Firing state* for `keep_firing_for`
//! before it is removed. If the condition becomes true again within that
//! window, the alert continues firing and the grace timer is cleared.

use std::sync::Arc;

use cave_metrics::model::{Labels, Sample};
use cave_metrics::promql::Engine;
use cave_metrics::rules::{AlertRule, AlertState};
use cave_metrics::tsdb::Tsdb;

const LOOKBACK: i64 = 5 * 60 * 1000; // engine DEFAULT_LOOKBACK_MS

/// Seed `m=1` at the given sample times.
fn seeded_engine(sample_ts: &[i64]) -> Engine {
    let tsdb = Arc::new(Tsdb::default());
    for &t in sample_ts {
        tsdb.append(
            Labels::from_pairs([("__name__", "m")]),
            Sample::new(t, 1.0),
        );
    }
    Engine::new(tsdb)
}

#[test]
fn keep_firing_for_retains_then_resolves() {
    // Samples through t=120_000; the series is absent after t+LOOKBACK.
    let engine = seeded_engine(&[0, 60_000, 120_000]);
    let mut rule = AlertRule::new("Hot", "m > 0", 60_000).with_keep_firing_for(120_000);

    assert_eq!(rule.evaluate(&engine, 0).unwrap()[0].state, AlertState::Pending);
    assert_eq!(
        rule.evaluate(&engine, 60_001).unwrap()[0].state,
        AlertState::Firing
    );

    // Condition resolves (series absent beyond lookback). First inactive eval:
    let resolve_t = 120_000 + LOOKBACK + 1;
    let kept = rule.evaluate(&engine, resolve_t).unwrap();
    assert_eq!(kept.len(), 1, "alert must be retained during keep_firing_for");
    assert_eq!(kept[0].state, AlertState::Firing, "still firing within grace window");

    // Within the grace window → still firing.
    let still = rule.evaluate(&engine, resolve_t + 119_999).unwrap();
    assert_eq!(still.len(), 1);
    assert_eq!(still[0].state, AlertState::Firing);

    // Past the grace window → resolved (no longer emitted).
    let gone = rule.evaluate(&engine, resolve_t + 120_001).unwrap();
    assert!(gone.is_empty(), "alert must resolve after keep_firing_for elapses");
}

#[test]
fn keep_firing_for_zero_resolves_immediately() {
    // Default (no keep_firing_for) → resolves the instant the condition clears.
    let engine = seeded_engine(&[0, 60_000]);
    let mut rule = AlertRule::new("Hot", "m > 0", 0);

    assert_eq!(
        rule.evaluate(&engine, 0).unwrap()[0].state,
        AlertState::Firing
    );
    let resolve_t = 60_000 + LOOKBACK + 1;
    assert!(
        rule.evaluate(&engine, resolve_t).unwrap().is_empty(),
        "with keep_firing_for=0 the alert resolves immediately"
    );
}

#[test]
fn reactivation_within_window_clears_grace_timer() {
    // Sample at t=0 and again at t=600_000 (after a gap). keep_firing keeps the
    // alert firing across the gap; when it fires again the grace timer resets.
    let engine = seeded_engine(&[0, 600_000]);
    let mut rule = AlertRule::new("Hot", "m > 0", 0).with_keep_firing_for(10 * 60 * 1000);

    assert_eq!(
        rule.evaluate(&engine, 0).unwrap()[0].state,
        AlertState::Firing
    );

    // Gap: series absent between samples → kept firing.
    let gap_t = LOOKBACK + 1;
    let kept = rule.evaluate(&engine, gap_t).unwrap();
    assert_eq!(kept[0].state, AlertState::Firing);

    // Condition true again at t=600_000 (within lookback) → firing, timer cleared.
    let active_again = rule.evaluate(&engine, 600_000).unwrap();
    assert_eq!(active_again[0].state, AlertState::Firing);

    // It should NOT resolve right after, because the grace timer was cleared by
    // reactivation; only a *fresh* resolution starts a new window.
    let after = rule.evaluate(&engine, 600_000 + 1).unwrap();
    assert_eq!(after[0].state, AlertState::Firing);
}
