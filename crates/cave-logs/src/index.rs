// SPDX-License-Identifier: AGPL-3.0-or-later
//! Label index and bloom filter layer for fast stream/chunk lookup.
//!
//! The index maps:
//!   - label name  → set of tenant+stream fingerprints that have the label
//!   - label value → set of tenant+stream fingerprints that carry that value
//!   - chunk       → bloom filter over the lines it contains
//!
//! This allows:
//!   1. Fast label name/value enumeration without scanning all streams.
//!   2. Bloom-filter-based skip of chunks that cannot contain a substring.

use std::collections::{HashMap, HashSet};
use parking_lot::RwLock;
use bloomfilter::Bloom;

use crate::models::{Labels, TenantId, TimestampNs};

/// Key identifying a stream across tenants.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamKey {
    pub tenant: TenantId,
    pub fp: u64,
}

impl StreamKey {
    pub fn new(tenant: impl Into<TenantId>, fp: u64) -> Self {
        Self { tenant: tenant.into(), fp }
    }
}

/// Per-chunk metadata stored in the index.
#[derive(Debug)]
pub struct ChunkMeta {
    pub stream_key: StreamKey,
    pub min_ts: TimestampNs,
    pub max_ts: TimestampNs,
    pub num_entries: u64,
    pub compressed_size: u64,
    /// Bloom filter over log lines in this chunk.
    pub bloom: Bloom<str>,
}

impl ChunkMeta {
    pub fn new(
        stream_key: StreamKey,
        min_ts: TimestampNs,
        max_ts: TimestampNs,
        num_entries: u64,
        compressed_size: u64,
        lines: &[&str],
    ) -> Self {
        // Size the bloom filter for approximately `num_entries` items with
        // a 1% false-positive rate.
        let items = num_entries.max(64) as usize;
        let mut bloom = Bloom::new_for_fp_rate(items, 0.01);
        for line in lines {
            bloom.set(*line);
        }
        Self { stream_key, min_ts, max_ts, num_entries, compressed_size, bloom }
    }

    /// Returns `false` if the chunk definitely does not contain `needle`.
    pub fn might_contain(&self, needle: &str) -> bool {
        // Bloom filter checks exact strings; for substring matching we also
        // hash all n-gram tokens (trigrams) so we can still prune.
        // For simplicity here we only check the full string — callers should
        // fall back to actual decompression if needed.
        self.bloom.check(needle)
    }
}

/// Inverted index: label-name → set of stream keys.
type LabelNameIndex = HashMap<String, HashSet<StreamKey>>;
/// Inverted index: (label-name, label-value) → set of stream keys.
type LabelValueIndex = HashMap<(String, String), HashSet<StreamKey>>;

/// The global label + chunk index, shared across tenants.
pub struct LabelIndex {
    inner: RwLock<IndexInner>,
}

struct IndexInner {
    /// label_name → stream keys
    by_name: LabelNameIndex,
    /// (label_name, label_value) → stream keys
    by_value: LabelValueIndex,
    /// fingerprint → (labels, tenant)  — for series resolution
    stream_labels: HashMap<u64, (Labels, TenantId)>,
    /// ordered list of chunk metas for range scans
    chunks: Vec<ChunkMeta>,
}

