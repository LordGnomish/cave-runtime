// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for the PromQL `ts_of_min_over_time`, `ts_of_max_over_time`
//! and `ts_of_last_over_time` range functions.
//!
//! Upstream: prometheus/prometheus `promql/functions.go` (PR #15232). They
//! return the *timestamp* (in seconds) of, respectively, the minimum, maximum
//! and last sample within the range — not the value itself.

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

fn scalar_of(expr: &str, values: &[f64]) -> f64 {
    let (engine, ts) = engine_with_series(values);
    let ast = parse(expr).unwrap();
    match engine.eval_instant(&ast, ts).unwrap() {
        QueryResult::InstantVector(iv) => {
            assert_eq!(iv.len(), 1, "expected one series for {expr}");
            iv[0].1
        }
        other => panic!("expected instant vector for {expr}, got {other:?}"),
    }
}

#[test]
fn ts_of_max_over_time_returns_timestamp_of_peak() {
    // values [3,7,2,9,1] at t = 0,1,2,3,4 s — max is 9 at t=3.
    assert_eq!(
        scalar_of("ts_of_max_over_time(m[1h])", &[3.0, 7.0, 2.0, 9.0, 1.0]),
        3.0
    );
}

#[test]
fn ts_of_min_over_time_returns_timestamp_of_trough() {
    // min is 1 at t=4 s.
    assert_eq!(
        scalar_of("ts_of_min_over_time(m[1h])", &[3.0, 7.0, 2.0, 9.0, 1.0]),
        4.0
    );
}

#[test]
fn ts_of_last_over_time_returns_timestamp_of_last_sample() {
    assert_eq!(
        scalar_of("ts_of_last_over_time(m[1h])", &[3.0, 7.0, 2.0, 9.0, 1.0]),
        4.0
    );
}
