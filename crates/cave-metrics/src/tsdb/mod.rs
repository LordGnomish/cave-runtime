//! Time-series database (TSDB).

#![allow(dead_code)]

pub mod compaction;
pub mod wal;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use crate::error::MetricsResult;
use crate::model::{Labels, LabelMatcher, Sample, TimeSeries, Timestamp, Value};
use self::wal::Wal;

#[derive(Debug, Clone)]
pub struct TsdbConfig {
    pub retention_ms: i64,
    pub wal_dir: Option<PathBuf>,
    pub max_block_samples: usize,
}

impl Default for TsdbConfig {
    fn default() -> Self {
        Self {
            retention_ms: 15 * 24 * 3600 * 1000, // 15 days
            wal_dir: None,
            max_block_samples: 120,
        }
    }
}

struct SeriesEntry {
    labels: Labels,
    samples: BTreeMap<Timestamp, Value>,
}

struct Inner {
    series: HashMap<u64, SeriesEntry>,
    // label_name=label_value → set of fingerprints
    label_index: HashMap<String, HashSet<u64>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            series: HashMap::new(),
            label_index: HashMap::new(),
        }
    }
}

pub struct Tsdb {
    inner: RwLock<Inner>,
    config: TsdbConfig,
    wal: Wal,
}

impl Tsdb {
    pub fn new(config: TsdbConfig) -> MetricsResult<Self> {
        let wal = Wal::new(config.wal_dir.as_deref())?;
        let mut inner = Inner::new();

        // Replay WAL if it exists
        if let Some(ref dir) = config.wal_dir {
            let wal_path = dir.join("wal.log");
            let records = Wal::replay(&wal_path)?;
            for record in records {
                match record {
                    wal::WalRecord::Meta { fp, labels } => {
                        inner.series.entry(fp).or_insert_with(|| SeriesEntry {
                            labels: labels.clone(),
                            samples: BTreeMap::new(),
                        });
                        for (k, v) in &labels.0 {
                            let key = format!("{}={}", k, v);
                            inner.label_index.entry(key).or_default().insert(fp);
                        }
                    }
                    wal::WalRecord::Sample { fp, ts, v } => {
                        if let Some(entry) = inner.series.get_mut(&fp) {
                            entry.samples.insert(ts, v);
                        }
                    }
                }
            }
        }

        Ok(Self {
            inner: RwLock::new(inner),
            config,
            wal,
        })
    }

    pub fn append(&self, labels: Labels, ts: Timestamp, value: Value) -> MetricsResult<()> {
        let fp = labels.fingerprint();
        {
            let inner = self.inner.read();
            if !inner.series.contains_key(&fp) {
                drop(inner);
                // Write meta to WAL before inserting
                self.wal.append_meta(fp, &labels)?;
                let mut inner = self.inner.write();
                if !inner.series.contains_key(&fp) {
                    // Build label index entries
                    for (k, v) in &labels.0 {
                        let key = format!("{}={}", k, v);
                        inner.label_index.entry(key).or_default().insert(fp);
                    }
                    inner.series.insert(fp, SeriesEntry {
                        labels,
                        samples: BTreeMap::new(),
                    });
                }
            }
        }
        // Write sample to WAL
        self.wal.append_sample(fp, ts, value)?;
        let mut inner = self.inner.write();
        if let Some(entry) = inner.series.get_mut(&fp) {
            entry.samples.insert(ts, value);
        }
        Ok(())
    }

    pub fn select(
        &self,
        matchers: &[LabelMatcher],
        start: Timestamp,
        end: Timestamp,
    ) -> Vec<TimeSeries> {
        let inner = self.inner.read();
        let fps = Self::fingerprints_for_matchers(&inner, matchers);
        fps.into_iter()
            .filter_map(|fp| {
                let entry = inner.series.get(&fp)?;
                let samples: Vec<Sample> = entry
                    .samples
                    .range(start..=end)
                    .map(|(&t, &v)| Sample { timestamp: t, value: v })
                    .collect();
                if samples.is_empty() {
                    None
                } else {
                    Some(TimeSeries {
                        labels: entry.labels.clone(),
                        samples,
                    })
                }
            })
            .collect()
    }

    /// Select the latest sample at or before `ts` within lookback window.
    pub fn select_at(
        &self,
        matchers: &[LabelMatcher],
        ts: Timestamp,
    ) -> Vec<(Labels, Sample)> {
        let inner = self.inner.read();
        let fps = Self::fingerprints_for_matchers(&inner, matchers);
        fps.into_iter()
            .filter_map(|fp| {
                let entry = inner.series.get(&fp)?;
                // Get the latest sample with timestamp <= ts
                let (&t, &v) = entry.samples.range(..=ts).next_back()?;
                Some((entry.labels.clone(), Sample { timestamp: t, value: v }))
            })
            .collect()
    }

    pub fn label_names(&self) -> Vec<String> {
        let inner = self.inner.read();
        let mut names: HashSet<String> = HashSet::new();
        for entry in inner.series.values() {
            for k in entry.labels.0.keys() {
                names.insert(k.clone());
            }
        }
        let mut v: Vec<String> = names.into_iter().collect();
        v.sort();
        v
    }

    pub fn label_values(&self, name: &str) -> Vec<String> {
        let inner = self.inner.read();
        let mut values: HashSet<String> = HashSet::new();
        for entry in inner.series.values() {
            if let Some(v) = entry.labels.get(name) {
                values.insert(v.to_string());
            }
        }
        let mut v: Vec<String> = values.into_iter().collect();
        v.sort();
        v
    }

    pub fn series_for(&self, matchers: &[LabelMatcher]) -> Vec<Labels> {
        let inner = self.inner.read();
        let fps = Self::fingerprints_for_matchers(&inner, matchers);
        fps.into_iter()
            .filter_map(|fp| inner.series.get(&fp).map(|e| e.labels.clone()))
            .collect()
    }

    pub fn enforce_retention(&self, now_ms: Timestamp) {
        let cutoff = now_ms - self.config.retention_ms;
        let mut inner = self.inner.write();
        for entry in inner.series.values_mut() {
            compaction::enforce_retention(&mut entry.samples, cutoff);
        }
    }

    /// Run periodic retention enforcement in a background task.
    pub fn start_retention_task(self: Arc<Self>) {
        let interval = std::time::Duration::from_secs(60);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let now_ms = chrono::Utc::now().timestamp_millis();
                self.enforce_retention(now_ms);
            }
        });
    }

    fn fingerprints_for_matchers(inner: &Inner, matchers: &[LabelMatcher]) -> Vec<u64> {
        // Try to narrow by an equality matcher first
        let mut candidate_fps: Option<HashSet<u64>> = None;
        for m in matchers {
            if let LabelMatcher::Equal { name, value } = m {
                let key = format!("{}={}", name, value);
                let set = inner.label_index.get(&key).cloned().unwrap_or_default();
                candidate_fps = Some(match candidate_fps {
                    None => set,
                    Some(existing) => existing.intersection(&set).cloned().collect(),
                });
            }
        }
        let fps: Vec<u64> = match candidate_fps {
            Some(set) => set.into_iter().collect(),
            None => inner.series.keys().cloned().collect(),
        };
        // Filter by all matchers
        fps.into_iter()
            .filter(|fp| {
                if let Some(entry) = inner.series.get(fp) {
                    matchers.iter().all(|m| m.matches(&entry.labels))
                } else {
                    false
                }
            })
            .collect()
    }
}

impl Default for Tsdb {
    fn default() -> Self {
        Self::new(TsdbConfig::default()).expect("Failed to create default Tsdb")
    }
}