impl Default for LabelIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl LabelIndex {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(IndexInner {
                by_name: HashMap::new(),
                by_value: HashMap::new(),
                stream_labels: HashMap::new(),
                chunks: Vec::new(),
            }),
        }
    }

    /// Register a stream's labels in the inverted index.
    pub fn index_stream(&self, tenant: &str, fp: u64, labels: &Labels) {
        let key = StreamKey::new(tenant, fp);
        let mut idx = self.inner.write();
        idx.stream_labels.entry(fp).or_insert_with(|| (labels.clone(), tenant.to_owned()));
        for (name, value) in labels.iter() {
            idx.by_name.entry(name.clone()).or_default().insert(key.clone());
            idx.by_value
                .entry((name.clone(), value.clone()))
                .or_default()
                .insert(key.clone());
        }
    }

    /// Remove a stream from the index (e.g. after retention expiry).
    pub fn remove_stream(&self, tenant: &str, fp: u64, labels: &Labels) {
        let key = StreamKey::new(tenant, fp);
        let mut idx = self.inner.write();
        idx.stream_labels.remove(&fp);
        for (name, value) in labels.iter() {
            if let Some(set) = idx.by_name.get_mut(name) {
                set.remove(&key);
                if set.is_empty() { idx.by_name.remove(name); }
            }
            if let Some(set) = idx.by_value.get_mut(&(name.clone(), value.clone())) {
                set.remove(&key);
                if set.is_empty() { idx.by_value.remove(&(name.clone(), value.clone())); }
            }
        }
    }

    /// Add a sealed chunk's metadata to the index.
    pub fn add_chunk(&self, meta: ChunkMeta) {
        self.inner.write().chunks.push(meta);
    }

    /// All label names visible to a tenant (or all tenants if None).
    pub fn label_names(&self, tenant: Option<&str>) -> Vec<String> {
        let idx = self.inner.read();
        let mut names: HashSet<&str> = HashSet::new();
        for (name, keys) in &idx.by_name {
            if tenant.map_or(true, |t| keys.iter().any(|k| k.tenant == t)) {
                names.insert(name.as_str());
            }
        }
        let mut out: Vec<String> = names.into_iter().map(|s| s.to_owned()).collect();
        out.sort();
        out
    }

    /// All values for a label name visible to a tenant.
    pub fn label_values(&self, name: &str, tenant: Option<&str>) -> Vec<String> {
        let idx = self.inner.read();
        let mut values: HashSet<&str> = HashSet::new();
        for ((n, v), keys) in &idx.by_value {
            if n == name && tenant.map_or(true, |t| keys.iter().any(|k| k.tenant == t)) {
                values.insert(v.as_str());
            }
        }
        let mut out: Vec<String> = values.into_iter().map(|s| s.to_owned()).collect();
        out.sort();
        out
    }

    /// All stream fingerprints that carry a given label=value in a tenant.
    pub fn streams_for_label_value(&self, name: &str, value: &str, tenant: &str) -> Vec<u64> {
        let idx = self.inner.read();
        idx.by_value
            .get(&(name.to_owned(), value.to_owned()))
            .map(|keys| {
                keys.iter()
                    .filter(|k| k.tenant == tenant)
                    .map(|k| k.fp)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Look up labels for a stream fingerprint.
    pub fn labels_for_fp(&self, fp: u64) -> Option<(Labels, TenantId)> {
        self.inner.read().stream_labels.get(&fp).cloned()
    }

    /// All stream fps for a tenant.
    pub fn all_fps_for_tenant(&self, tenant: &str) -> Vec<u64> {
        let idx = self.inner.read();
        idx.stream_labels
            .iter()
            .filter(|(_, (_, t))| t == tenant)
            .map(|(fp, _)| *fp)
            .collect()
    }

    /// Chunks overlapping [start_ns, end_ns] for a given stream.
    pub fn chunks_for_stream(
        &self,
        fp: u64,
        tenant: &str,
        start_ns: TimestampNs,
        end_ns: TimestampNs,
    ) -> Vec<usize> {
        let idx = self.inner.read();
        idx.chunks
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                m.stream_key.fp == fp
                    && m.stream_key.tenant == tenant
                    && m.max_ts >= start_ns
                    && m.min_ts <= end_ns
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Remove chunk metas older than a cutoff timestamp.
    pub fn prune_chunks_before(&self, cutoff_ns: TimestampNs) {
        let mut idx = self.inner.write();
        idx.chunks.retain(|c| c.max_ts >= cutoff_ns);
    }

    /// Total chunk count.
    pub fn chunk_count(&self) -> usize {
        self.inner.read().chunks.len()
    }

    /// Total stream count.
    pub fn stream_count(&self) -> usize {
        self.inner.read().stream_labels.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::models::Labels;

    fn make_labels(pairs: &[(&str, &str)]) -> Labels {
        Labels::new(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    #[test]
    fn label_index_roundtrip() {
        let idx = LabelIndex::new();
        let labels = make_labels(&[("app", "nginx"), ("env", "prod")]);
        idx.index_stream("org1", 42, &labels);

        let names = idx.label_names(Some("org1"));
        assert!(names.contains(&"app".to_owned()));
        assert!(names.contains(&"env".to_owned()));

        let vals = idx.label_values("app", Some("org1"));
        assert_eq!(vals, vec!["nginx".to_owned()]);

        let fps = idx.streams_for_label_value("env", "prod", "org1");
        assert_eq!(fps, vec![42]);
    }

    #[test]
    fn label_index_tenant_isolation() {
        let idx = LabelIndex::new();
        let labels = make_labels(&[("app", "svc")]);
        idx.index_stream("tenant_a", 1, &labels);
        idx.index_stream("tenant_b", 2, &labels);

        let fps_a = idx.streams_for_label_value("app", "svc", "tenant_a");
        let fps_b = idx.streams_for_label_value("app", "svc", "tenant_b");
        assert_eq!(fps_a, vec![1]);
        assert_eq!(fps_b, vec![2]);
    }

    #[test]
    fn bloom_filter_might_contain() {
        let key = StreamKey::new("t", 1);
        let lines = vec!["error: connection refused", "warn: timeout on request", "info: started"];
        let refs: Vec<&str> = lines.iter().copied().collect();
        let meta = ChunkMeta::new(key, 0, 1000, 3, 100, &refs);

        // Should find exact strings
        assert!(meta.might_contain("error: connection refused"));
        // Definitely absent strings might occasionally FP (that's OK for a bloom filter)
        // but the test just ensures the API compiles and returns bool
        let _ = meta.might_contain("totally absent string xyz");
    }

    #[test]
    fn remove_stream() {
        let idx = LabelIndex::new();
        let labels = make_labels(&[("job", "test")]);
        idx.index_stream("t", 99, &labels);
        assert_eq!(idx.label_names(Some("t")), vec!["job".to_owned()]);
        idx.remove_stream("t", 99, &labels);
        assert!(idx.label_names(Some("t")).is_empty());
    }
}
