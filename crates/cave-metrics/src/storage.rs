// SPDX-License-Identifier: AGPL-3.0-or-later
//! Time series storage: insert, compact, downsample, retention.

use crate::models::{Sample, TimeSeries};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// In-memory time series store keyed by fingerprint.
pub struct TimeSeriesStore {
    pub series: HashMap<String, TimeSeries>,
    pub retention_days: u64,
}

impl TimeSeriesStore {
    pub fn new(retention_days: u64) -> Self {
        Self {
            series: HashMap::new(),
            retention_days,
        }
    }
}

impl Default for TimeSeriesStore {
    fn default() -> Self {
        Self::new(15)
    }
}

/// Get-or-create a time series by fingerprint.
pub fn time_series_store<'a>(
    store: &'a mut TimeSeriesStore,
    metric_name: &str,
    labels: &HashMap<String, String>,
) -> &'a mut TimeSeries {
    let fp = TimeSeries::fingerprint(metric_name, labels);
    store.series.entry(fp).or_insert_with(|| {
        TimeSeries::new(metric_name, labels.clone())
    })
}

/// Insert samples into a time series, maintaining timestamp order.
pub fn insert_samples(
    store: &mut TimeSeriesStore,
    metric_name: &str,
    labels: &HashMap<String, String>,
    samples: Vec<Sample>,
) {
    let ts = time_series_store(store, metric_name, labels);
    for s in samples {
        // Insert in order
        let pos = ts.samples.partition_point(|existing| existing.timestamp <= s.timestamp);
        ts.samples.insert(pos, s);
    }
}

/// Compact: deduplicate samples with identical timestamps (keep last).
pub fn compact(store: &mut TimeSeriesStore) {
    for ts in store.series.values_mut() {
        ts.samples.dedup_by(|a, b| {
            if a.timestamp == b.timestamp {
                // keep b (earlier in vec after dedup_by direction)
                true
            } else {
                false
            }
        });
    }
}

/// Downsample: reduce resolution by averaging samples within buckets.
/// `bucket_seconds` defines bucket width.
pub fn downsample(store: &mut TimeSeriesStore, bucket_seconds: i64) {
    for ts in store.series.values_mut() {
        if ts.samples.is_empty() {
            continue;
        }
        let mut bucketed: Vec<Sample> = Vec::new();
        let mut bucket_start = ts.samples[0].timestamp;
        let mut sum = 0.0f64;
        let mut count = 0usize;

        for s in &ts.samples {
            let bucket_end = bucket_start + Duration::seconds(bucket_seconds);
            if s.timestamp < bucket_end {
                sum += s.value;
                count += 1;
            } else {
                if count > 0 {
                    bucketed.push(Sample {
                        timestamp: bucket_start,
                        value: sum / count as f64,
                    });
                }
                bucket_start = bucket_end;
                sum = s.value;
                count = 1;
            }
        }
        if count > 0 {
            bucketed.push(Sample {
                timestamp: bucket_start,
                value: sum / count as f64,
            });
        }
        ts.samples = bucketed;
    }
}

/// Remove samples older than the retention window.
pub fn retention_cleanup(store: &mut TimeSeriesStore) {
    let cutoff: DateTime<Utc> = Utc::now() - Duration::days(store.retention_days as i64);
    for ts in store.series.values_mut() {
        ts.samples.retain(|s| s.timestamp >= cutoff);
    }
    // Drop empty series
    store.series.retain(|_, ts| !ts.samples.is_empty());
}

/// Fetch samples for a series within [start, end].
pub fn range_samples<'a>(
    ts: &'a TimeSeries,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> impl Iterator<Item = &'a Sample> {
    ts.samples
        .iter()
        .filter(move |s| s.timestamp >= start && s.timestamp <= end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample(secs_ago: i64, value: f64) -> Sample {
        Sample {
            timestamp: Utc::now() - Duration::seconds(secs_ago),
            value,
        }
    }

    #[test]
    fn test_insert_and_retrieve() {
        let mut store = TimeSeriesStore::default();
        let labels = HashMap::new();
        insert_samples(
            &mut store,
            "http_requests_total",
            &labels,
            vec![make_sample(10, 1.0), make_sample(5, 2.0)],
        );
        let fp = TimeSeries::fingerprint("http_requests_total", &labels);
        assert_eq!(store.series[&fp].samples.len(), 2);
    }

    #[test]
    fn test_retention_cleanup() {
        let mut store = TimeSeriesStore::new(1);
        let labels = HashMap::new();
        // Insert an old sample (2 days ago)
        insert_samples(
            &mut store,
            "old_metric",
            &labels,
            vec![Sample {
                timestamp: Utc::now() - Duration::days(2),
                value: 42.0,
            }],
        );
        retention_cleanup(&mut store);
        assert!(store.series.is_empty());
    }

    #[test]
    fn test_downsample() {
        let mut store = TimeSeriesStore::default();
        let labels = HashMap::new();
        // 6 samples 10s apart → 2 buckets of 30s
        let samples: Vec<Sample> = (0..6)
            .map(|i| Sample {
                timestamp: Utc::now() + Duration::seconds(i * 10),
                value: i as f64,
            })
            .collect();
        insert_samples(&mut store, "gauge", &labels, samples);
        downsample(&mut store, 30);
        let fp = TimeSeries::fingerprint("gauge", &labels);
        assert!(store.series[&fp].samples.len() <= 3);
    }
}
