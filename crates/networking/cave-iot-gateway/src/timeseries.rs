// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Time-series (ts_kv) storage + aggregation.
//!
//! A pure-Rust model of the ThingsBoard `ts_kv` / `ts_kv_latest` tables — the
//! TSDB extension the runtime layers over `cave-rdbms`. Entries are stored per
//! `(entity, key)` in ascending timestamp order; queries support a half-open
//! `[start, end)` range, a `ts_kv_latest`-style most-recent lookup, and the
//! window aggregations (`AVG`/`MIN`/`MAX`/`COUNT`/`SUM`) ThingsBoard exposes
//! over its telemetry API. Non-numeric samples are skipped by numeric aggs.

use crate::KvValue;
use std::collections::HashMap;

/// A single time-series sample.
#[derive(Debug, Clone, PartialEq)]
pub struct TsEntry {
    pub ts: i64,
    pub value: KvValue,
}

/// Window aggregation function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Aggregation {
    Avg,
    Min,
    Max,
    Count,
    Sum,
}

/// Floor a timestamp to the start of its `interval`-sized bucket.
pub fn partition(ts: i64, interval: i64) -> i64 {
    if interval <= 0 {
        return ts;
    }
    (ts / interval) * interval
}

/// In-memory ts_kv store keyed by `(entity, key)`.
#[derive(Debug, Default)]
pub struct TsStore {
    series: HashMap<(String, String), Vec<TsEntry>>,
}

impl TsStore {
    pub fn new() -> TsStore {
        TsStore::default()
    }

    /// Insert a sample, keeping the series sorted by timestamp.
    pub fn insert(&mut self, entity: &str, key: &str, ts: i64, value: KvValue) {
        let s = self
            .series
            .entry((entity.to_string(), key.to_string()))
            .or_default();
        let pos = s.partition_point(|e| e.ts < ts);
        s.insert(pos, TsEntry { ts, value });
    }

    /// Most-recent sample (ts_kv_latest).
    pub fn latest(&self, entity: &str, key: &str) -> Option<(i64, &KvValue)> {
        self.series
            .get(&(entity.to_string(), key.to_string()))
            .and_then(|s| s.last())
            .map(|e| (e.ts, &e.value))
    }

    /// Samples in the half-open range `[start, end)`.
    pub fn query_range(&self, entity: &str, key: &str, start: i64, end: i64) -> Vec<TsEntry> {
        self.series
            .get(&(entity.to_string(), key.to_string()))
            .map(|s| {
                s.iter()
                    .filter(|e| e.ts >= start && e.ts < end)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Aggregate numeric samples in `[start, end)` into `interval`-sized
    /// windows, returning `(window_start, value)` for non-empty windows.
    pub fn aggregate(
        &self,
        entity: &str,
        key: &str,
        start: i64,
        end: i64,
        interval: i64,
        agg: Aggregation,
    ) -> Vec<(i64, f64)> {
        use std::collections::BTreeMap;
        let mut buckets: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
        for e in self.query_range(entity, key, start, end) {
            if let Some(v) = e.value.as_f64() {
                // Windows are anchored at the query `start` and step by
                // `interval` (ThingsBoard aggregation semantics), not floored
                // to absolute partition boundaries.
                let window = if interval > 0 {
                    start + ((e.ts - start) / interval) * interval
                } else {
                    start
                };
                buckets.entry(window).or_default().push(v);
            }
        }
        buckets
            .into_iter()
            .map(|(window, vals)| {
                let result = match agg {
                    Aggregation::Avg => vals.iter().sum::<f64>() / vals.len() as f64,
                    Aggregation::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                    Aggregation::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    Aggregation::Count => vals.len() as f64,
                    Aggregation::Sum => vals.iter().sum(),
                };
                (window, result)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KvValue;

    fn seed() -> TsStore {
        let mut s = TsStore::new();
        s.insert("dev", "temp", 1000, KvValue::Double(10.0));
        s.insert("dev", "temp", 2000, KvValue::Double(20.0));
        s.insert("dev", "temp", 3000, KvValue::Double(30.0));
        s.insert("dev", "temp", 4000, KvValue::Double(40.0));
        s
    }

    #[test]
    fn latest_returns_highest_ts() {
        let s = seed();
        let (ts, v) = s.latest("dev", "temp").unwrap();
        assert_eq!(ts, 4000);
        assert_eq!(v, &KvValue::Double(40.0));
        assert!(s.latest("dev", "missing").is_none());
    }

    #[test]
    fn out_of_order_insert_keeps_sorted_order() {
        let mut s = TsStore::new();
        s.insert("d", "k", 3000, KvValue::Long(3));
        s.insert("d", "k", 1000, KvValue::Long(1));
        s.insert("d", "k", 2000, KvValue::Long(2));
        let r = s.query_range("d", "k", 0, 10000);
        let tss: Vec<i64> = r.iter().map(|e| e.ts).collect();
        assert_eq!(tss, vec![1000, 2000, 3000]);
        // latest is still the max ts, not the last inserted.
        assert_eq!(s.latest("d", "k").unwrap().0, 3000);
    }

    #[test]
    fn query_range_is_inclusive_start_exclusive_end() {
        let s = seed();
        let r = s.query_range("dev", "temp", 2000, 4000);
        let tss: Vec<i64> = r.iter().map(|e| e.ts).collect();
        assert_eq!(tss, vec![2000, 3000]);
    }

    #[test]
    fn aggregate_avg_windows() {
        let s = seed();
        // window 2000ms: [1000,3000) avg(10,20)=15 ; [3000,5000) avg(30,40)=35
        let agg = s.aggregate("dev", "temp", 1000, 5000, 2000, Aggregation::Avg);
        assert_eq!(agg, vec![(1000, 15.0), (3000, 35.0)]);
    }

    #[test]
    fn aggregate_min_max_count_sum() {
        let s = seed();
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Min),
            vec![(1000, 10.0)]
        );
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Max),
            vec![(1000, 40.0)]
        );
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Count),
            vec![(1000, 4.0)]
        );
        assert_eq!(
            s.aggregate("dev", "temp", 1000, 5000, 4000, Aggregation::Sum),
            vec![(1000, 100.0)]
        );
    }

    #[test]
    fn non_numeric_values_excluded_from_numeric_agg() {
        let mut s = TsStore::new();
        s.insert("d", "k", 1000, KvValue::Double(10.0));
        s.insert("d", "k", 1500, KvValue::Str("oops".into()));
        s.insert("d", "k", 1800, KvValue::Double(20.0));
        let agg = s.aggregate("d", "k", 1000, 2000, 1000, Aggregation::Avg);
        assert_eq!(agg, vec![(1000, 15.0)]);
    }

    #[test]
    fn partition_bucket_floors_to_interval() {
        assert_eq!(partition(2500, 1000), 2000);
        assert_eq!(partition(1000, 1000), 1000);
        assert_eq!(partition(999, 1000), 0);
    }
}
