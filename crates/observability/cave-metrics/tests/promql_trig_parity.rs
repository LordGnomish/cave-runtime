// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for Prometheus trigonometric & angle PromQL functions.
//!
//! Upstream: prometheus/prometheus `promql/functions.go` — the trig family
//! (`sin cos tan asin acos atan sinh cosh tanh asinh acosh atanh`), the angle
//! conversions (`deg`, `rad`) and the `pi()` constant. Introduced in
//! Prometheus 2.27 (PR #8919). Each operates element-wise over an instant
//! vector; `pi()` takes no argument and yields a scalar.

use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use cave_metrics::model::{Labels, QueryResult, Sample};
use cave_metrics::promql::{parse, Engine};
use cave_metrics::tsdb::{Tsdb, TsdbConfig};

fn engine_with_value(v: f64, ts: i64) -> Engine {
    let db = Arc::new(Tsdb::new(TsdbConfig::default()));
    db.append(
        Labels::from_pairs([("__name__", "m")]),
        Sample::new(ts, v),
    );
    Engine::new(db)
}

fn eval_scalar_of_vector(expr: &str, input: f64) -> f64 {
    let ts = 1_700_000_000_000i64;
    let engine = engine_with_value(input, ts);
    let ast = parse(expr).unwrap();
    match engine.eval_instant(&ast, ts).unwrap() {
        QueryResult::InstantVector(iv) => {
            assert_eq!(iv.len(), 1, "expected a single-series result for {expr}");
            iv[0].1
        }
        other => panic!("expected instant vector for {expr}, got {other:?}"),
    }
}

#[test]
fn sin_cos_tan_match_libm() {
    assert!((eval_scalar_of_vector("sin(m)", FRAC_PI_2) - 1.0).abs() < 1e-12);
    assert!((eval_scalar_of_vector("cos(m)", 0.0) - 1.0).abs() < 1e-12);
    assert!(eval_scalar_of_vector("tan(m)", 0.0).abs() < 1e-12);
}

#[test]
fn inverse_trig_match_libm() {
    assert!((eval_scalar_of_vector("asin(m)", 1.0) - FRAC_PI_2).abs() < 1e-12);
    assert!((eval_scalar_of_vector("acos(m)", 1.0)).abs() < 1e-12);
    assert!((eval_scalar_of_vector("atan(m)", 0.0)).abs() < 1e-12);
}

#[test]
fn hyperbolic_match_libm() {
    assert!((eval_scalar_of_vector("sinh(m)", 0.0)).abs() < 1e-12);
    assert!((eval_scalar_of_vector("cosh(m)", 0.0) - 1.0).abs() < 1e-12);
    assert!((eval_scalar_of_vector("tanh(m)", 0.0)).abs() < 1e-12);
    assert!((eval_scalar_of_vector("asinh(m)", 0.0)).abs() < 1e-12);
    assert!((eval_scalar_of_vector("acosh(m)", 1.0)).abs() < 1e-12);
    assert!((eval_scalar_of_vector("atanh(m)", 0.0)).abs() < 1e-12);
}

#[test]
fn deg_and_rad_convert_angles() {
    // rad(180) == π ; deg(π) == 180
    assert!((eval_scalar_of_vector("rad(m)", 180.0) - PI).abs() < 1e-12);
    assert!((eval_scalar_of_vector("deg(m)", PI) - 180.0).abs() < 1e-9);
}

#[test]
fn pi_is_scalar_constant() {
    let db = Arc::new(Tsdb::default());
    let engine = Engine::new(db);
    let ast = parse("pi()").unwrap();
    match engine.eval_instant(&ast, 0).unwrap() {
        QueryResult::Scalar(v) => assert!((v - PI).abs() < 1e-12),
        other => panic!("pi() must be a scalar, got {other:?}"),
    }
}
