// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for the PromQL `sort_by_label` / `sort_by_label_desc`
//! functions.
//!
//! Upstream: prometheus/prometheus `promql/functions.go`
//! `funcSortByLabel` / `funcSortByLabelDesc` (PR #11299, stabilized in
//! Prometheus 3.0). They reorder an instant vector by the values of the
//! given label name(s) in order; ties fall back to a comparison of the full
//! label set so the order is deterministic. `_desc` reverses the ordering.

use std::sync::Arc;

use cave_metrics::model::{Labels, QueryResult, Sample};
use cave_metrics::promql::{parse, Engine};
use cave_metrics::tsdb::Tsdb;

fn seeded_engine() -> (Engine, i64) {
    let db = Arc::new(Tsdb::default());
    let ts = 1000;
    // Insert in deliberately scrambled order; the index stores by fingerprint
    // so the natural result order is non-deterministic until sorted.
    for inst in ["b", "a", "c"] {
        db.append(
            Labels::from_pairs([("__name__", "m"), ("instance", inst)]),
            Sample::new(ts, 1.0),
        );
    }
    (Engine::new(db), ts)
}

fn instances(expr: &str) -> Vec<String> {
    let (engine, ts) = seeded_engine();
    let ast = parse(expr).unwrap();
    match engine.eval_instant(&ast, ts).unwrap() {
        QueryResult::InstantVector(iv) => iv
            .iter()
            .map(|(l, _)| l.get("instance").unwrap_or("").to_string())
            .collect(),
        other => panic!("expected instant vector, got {other:?}"),
    }
}

#[test]
fn sort_by_label_orders_ascending() {
    assert_eq!(
        instances("sort_by_label(m, \"instance\")"),
        vec!["a", "b", "c"]
    );
}

#[test]
fn sort_by_label_desc_orders_descending() {
    assert_eq!(
        instances("sort_by_label_desc(m, \"instance\")"),
        vec!["c", "b", "a"]
    );
}
