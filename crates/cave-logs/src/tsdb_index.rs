// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TSDB-style index (Loki v2.8+ post-shipper) — `pkg/storage/stores/tsdb`.
//!
//! Loki's TSDB index variant replaces the legacy series-store with a
//! Prometheus-style label↔series inverted index built on top of postings
//! lists. Per series we record a fingerprint, label set, and the chunk
//! references that cover the series's time range. The disk format on
//! upstream is the Prometheus TSDB block layout (`index`+`chunks`+`meta.json`),
//! but cave-logs implements the in-memory query path: posting-set
//! intersection for label matchers, chunk-ref enumeration by time range.
//!
//! Mapped surfaces:
//! * `pkg/storage/stores/tsdb/index.go` — series + postings table
//! * `pkg/storage/stores/tsdb/single_file_index.go` — block header / index
//! * `pkg/storage/stores/tsdb/sharding/sharded_chunk_refs.go` — chunk-ref slices
//! * `pkg/storage/stores/tsdb/tenant_heads.go` — per-tenant write-amp head series

use crate::models::{Labels, TenantId, TimestampNs};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Reference to a single chunk on disk (or virtual chunk in-memory).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkRef {
    pub stream_fp: u64,
    pub tenant: TenantId,
    pub min_ts: TimestampNs,
    pub max_ts: TimestampNs,
    pub kb: u32,
    pub entries: u32,
}

impl ChunkRef {
    pub fn overlaps(&self, from: TimestampNs, through: TimestampNs) -> bool {
        self.max_ts >= from && self.min_ts <= through
    }
}

/// One series entry in the TSDB index.
#[derive(Debug, Clone)]
pub struct SeriesEntry {
    pub fp: u64,
    pub labels: Labels,
    pub chunks: Vec<ChunkRef>,
}

/// TSDB-style index — Prometheus-shaped postings + chunk refs.
#[derive(Debug, Default)]
pub struct TsdbIndex {
    /// Per-tenant series map keyed by fingerprint.
    series: HashMap<TenantId, HashMap<u64, SeriesEntry>>,
    /// Postings: `(label, value) → set of fingerprints`. Per-tenant.
    postings: HashMap<TenantId, BTreeMap<(String, String), HashSet<u64>>>,
}

