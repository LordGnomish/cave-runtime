// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Block compaction, downsampling, and retention helpers.

use crate::model::Sample;

/// Downsample a series to a fixed resolution by averaging samples in each bucket.
pub fn downsample_series(samples: &[Sample], resolution_ms: i64) -> Vec<Sample> {
    if samples.is_empty() || resolution_ms <= 0 {
        return samples.to_vec();
    }

    let mut buckets: std::collections::BTreeMap<i64, Vec<f64>> = std::collections::BTreeMap::new();

    for s in samples {
        let bucket_ts = (s.timestamp_ms / resolution_ms) * resolution_ms;
        buckets.entry(bucket_ts).or_default().push(s.value);
    }

    buckets
        .into_iter()
        .map(|(ts, values)| {
            let avg = values.iter().sum::<f64>() / values.len() as f64;
            Sample::new(ts, avg)
        })
        .collect()
}

/// Merge two sorted sample slices, deduplicating by timestamp (last write wins).
pub fn merge_samples(a: &[Sample], b: &[Sample]) -> Vec<Sample> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].timestamp_ms.cmp(&b[j].timestamp_ms) {
            std::cmp::Ordering::Less => {
                out.push(a[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(b[j]);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push(b[j]);
                i += 1;
                j += 1;
            } // b wins
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}

/// Remove samples older than `cutoff_ms` from a sorted slice.
pub fn apply_retention(samples: &mut Vec<Sample>, cutoff_ms: i64) {
    let pos = samples.partition_point(|s| s.timestamp_ms < cutoff_ms);
    samples.drain(..pos);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downsample() {
        let samples = vec![
            Sample::new(0, 1.0),
            Sample::new(1_000, 3.0),
            Sample::new(5_000, 2.0),
            Sample::new(6_000, 4.0),
        ];
        let ds = downsample_series(&samples, 5_000);
        assert_eq!(ds.len(), 2);
        assert_eq!(ds[0].value, 2.0); // avg(1, 3)
        assert_eq!(ds[1].value, 3.0); // avg(2, 4)
    }

    #[test]
    fn test_merge() {
        let a = vec![
            Sample::new(1, 1.0),
            Sample::new(3, 3.0),
            Sample::new(5, 5.0),
        ];
        let b = vec![
            Sample::new(2, 2.0),
            Sample::new(3, 3.5),
            Sample::new(4, 4.0),
        ];
        let m = merge_samples(&a, &b);
        assert_eq!(m.len(), 5);
        assert_eq!(m[2].value, 3.5); // b wins at ts=3
    }
}
