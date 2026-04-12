//! TSDB compaction: downsampling and merge/dedup.

#![allow(dead_code)]

use std::collections::BTreeMap;
use crate::model::{Sample, Timestamp, Value};

/// Compact a series BTreeMap if it exceeds max_samples_per_series.
/// Downsamples by averaging every N consecutive samples.
pub fn compact(
    series_map: &mut BTreeMap<Timestamp, Value>,
    max_samples_per_series: usize,
) {
    if series_map.len() <= max_samples_per_series {
        return;
    }
    let samples: Vec<(Timestamp, Value)> = series_map.iter().map(|(&t, &v)| (t, v)).collect();
    let n = samples.len();
    let factor = (n + max_samples_per_series - 1) / max_samples_per_series;
    series_map.clear();
    for chunk in samples.chunks(factor) {
        let avg_ts: i64 = chunk.iter().map(|(t, _)| t).sum::<i64>() / chunk.len() as i64;
        let avg_v: f64 = chunk.iter().map(|(_, v)| v).sum::<f64>() / chunk.len() as f64;
        series_map.insert(avg_ts, avg_v);
    }
}

/// Merge two sorted sample vecs, deduplicating by timestamp (prefer a's value).
pub fn merge_series(a: Vec<Sample>, b: Vec<Sample>) -> Vec<Sample> {
    let mut map: BTreeMap<Timestamp, Value> = BTreeMap::new();
    for s in b {
        map.insert(s.timestamp, s.value);
    }
    // a overwrites b for same timestamps
    for s in a {
        map.insert(s.timestamp, s.value);
    }
    map.into_iter().map(|(t, v)| Sample { timestamp: t, value: v }).collect()
}

/// Drop all samples older than cutoff_ms.
pub fn enforce_retention(
    series_map: &mut BTreeMap<Timestamp, Value>,
    cutoff_ms: Timestamp,
) {
    series_map.retain(|&ts, _| ts >= cutoff_ms);
}