impl TsdbIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a series; appends `chunk` to the series's chunk-ref list.
    pub fn record_chunk(&mut self, tenant: &TenantId, labels: &Labels, chunk: ChunkRef) {
        let fp = labels.fingerprint();
        let by_tenant = self.series.entry(tenant.clone()).or_default();
        let entry = by_tenant.entry(fp).or_insert_with(|| SeriesEntry {
            fp,
            labels: labels.clone(),
            chunks: Vec::new(),
        });
        entry.chunks.push(chunk);

        let postings = self.postings.entry(tenant.clone()).or_default();
        for (k, v) in labels.iter() {
            postings
                .entry((k.clone(), v.clone()))
                .or_default()
                .insert(fp);
        }
    }

    /// Equality-match postings AND-intersection. Returns matching fingerprints.
    pub fn select(&self, tenant: &TenantId, matchers: &[(String, String)]) -> HashSet<u64> {
        let Some(postings) = self.postings.get(tenant) else {
            return HashSet::new();
        };
        if matchers.is_empty() {
            return self
                .series
                .get(tenant)
                .map(|m| m.keys().copied().collect())
                .unwrap_or_default();
        }
        let mut iter = matchers.iter();
        let first = iter.next().unwrap();
        let mut acc: HashSet<u64> = postings
            .get(&(first.0.clone(), first.1.clone()))
            .cloned()
            .unwrap_or_default();
        for m in iter {
            let next = postings
                .get(&(m.0.clone(), m.1.clone()))
                .cloned()
                .unwrap_or_default();
            acc = acc.intersection(&next).copied().collect();
            if acc.is_empty() {
                break;
            }
        }
        acc
    }

    /// Enumerate chunk refs for the time range across all series of one tenant.
    pub fn chunk_refs_in_range(
        &self,
        tenant: &TenantId,
        from: TimestampNs,
        through: TimestampNs,
    ) -> Vec<ChunkRef> {
        let Some(by_tenant) = self.series.get(tenant) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for s in by_tenant.values() {
            for c in &s.chunks {
                if c.overlaps(from, through) {
                    out.push(c.clone());
                }
            }
        }
        out
    }

    /// `LabelNames(tenant)` upstream: enumerate every label key across postings.
    pub fn label_names(&self, tenant: &TenantId) -> Vec<String> {
        let mut set: HashSet<&str> = HashSet::new();
        if let Some(postings) = self.postings.get(tenant) {
            for (k, _) in postings.keys() {
                set.insert(k.as_str());
            }
        }
        let mut v: Vec<String> = set.into_iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    /// `LabelValues(tenant, name)` upstream.
    pub fn label_values(&self, tenant: &TenantId, name: &str) -> Vec<String> {
        let mut set: HashSet<&str> = HashSet::new();
        if let Some(postings) = self.postings.get(tenant) {
            for (k, v) in postings.keys() {
                if k == name {
                    set.insert(v.as_str());
                }
            }
        }
        let mut v: Vec<String> = set.into_iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    /// Total series count across all tenants — for parity with TSDB's head stats.
    pub fn total_series(&self) -> usize {
        self.series.values().map(|m| m.len()).sum()
    }

    /// Total series count for one tenant.
    pub fn series_count(&self, tenant: &TenantId) -> usize {
        self.series.get(tenant).map(|m| m.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lbl(pairs: &[(&str, &str)]) -> Labels {
        Labels::new(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    fn chunk(fp: u64, tenant: &str, min: i64, max: i64) -> ChunkRef {
        ChunkRef {
            stream_fp: fp,
            tenant: tenant.to_string(),
            min_ts: min,
            max_ts: max,
            kb: 1,
            entries: 1,
        }
    }

    #[test]
    fn record_then_select_intersects_postings() {
        let mut idx = TsdbIndex::new();
        let a = lbl(&[("app", "api"), ("env", "prod")]);
        let b = lbl(&[("app", "api"), ("env", "dev")]);
        let c = lbl(&[("app", "db"), ("env", "prod")]);
        idx.record_chunk(&"t1".into(), &a, chunk(a.fingerprint(), "t1", 0, 100));
        idx.record_chunk(&"t1".into(), &b, chunk(b.fingerprint(), "t1", 0, 100));
        idx.record_chunk(&"t1".into(), &c, chunk(c.fingerprint(), "t1", 0, 100));

        let hits = idx.select(
            &"t1".into(),
            &[
                ("app".into(), "api".into()),
                ("env".into(), "prod".into()),
            ],
        );
        assert_eq!(hits.len(), 1);
        assert!(hits.contains(&a.fingerprint()));
    }

    #[test]
    fn select_empty_matchers_returns_all_series() {
        let mut idx = TsdbIndex::new();
        let a = lbl(&[("app", "api")]);
        idx.record_chunk(&"t1".into(), &a, chunk(a.fingerprint(), "t1", 0, 100));
        let hits = idx.select(&"t1".into(), &[]);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn chunk_refs_filter_by_time_range() {
        let mut idx = TsdbIndex::new();
        let a = lbl(&[("app", "api")]);
        idx.record_chunk(&"t1".into(), &a, chunk(a.fingerprint(), "t1", 0, 50));
        idx.record_chunk(&"t1".into(), &a, chunk(a.fingerprint(), "t1", 100, 200));
        idx.record_chunk(&"t1".into(), &a, chunk(a.fingerprint(), "t1", 300, 400));
        let refs = idx.chunk_refs_in_range(&"t1".into(), 75, 250);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].min_ts, 100);
    }

    #[test]
    fn label_names_and_values_sorted() {
        let mut idx = TsdbIndex::new();
        idx.record_chunk(
            &"t".into(),
            &lbl(&[("z", "1"), ("a", "1"), ("m", "1")]),
            chunk(1, "t", 0, 1),
        );
        let names = idx.label_names(&"t".into());
        assert_eq!(names, vec!["a", "m", "z"]);
        idx.record_chunk(&"t".into(), &lbl(&[("a", "2")]), chunk(2, "t", 0, 1));
        let mut values = idx.label_values(&"t".into(), "a");
        values.sort();
        assert_eq!(values, vec!["1", "2"]);
    }

    #[test]
    fn per_tenant_isolation_no_cross_leak() {
        let mut idx = TsdbIndex::new();
        idx.record_chunk(
            &"t1".into(),
            &lbl(&[("app", "api")]),
            chunk(1, "t1", 0, 100),
        );
        idx.record_chunk(
            &"t2".into(),
            &lbl(&[("app", "api")]),
            chunk(2, "t2", 0, 100),
        );
        let hits_t1 = idx.select(&"t1".into(), &[("app".into(), "api".into())]);
        assert_eq!(hits_t1.len(), 1);
        let hits_other = idx.select(&"missing".into(), &[("app".into(), "api".into())]);
        assert!(hits_other.is_empty());
    }

    #[test]
    fn record_chunk_appends_to_existing_series() {
        let mut idx = TsdbIndex::new();
        let a = lbl(&[("app", "api")]);
        idx.record_chunk(&"t".into(), &a, chunk(a.fingerprint(), "t", 0, 100));
        idx.record_chunk(&"t".into(), &a, chunk(a.fingerprint(), "t", 200, 300));
        assert_eq!(idx.series_count(&"t".into()), 1);
        let refs = idx.chunk_refs_in_range(&"t".into(), 0, 500);
        assert_eq!(refs.len(), 2);
    }
}
