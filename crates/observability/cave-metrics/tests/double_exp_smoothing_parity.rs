// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for the PromQL `double_exponential_smoothing` function.
//!
//! Upstream: prometheus/prometheus `promql/functions.go`
//! `funcDoubleExponentialSmoothing` — renamed from `holt_winters` in
//! Prometheus 3.0 (PR #14930; original Holt-Winters double exponential
//! smoothing PR #2725). Signature:
//!   `double_exponential_smoothing(v range-vector, sf scalar, tf scalar)`
//! with sf (smoothing factor) and tf (trend factor) in (0,1).
//!
//! Recurrence (upstream):
//!   s1 = v[0]; b = v[1] - v[0]
//!   for i in 1..n:
//!     x = sf*v[i]; b = calcTrend(i-1); y = (1-sf)*(s1 + b); s1 = x + y
//! where calcTrend(0) = b and calcTrend(i>0) = tf*(s1-s0) + (1-tf)*b.
//! Requires at least two samples; otherwise no result is emitted.

use std::sync::Arc;

use cave_metrics::model::{Labels, QueryResult, Sample};
use cave_metrics::promql::{parse, Engine};
use cave_metrics::tsdb::{Tsdb, TsdbConfig};

fn engine_with_series(values: &[f64]) -> (Engine, i64) {
    let db = Arc::new(Tsdb::new(TsdbConfig::default()));
    for (i, v) in values.iter().enumerate() {
        db.append(
            Labels::from_pairs([("__name__", "m")]),
            Sample::new(i as i64 * 1000, *v),
        );
    }
    let last_ts = (values.len() as i64 - 1).max(0) * 1000;
    (Engine::new(db), last_ts)
}

#[test]
fn double_exponential_smoothing_matches_upstream_recurrence() {
    // values [10,12,14,18], sf=0.5, tf=0.5 → 17.0 (hand-computed from the
    // upstream recurrence: trend stays 2, s1 walks 10→12→14→17).
    let (engine, ts) = engine_with_series(&[10.0, 12.0, 14.0, 18.0]);
    let ast = parse("double_exponential_smoothing(m[1h], 0.5, 0.5)").unwrap();
    match engine.eval_instant(&ast, ts).unwrap() {
        QueryResult::InstantVector(iv) => {
            assert_eq!(iv.len(), 1, "expected one smoothed series");
            assert!((iv[0].1 - 17.0).abs() < 1e-9, "got {}", iv[0].1);
        }
        other => panic!("expected instant vector, got {other:?}"),
    }
}

#[test]
fn double_exponential_smoothing_needs_two_samples() {
    let (engine, ts) = engine_with_series(&[42.0]);
    let ast = parse("double_exponential_smoothing(m[1h], 0.5, 0.5)").unwrap();
    match engine.eval_instant(&ast, ts).unwrap() {
        QueryResult::InstantVector(iv) => {
            assert!(iv.is_empty(), "a single sample must yield no result")
        }
        other => panic!("expected instant vector, got {other:?}"),
    }
}
