// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for TSDB delete-series tombstones.
//!
//! Upstream: prometheus/prometheus `tsdb` Delete API + `tsdb/tombstones`.
//! `DB.Delete(mint, maxt, matchers...)` records a tombstone interval for
//! every matching series; subsequent reads must skip samples whose timestamp
//! falls inside a tombstone interval (inclusive on both ends). A delete whose
//! matchers select nothing is a no-op.

use std::sync::Arc;

use cave_metrics::model::{LabelMatcher, Labels, Sample};
use cave_metrics::tsdb::Tsdb;

fn seeded() -> Arc<Tsdb> {
    let db = Arc::new(Tsdb::default());
    for i in 0..5i64 {
        db.append(
            Labels::from_pairs([("__name__", "m")]),
            Sample::new(i * 1000, i as f64),
        );
    }
    db
}

#[test]
fn delete_removes_samples_in_interval() {
    let db = seeded();
    let m = vec![LabelMatcher::equal("__name__", "m")];
    db.delete(&m, 1000, 3000);
    let res = db.select(&m, 0, 4000);
    assert_eq!(res.len(), 1, "the series survives (endpoints remain)");
    let ts: Vec<i64> = res[0].1.iter().map(|s| s.timestamp_ms).collect();
    assert_eq!(ts, vec![0, 4000], "samples in [1000,3000] are tombstoned");
}

#[test]
fn delete_with_nonmatching_matcher_is_noop() {
    let db = seeded();
    db.delete(&[LabelMatcher::equal("__name__", "other")], 0, 9999);
    let res = db.select(&[LabelMatcher::equal("__name__", "m")], 0, 4000);
    assert_eq!(res[0].1.len(), 5, "no series matched → nothing deleted");
}
