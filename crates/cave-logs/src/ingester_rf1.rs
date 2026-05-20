// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ingester RF-1 — `pkg/ingester-rf1`.
//!
//! Loki 3.x ships an experimental ingester variant under `pkg/ingester-rf1/`
//! that drops the legacy WAL replay path in favour of a write-ahead "object"
//! log: every push is appended to a per-tenant in-memory segment, and on the
//! flush interval the segment is rolled to a single object-store key. The
//! "rf1" suffix marks the replication factor — one copy per object,
//! relying on the storage backend's own durability rather than upstream's
//! N-way ingester ring.
//!
//! This port mirrors the segment lifecycle (`open → append → roll → close`)
//! plus the flush trigger semantics (size-bound + age-bound).
//!
//! Mapped surfaces:
//! * `pkg/ingester-rf1/ingester.go` — segment lifecycle + flush trigger
//! * `pkg/ingester-rf1/objstore/object.go` — object-key naming
//! * `pkg/ingester-rf1/metastore/streams.go` — per-tenant stream catalog

use crate::models::{LogEntry, TenantId, TimestampNs};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Segment {
    pub tenant: TenantId,
    pub fp: u64,
    pub entries: Vec<LogEntry>,
    pub bytes: usize,
    pub opened_at: Instant,
    pub min_ts: TimestampNs,
    pub max_ts: TimestampNs,
}

impl Segment {
    pub fn new(tenant: TenantId, fp: u64) -> Self {
        Self {
            tenant,
            fp,
            entries: Vec::new(),
            bytes: 0,
            opened_at: Instant::now(),
            min_ts: i64::MAX,
            max_ts: i64::MIN,
        }
    }

    pub fn append(&mut self, e: LogEntry) {
        self.bytes += e.size_bytes();
        self.min_ts = self.min_ts.min(e.ts);
        self.max_ts = self.max_ts.max(e.ts);
        self.entries.push(e);
    }

    pub fn age(&self) -> Duration {
        self.opened_at.elapsed()
    }
}

/// Trigger reason — useful for parity assertions and observability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushReason {
    SizeThreshold,
    AgeThreshold,
    Manual,
}

/// Ingester RF-1 — per-tenant + per-stream segment manager with flush triggers.
pub struct IngesterRf1 {
    pub flush_bytes: usize,
    pub flush_age: Duration,
    segments: HashMap<(TenantId, u64), Segment>,
    flushed: Vec<(Segment, FlushReason)>,
}

impl IngesterRf1 {
    pub fn new(flush_bytes: usize, flush_age: Duration) -> Self {
        Self {
            flush_bytes,
            flush_age,
            segments: HashMap::new(),
            flushed: Vec::new(),
        }
    }

    /// Append an entry to the (tenant, stream-fp) segment; flushes inline if size bound hit.
    pub fn ingest(&mut self, tenant: &TenantId, fp: u64, entry: LogEntry) -> Option<FlushReason> {
        let key = (tenant.clone(), fp);
        let seg = self
            .segments
            .entry(key.clone())
            .or_insert_with(|| Segment::new(tenant.clone(), fp));
        seg.append(entry);
        if seg.bytes >= self.flush_bytes {
            let s = self.segments.remove(&key).unwrap();
            self.flushed.push((s, FlushReason::SizeThreshold));
            return Some(FlushReason::SizeThreshold);
        }
        None
    }

    /// Run the age-based flush sweep — flushes any segment older than `flush_age`.
    pub fn sweep_age(&mut self) -> usize {
        let cutoff = self.flush_age;
        let mut to_flush: Vec<(TenantId, u64)> = Vec::new();
        for (k, s) in self.segments.iter() {
            if s.age() >= cutoff {
                to_flush.push(k.clone());
            }
        }
        let n = to_flush.len();
        for k in to_flush {
            if let Some(s) = self.segments.remove(&k) {
                self.flushed.push((s, FlushReason::AgeThreshold));
            }
        }
        n
    }

    /// Force-flush every open segment (shutdown / drain path).
    pub fn flush_all(&mut self) -> usize {
        let keys: Vec<_> = self.segments.keys().cloned().collect();
        let n = keys.len();
        for k in keys {
            if let Some(s) = self.segments.remove(&k) {
                self.flushed.push((s, FlushReason::Manual));
            }
        }
        n
    }

    pub fn open_segments(&self) -> usize {
        self.segments.len()
    }

    pub fn flushed_count(&self) -> usize {
        self.flushed.len()
    }

    pub fn flushed(&self) -> &[(Segment, FlushReason)] {
        &self.flushed
    }

    /// Upstream `objstore.SegmentKey` — segment-key naming for RF-1 objects.
    pub fn segment_key(tenant: &TenantId, fp: u64, opened_ns: TimestampNs) -> String {
        format!("rf1/{}/{:016x}/{}", tenant, fp, opened_ns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ts: TimestampNs, line: &str) -> LogEntry {
        LogEntry::new(ts, line)
    }

    #[test]
    fn append_below_threshold_keeps_segment_open() {
        let mut ing = IngesterRf1::new(1024, Duration::from_secs(60));
        assert!(ing.ingest(&"t".into(), 1, entry(0, "hello")).is_none());
        assert_eq!(ing.open_segments(), 1);
        assert_eq!(ing.flushed_count(), 0);
    }

    #[test]
    fn size_threshold_triggers_flush() {
        let mut ing = IngesterRf1::new(10, Duration::from_secs(60));
        let r = ing.ingest(&"t".into(), 1, entry(0, "0123456789ABC"));
        assert_eq!(r, Some(FlushReason::SizeThreshold));
        assert_eq!(ing.open_segments(), 0);
        assert_eq!(ing.flushed_count(), 1);
    }

    #[test]
    fn flush_all_drains_segments() {
        let mut ing = IngesterRf1::new(1024, Duration::from_secs(60));
        ing.ingest(&"t".into(), 1, entry(0, "a"));
        ing.ingest(&"t".into(), 2, entry(0, "b"));
        ing.ingest(&"u".into(), 1, entry(0, "c"));
        assert_eq!(ing.open_segments(), 3);
        let n = ing.flush_all();
        assert_eq!(n, 3);
        assert_eq!(ing.open_segments(), 0);
        assert_eq!(ing.flushed_count(), 3);
    }

    #[test]
    fn segment_key_includes_tenant_fp_and_open_ns() {
        let k = IngesterRf1::segment_key(&"tenant".into(), 0xABCD, 1234567890);
        assert!(k.starts_with("rf1/tenant/"));
        assert!(k.contains("000000000000abcd"));
        assert!(k.ends_with("/1234567890"));
    }

    #[test]
    fn segment_tracks_min_max_ts() {
        let mut s = Segment::new("t".into(), 1);
        s.append(entry(100, "a"));
        s.append(entry(50, "b"));
        s.append(entry(300, "c"));
        assert_eq!(s.min_ts, 50);
        assert_eq!(s.max_ts, 300);
        assert_eq!(s.entries.len(), 3);
    }

    #[test]
    fn age_sweep_with_zero_threshold_flushes_all_open() {
        let mut ing = IngesterRf1::new(1024, Duration::from_secs(0));
        ing.ingest(&"t".into(), 1, entry(0, "x"));
        ing.ingest(&"t".into(), 2, entry(0, "y"));
        let n = ing.sweep_age();
        assert_eq!(n, 2);
        assert_eq!(ing.open_segments(), 0);
    }
}
