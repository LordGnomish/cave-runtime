// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Time-series database with Gorilla-style XOR compression, inverted index,
//! block-based compaction, downsampling, and configurable retention.

pub mod block;
pub mod compaction;
pub mod wal;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use parking_lot::RwLock;

use crate::model::{Labels, LabelMatcher, Sample, TimeSeries};

// ─── In-memory head chunk ───────────────────────────────────────────────────

/// A series stored in the head (hot) chunk.
#[derive(Debug, Clone)]
pub struct HeadSeries {
    pub labels: Labels,
    pub fingerprint: u64,
    /// Samples stored as (delta_ms from first_ts, xor_encoded_value).
    /// For simplicity we store raw samples; Gorilla encoding is in block::ChunkWriter.
    pub samples: Vec<Sample>,
    pub first_ts: i64,
    pub last_ts: i64,
}

impl HeadSeries {
    pub fn new(labels: Labels) -> Self {
        let fp = labels.fingerprint();
        Self { labels, fingerprint: fp, samples: Vec::new(), first_ts: i64::MAX, last_ts: i64::MIN }
    }

    pub fn append(&mut self, sample: Sample) {
        if sample.timestamp_ms < self.first_ts { self.first_ts = sample.timestamp_ms; }
        if sample.timestamp_ms > self.last_ts  { self.last_ts  = sample.timestamp_ms; }
        // Keep sorted; typically samples arrive in order.
        match self.samples.binary_search_by_key(&sample.timestamp_ms, |s| s.timestamp_ms) {
            Ok(i)  => self.samples[i] = sample,   // duplicate ts → overwrite
            Err(i) => self.samples.insert(i, sample),
        }
    }

    pub fn samples_in_range(&self, start_ms: i64, end_ms: i64) -> &[Sample] {
        let lo = self.samples.partition_point(|s| s.timestamp_ms < start_ms);
        let hi = self.samples.partition_point(|s| s.timestamp_ms <= end_ms);
        &self.samples[lo..hi]
    }

    pub fn latest_at(&self, ts_ms: i64, lookback_ms: i64) -> Option<Sample> {
        let window = self.samples_in_range(ts_ms - lookback_ms, ts_ms);
        window.last().copied()
    }
}

// ─── Inverted index ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct InvertedIndex {
    /// label name → label value → set of fingerprints
    index: HashMap<String, HashMap<String, HashSet<u64>>>,
}

impl InvertedIndex {
    fn add(&mut self, labels: &Labels, fp: u64) {
        for (name, value) in labels.iter() {
            self.index.entry(name.to_string()).or_default()
                .entry(value.to_string()).or_default()
                .insert(fp);
        }
    }

    fn remove(&mut self, labels: &Labels, fp: u64) {
        for (name, value) in labels.iter() {
            if let Some(vals) = self.index.get_mut(name) {
                if let Some(fps) = vals.get_mut(value) {
                    fps.remove(&fp);
                }
            }
        }
    }

    fn label_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.index.keys().cloned().collect();
        names.sort();
        names
    }

    fn label_values(&self, name: &str) -> Vec<String> {
        let mut vals: Vec<_> = self.index.get(name).map(|m| m.keys().cloned().collect()).unwrap_or_default();
        vals.sort();
        vals
    }

    /// Return fingerprints matching all matchers (intersection).
    fn matching(&self, matchers: &[LabelMatcher]) -> HashSet<u64> {
        if matchers.is_empty() {
            // Return all fps
            let mut all = HashSet::new();
            for vals in self.index.values() {
                for fps in vals.values() {
                    all.extend(fps);
                }
            }
            return all;
        }

        let mut result: Option<HashSet<u64>> = None;

        for matcher in matchers {
            use crate::model::MatchOp;
            let candidates: HashSet<u64> = match &matcher.op {
                MatchOp::Equal => {
                    self.index.get(&matcher.name)
                        .and_then(|vals| vals.get(&matcher.value))
                        .cloned()
                        .unwrap_or_default()
                }
                MatchOp::NotEqual => {
                    // All fps that do NOT have this exact label value
                    let exclude: HashSet<u64> = self.index.get(&matcher.name)
                        .and_then(|vals| vals.get(&matcher.value))
                        .cloned()
                        .unwrap_or_default();
                    let mut all = HashSet::new();
                    for vals in self.index.values() {
                        for fps in vals.values() { all.extend(fps); }
                    }
                    all.difference(&exclude).cloned().collect()
                }
                MatchOp::RegexMatch | MatchOp::RegexNotMatch => {
                    // Enumerate all values for the label name
                    let mut matched_fps = HashSet::new();
                    if let Some(vals) = self.index.get(&matcher.name) {
                        for (val, fps) in vals {
                            let lbl = Labels::from_pairs([(&matcher.name, val.as_str())]);
                            if matcher.matches(&lbl) {
                                matched_fps.extend(fps);
                            }
                        }
                    }
                    if matcher.op == MatchOp::RegexNotMatch {
                        let mut all = HashSet::new();
                        for vals in self.index.values() {
                            for fps in vals.values() { all.extend(fps); }
                        }
                        all.difference(&matched_fps).cloned().collect()
                    } else {
                        matched_fps
                    }
                }
            };

            result = Some(match result {
                None    => candidates,
                Some(r) => r.intersection(&candidates).cloned().collect(),
            });
        }

        result.unwrap_or_default()
    }
}

