// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for PromQL `histogram_fraction()` over classic le-bucket
//! histograms.
//!
//! Upstream: prometheus/prometheus `promql/quantile.go` `BucketFraction`
//! (v3.12.0). `histogram_fraction(lower, upper, v instant-vector)` returns the
//! estimated fraction of observations between `lower` and `upper`, computed by
//! linear interpolation within the bucket that each bound falls into. Mirrors
//! the companion `histogram_quantile()` already implemented: buckets are the
//! `*_bucket` series carrying an `le` label, grouped by all other labels.
//!
//! Pinned semantics:
//!   * top bucket must be `+Inf`, else NaN
//!   * total count 0 → NaN
//!   * `lower >= upper` → 0
//!   * a bound past all finite buckets clamps its rank to the total count
//!   * linear interpolation inside the straddled bucket

use std::sync::Arc;

use cave_metrics::model::{Labels, QueryResult, Sample};
use cave_metrics::promql::functions as fns;
use cave_metrics::promql::{parse, Engine};
use cave_metrics::tsdb::{Tsdb, TsdbConfig};

const INF: f64 = f64::INFINITY;

// buckets: cumulative counts — (le=1)=1, (le=2)=3, (le=+Inf)=4
fn sample_buckets() -> Vec<(f64, f64)> {
    vec![(1.0, 1.0), (2.0, 3.0), (INF, 4.0)]
}

#[test]
fn fraction_basic_full_lower_bucket_boundary() {
    // 3 of 4 observations are <= 2  → 0.75
    let f = fns::histogram_fraction(0.0, 2.0, sample_buckets());
    assert!((f - 0.75).abs() < 1e-12, "got {f}");
}

#[test]
fn fraction_interpolates_within_bucket() {
    // upper=1.5 straddles the (1,2] bucket (2 obs): half of them counted →
    // (1 + 1) / 4 = 0.5
    let f = fns::histogram_fraction(0.0, 1.5, sample_buckets());
    assert!((f - 0.5).abs() < 1e-12, "got {f}");
}

#[test]
fn fraction_lower_ge_upper_is_zero() {
    assert_eq!(fns::histogram_fraction(2.0, 1.0, sample_buckets()), 0.0);
    assert_eq!(fns::histogram_fraction(2.0, 2.0, sample_buckets()), 0.0);
}

#[test]
fn fraction_to_infinity_is_one() {
    // everything is below +Inf → the whole population
    let f = fns::histogram_fraction(0.0, INF, sample_buckets());
    assert!((f - 1.0).abs() < 1e-12, "got {f}");
}

#[test]
fn fraction_requires_inf_top_bucket() {
    let buckets = vec![(1.0, 1.0), (2.0, 3.0)]; // no +Inf
    assert!(fns::histogram_fraction(0.0, 2.0, buckets).is_nan());
}

#[test]
fn fraction_zero_count_is_nan() {
    let buckets = vec![(1.0, 0.0), (2.0, 0.0), (INF, 0.0)];
    assert!(fns::histogram_fraction(0.0, 2.0, buckets).is_nan());
}

#[test]
fn fraction_empty_is_nan() {
    assert!(fns::histogram_fraction(0.0, 2.0, vec![]).is_nan());
}

// ─── Engine integration: groups *_bucket series by labels-except-le ──────────

fn engine_with_histogram(ts: i64) -> Engine {
    let db = Arc::new(Tsdb::new(TsdbConfig::default()));
    for (le, count) in [("1", 1.0), ("2", 3.0), ("+Inf", 4.0)] {
        db.append(
            Labels::from_pairs([("__name__", "h_bucket"), ("le", le)]),
            Sample::new(ts, count),
        );
    }
    Engine::new(db)
}

#[test]
fn engine_histogram_fraction_groups_by_le() {
    let ts = 1_700_000_000_000i64;
    let engine = engine_with_histogram(ts);
    let ast = parse("histogram_fraction(0, 2, h_bucket)").unwrap();
    match engine.eval_instant(&ast, ts).unwrap() {
        QueryResult::InstantVector(iv) => {
            assert_eq!(iv.len(), 1);
            assert!((iv[0].1 - 0.75).abs() < 1e-12, "got {}", iv[0].1);
            // result series must drop the `le` label
            assert!(iv[0].0.get("le").is_none());
        }
        other => panic!("expected instant vector, got {other:?}"),
    }
}