// ─── Sealed blocks ───────────────────────────────────────────────────────────

/// A sealed immutable block (compacted from head, or loaded from disk).
#[derive(Debug, Clone)]
pub struct Block {
    pub min_ts: i64,
    pub max_ts: i64,
    pub series: Vec<HeadSeries>,
}

impl Block {
    pub fn select(&self, matchers: &[LabelMatcher], start_ms: i64, end_ms: i64) -> Vec<(Labels, Vec<Sample>)> {
        self.series.iter()
            .filter(|s| s.last_ts >= start_ms && s.first_ts <= end_ms)
            .filter(|s| matchers.iter().all(|m| m.matches(&s.labels)))
            .map(|s| (s.labels.clone(), s.samples_in_range(start_ms, end_ms).to_vec()))
            .filter(|(_, samples)| !samples.is_empty())
            .collect()
    }
}

// ─── TSDB ────────────────────────────────────────────────────────────────────

/// Configuration for the TSDB.
#[derive(Debug, Clone)]
pub struct TsdbConfig {
    /// Maximum age of samples to retain (milliseconds).
    pub retention_ms: i64,
    /// Head chunk is flushed to a block when it exceeds this many milliseconds.
    pub block_duration_ms: i64,
    /// Downsampling resolutions in milliseconds.
    pub downsample_resolutions: Vec<i64>,
}

impl Default for TsdbConfig {
    fn default() -> Self {
        Self {
            retention_ms:        15 * 24 * 60 * 60 * 1000, // 15 days
            block_duration_ms:    2 * 60 * 60 * 1000,       // 2 h
            downsample_resolutions: vec![
                5  * 60 * 1000,  // 5 m
                60 * 60 * 1000,  // 1 h
                24 * 60 * 60 * 1000, // 1 d
            ],
        }
    }
}

pub struct Tsdb {
    config: TsdbConfig,
    /// Hot head chunk.
    head: Arc<RwLock<HashMap<u64, HeadSeries>>>,
    /// Inverted label index (covers head + all blocks).
    index: Arc<RwLock<InvertedIndex>>,
    /// Sealed immutable blocks.
    blocks: Arc<RwLock<Vec<Block>>>,
}

impl Tsdb {
    pub fn new(config: TsdbConfig) -> Self {
        Self {
            config,
            head:   Arc::new(RwLock::new(HashMap::new())),
            index:  Arc::new(RwLock::new(InvertedIndex::default())),
            blocks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Append a sample.  Creates the series if it does not exist.
    pub fn append(&self, labels: Labels, sample: Sample) {
        let fp = labels.fingerprint();
        let mut head = self.head.write();
        if !head.contains_key(&fp) {
            let mut idx = self.index.write();
            idx.add(&labels, fp);
            head.insert(fp, HeadSeries::new(labels));
        }
        head.get_mut(&fp).unwrap().append(sample);
    }

    /// Append multiple samples (batch).
    pub fn append_many(&self, ts: &TimeSeries) {
        for sample in &ts.samples {
            self.append(ts.labels.clone(), *sample);
        }
    }

    /// Select series matching matchers within [start_ms, end_ms].
    pub fn select(&self, matchers: &[LabelMatcher], start_ms: i64, end_ms: i64) -> Vec<(Labels, Vec<Sample>)> {
        let fps = self.index.read().matching(matchers);
        let head = self.head.read();
        let blocks = self.blocks.read();

        let mut out: HashMap<u64, (Labels, Vec<Sample>)> = HashMap::new();

        // Head
        for fp in &fps {
            if let Some(series) = head.get(fp) {
                let samps = series.samples_in_range(start_ms, end_ms).to_vec();
                if !samps.is_empty() {
                    out.insert(*fp, (series.labels.clone(), samps));
                }
            }
        }

        // Sealed blocks
        for block in blocks.iter() {
            for (labels, samps) in block.select(matchers, start_ms, end_ms) {
                let fp = labels.fingerprint();
                if fps.contains(&fp) {
                    out.entry(fp).or_insert_with(|| (labels, Vec::new())).1.extend(samps);
                }
            }
        }

        // Sort each series by timestamp and deduplicate.
        let mut result: Vec<(Labels, Vec<Sample>)> = out.into_values().collect();
        for (_, samps) in &mut result {
            samps.sort_by_key(|s| s.timestamp_ms);
            samps.dedup_by_key(|s| s.timestamp_ms);
        }
        result
    }

    /// Get the latest sample for each series at `ts_ms` within a lookback window.
    pub fn select_at(&self, matchers: &[LabelMatcher], ts_ms: i64, lookback_ms: i64) -> Vec<(Labels, Sample)> {
        let range = self.select(matchers, ts_ms - lookback_ms, ts_ms);
        range.into_iter()
            .filter_map(|(labels, samps)| samps.last().copied().map(|s| (labels, s)))
            .collect()
    }

    /// All label names present in the index (optionally filtered by matchers).
    pub fn label_names(&self, matchers: &[LabelMatcher]) -> Vec<String> {
        if matchers.is_empty() {
            return self.index.read().label_names();
        }
        let fps = self.index.read().matching(matchers);
        let head = self.head.read();
        let mut names = HashSet::new();
        for fp in fps {
            if let Some(s) = head.get(&fp) {
                for (k, _) in s.labels.iter() { names.insert(k.to_string()); }
            }
        }
        let mut v: Vec<_> = names.into_iter().collect();
        v.sort();
        v
    }

    /// All values for a label name (optionally filtered by matchers).
    pub fn label_values(&self, name: &str, matchers: &[LabelMatcher]) -> Vec<String> {
        if matchers.is_empty() {
            return self.index.read().label_values(name);
        }
        let fps = self.index.read().matching(matchers);
        let head = self.head.read();
        let mut vals = HashSet::new();
        for fp in fps {
            if let Some(s) = head.get(&fp) {
                if let Some(v) = s.labels.get(name) { vals.insert(v.to_string()); }
            }
        }
        let mut v: Vec<_> = vals.into_iter().collect();
        v.sort();
        v
    }

    /// All series matching matchers (labels only, no samples).
    pub fn series_for(&self, matchers: &[LabelMatcher]) -> Vec<Labels> {
        let fps = self.index.read().matching(matchers);
        let head = self.head.read();
        fps.into_iter()
            .filter_map(|fp| head.get(&fp).map(|s| s.labels.clone()))
            .filter(|labels| matchers.iter().all(|m| m.matches(labels)))
            .collect()
    }

    /// Remove samples older than `retention_ms`.
    pub fn enforce_retention(&self) {
        let cutoff = now_ms() - self.config.retention_ms;
        let mut head = self.head.write();
        let mut index = self.index.write();
        let mut to_remove = Vec::new();

        for (fp, series) in head.iter_mut() {
            series.samples.retain(|s| s.timestamp_ms >= cutoff);
            if series.samples.is_empty() {
                to_remove.push(*fp);
            } else {
                series.first_ts = series.samples[0].timestamp_ms;
                series.last_ts  = series.samples.last().unwrap().timestamp_ms;
            }
        }

        for fp in to_remove {
            if let Some(series) = head.remove(&fp) {
                index.remove(&series.labels, fp);
            }
        }

        // Also remove expired blocks.
        let mut blocks = self.blocks.write();
        blocks.retain(|b| b.max_ts >= cutoff);
    }

    /// Flush head series older than `block_duration_ms` into a sealed block.
    pub fn compact(&self) {
        let threshold = now_ms() - self.config.block_duration_ms;
        let mut head = self.head.write();

        let flush_fps: Vec<u64> = head.iter()
            .filter(|(_, s)| s.last_ts < threshold)
            .map(|(fp, _)| *fp)
            .collect();

        if flush_fps.is_empty() { return; }

        let mut block_series = Vec::new();
        for fp in flush_fps {
            if let Some(series) = head.remove(&fp) {
                block_series.push(series);
            }
        }

        if block_series.is_empty() { return; }

        let min_ts = block_series.iter().map(|s| s.first_ts).min().unwrap_or(0);
        let max_ts = block_series.iter().map(|s| s.last_ts).max().unwrap_or(0);

        let block = Block { min_ts, max_ts, series: block_series };
        self.blocks.write().push(block);
    }

    /// Downsample blocks at a given resolution.
    pub fn downsample(&self, resolution_ms: i64) -> Vec<(Labels, Vec<Sample>)> {
        let blocks = self.blocks.read();
        let mut out = Vec::new();
        for block in blocks.iter() {
            for series in &block.series {
                let downsampled = compaction::downsample_series(&series.samples, resolution_ms);
                out.push((series.labels.clone(), downsampled));
            }
        }
        out
    }

    /// Spawn background tasks for retention enforcement and compaction.
    pub fn start_background_tasks(self: Arc<Self>) {
        let db = Arc::clone(&self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                db.enforce_retention();
                db.compact();
            }
        });
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl Default for Tsdb {
    fn default() -> Self {
        Self::new(TsdbConfig::default())
    }
}

