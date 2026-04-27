//! Maintenance full — DBStats, per-bucket storage stats, snapshot
//! progress reporting, and a maintenance-driven compaction trigger.
//!
//! Mirrors etcd v3.6.10
//!   `server/storage/backend/backend.go` (Stats struct),
//!   `server/storage/mvcc/store_compaction.go` (Compact),
//!   `server/etcdserver/api/v3rpc/maintenance.go#Snapshot` (progress).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

// ── DBStats ───────────────────────────────────────────────────────────────

/// `DBStats` mirrors etcd's `backend.Stats`.  Tracks total transactions
/// and pending writers since process start.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DBStats {
    pub tx_total: u64,
    pub tx_writes: u64,
    pub tx_reads: u64,
    pub pending_writes: u64,
    pub spilled_pages: u64,
    pub free_pages: u64,
    pub allocated_pages: u64,
}

impl DBStats {
    pub fn fragmentation_ratio(&self) -> f64 {
        if self.allocated_pages == 0 { return 0.0; }
        self.free_pages as f64 / self.allocated_pages as f64
    }

    pub fn is_fragmented(&self, threshold: f64) -> bool {
        self.fragmentation_ratio() >= threshold
    }
}

pub struct DBStatsCollector {
    inner: RwLock<DBStats>,
}

impl DBStatsCollector {
    pub fn new() -> Self { Self { inner: RwLock::new(DBStats::default()) } }

    pub fn record_read(&self) {
        let mut s = self.inner.write().unwrap();
        s.tx_total += 1;
        s.tx_reads += 1;
    }

    pub fn record_write(&self) {
        let mut s = self.inner.write().unwrap();
        s.tx_total += 1;
        s.tx_writes += 1;
    }

    pub fn begin_pending(&self) { self.inner.write().unwrap().pending_writes += 1; }
    pub fn finish_pending(&self) {
        let mut s = self.inner.write().unwrap();
        if s.pending_writes > 0 { s.pending_writes -= 1; }
    }

    pub fn set_pages(&self, allocated: u64, free: u64, spilled: u64) {
        let mut s = self.inner.write().unwrap();
        s.allocated_pages = allocated;
        s.free_pages = free;
        s.spilled_pages = spilled;
    }

    pub fn snapshot(&self) -> DBStats { self.inner.read().unwrap().clone() }
}

impl Default for DBStatsCollector {
    fn default() -> Self { Self::new() }
}

// ── Bucket stats ──────────────────────────────────────────────────────────

/// Per-bucket storage stats — mirrors `backend.Bucket.Stats()`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BucketStats {
    pub key_count: u64,
    pub bytes_in_use: u64,
    pub branch_pages: u64,
    pub leaf_pages: u64,
}

impl BucketStats {
    pub fn avg_key_size(&self) -> u64 {
        if self.key_count == 0 { 0 } else { self.bytes_in_use / self.key_count }
    }
}

pub struct BucketRegistry {
    inner: RwLock<BTreeMap<String, BucketStats>>,
}

impl BucketRegistry {
    pub fn new() -> Self { Self { inner: RwLock::new(BTreeMap::new()) } }

    pub fn upsert(&self, name: impl Into<String>, stats: BucketStats) {
        self.inner.write().unwrap().insert(name.into(), stats);
    }

    pub fn get(&self, name: &str) -> Option<BucketStats> {
        self.inner.read().unwrap().get(name).cloned()
    }

    pub fn list(&self) -> Vec<(String, BucketStats)> {
        self.inner.read().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn total_bytes_in_use(&self) -> u64 {
        self.inner.read().unwrap().values().map(|s| s.bytes_in_use).sum()
    }

    pub fn total_key_count(&self) -> u64 {
        self.inner.read().unwrap().values().map(|s| s.key_count).sum()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    pub fn delete(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
}

impl Default for BucketRegistry {
    fn default() -> Self { Self::new() }
}

// ── Snapshot progress ─────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum MaintError {
    SnapshotAlreadyInProgress,
    SnapshotNotInProgress,
    BadProgress(String),
    UnknownBucket(String),
}

impl std::fmt::Display for MaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SnapshotAlreadyInProgress => write!(f, "snapshot already in progress"),
            Self::SnapshotNotInProgress => write!(f, "snapshot not in progress"),
            Self::BadProgress(m) => write!(f, "bad progress: {m}"),
            Self::UnknownBucket(b) => write!(f, "unknown bucket: {b}"),
        }
    }
}

impl std::error::Error for MaintError {}

#[derive(Debug, Clone)]
pub struct SnapshotProgress {
    pub member_id: u64,
    pub total_bytes: u64,
    pub bytes_sent: u64,
    pub started_at: Instant,
}

impl SnapshotProgress {
    pub fn percent(&self) -> f64 {
        if self.total_bytes == 0 { 0.0 } else {
            (self.bytes_sent as f64 / self.total_bytes as f64) * 100.0
        }
    }

    pub fn is_complete(&self) -> bool { self.bytes_sent >= self.total_bytes }
    pub fn elapsed(&self) -> Duration { self.started_at.elapsed() }
}

/// Tracks a single in-flight snapshot per member.
pub struct SnapshotTracker {
    inner: RwLock<BTreeMap<u64, SnapshotProgress>>,
}

impl SnapshotTracker {
    pub fn new() -> Self { Self { inner: RwLock::new(BTreeMap::new()) } }

    pub fn begin(&self, member_id: u64, total_bytes: u64) -> Result<(), MaintError> {
        let mut g = self.inner.write().unwrap();
        if g.contains_key(&member_id) { return Err(MaintError::SnapshotAlreadyInProgress); }
        g.insert(member_id, SnapshotProgress {
            member_id, total_bytes, bytes_sent: 0,
            started_at: Instant::now(),
        });
        Ok(())
    }

    pub fn advance(&self, member_id: u64, bytes_sent: u64) -> Result<(), MaintError> {
        let mut g = self.inner.write().unwrap();
        let p = g.get_mut(&member_id).ok_or(MaintError::SnapshotNotInProgress)?;
        if bytes_sent > p.total_bytes {
            return Err(MaintError::BadProgress(format!(
                "bytes_sent {bytes_sent} > total {}", p.total_bytes
            )));
        }
        if bytes_sent < p.bytes_sent {
            return Err(MaintError::BadProgress("bytes_sent regressed".into()));
        }
        p.bytes_sent = bytes_sent;
        Ok(())
    }

    pub fn finish(&self, member_id: u64) -> Result<SnapshotProgress, MaintError> {
        let mut g = self.inner.write().unwrap();
        g.remove(&member_id).ok_or(MaintError::SnapshotNotInProgress)
    }

    pub fn cancel(&self, member_id: u64) -> bool {
        self.inner.write().unwrap().remove(&member_id).is_some()
    }

    pub fn progress(&self, member_id: u64) -> Option<SnapshotProgress> {
        self.inner.read().unwrap().get(&member_id).cloned()
    }

    pub fn in_flight(&self) -> Vec<SnapshotProgress> {
        self.inner.read().unwrap().values().cloned().collect()
    }
}

impl Default for SnapshotTracker {
    fn default() -> Self { Self::new() }
}

// ── Compaction trigger via Maintenance ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionRequester {
    AdminApi,
    AutoSchedule,
    LeaderTransfer,
}

pub struct CompactionDispatcher {
    last_compacted: AtomicU64,
    compactions_total: AtomicU64,
    last_request: RwLock<Option<(CompactionRequester, Instant)>>,
}

impl CompactionDispatcher {
    pub fn new() -> Self {
        Self {
            last_compacted: AtomicU64::new(0),
            compactions_total: AtomicU64::new(0),
            last_request: RwLock::new(None),
        }
    }

    /// Request a compaction up to `revision`.  Returns the previously-
    /// compacted revision (0 if first call).  Refuses to go backwards.
    pub fn compact(&self, revision: u64, requester: CompactionRequester) -> Result<u64, MaintError> {
        let prev = self.last_compacted.load(Ordering::SeqCst);
        if revision < prev {
            return Err(MaintError::BadProgress(format!(
                "compaction revision {revision} < last compacted {prev}"
            )));
        }
        self.last_compacted.store(revision, Ordering::SeqCst);
        self.compactions_total.fetch_add(1, Ordering::SeqCst);
        *self.last_request.write().unwrap() = Some((requester, Instant::now()));
        Ok(prev)
    }

    pub fn last_compacted(&self) -> u64 { self.last_compacted.load(Ordering::SeqCst) }
    pub fn total(&self) -> u64 { self.compactions_total.load(Ordering::SeqCst) }
    pub fn last_request(&self) -> Option<(CompactionRequester, Instant)> {
        self.last_request.read().unwrap().clone()
    }
}

impl Default for CompactionDispatcher {
    fn default() -> Self { Self::new() }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M8 maintenance_full
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DBStats ───────────────────────────────────────────────────────

    #[test]
    fn test_dbstats_default_zero() {
        let s = DBStats::default();
        assert_eq!(s.tx_total, 0);
        assert_eq!(s.fragmentation_ratio(), 0.0);
    }

    #[test]
    fn test_dbstats_collector_records_read_write() {
        let c = DBStatsCollector::new();
        c.record_read();
        c.record_read();
        c.record_write();
        let s = c.snapshot();
        assert_eq!(s.tx_total, 3);
        assert_eq!(s.tx_reads, 2);
        assert_eq!(s.tx_writes, 1);
    }

    #[test]
    fn test_dbstats_pending_writers_counter() {
        let c = DBStatsCollector::new();
        c.begin_pending();
        c.begin_pending();
        c.finish_pending();
        assert_eq!(c.snapshot().pending_writes, 1);
    }

    #[test]
    fn test_dbstats_finish_pending_below_zero() {
        let c = DBStatsCollector::new();
        c.finish_pending();
        assert_eq!(c.snapshot().pending_writes, 0);
    }

    #[test]
    fn test_dbstats_pages_recorded() {
        let c = DBStatsCollector::new();
        c.set_pages(1000, 200, 50);
        let s = c.snapshot();
        assert_eq!(s.allocated_pages, 1000);
        assert_eq!(s.free_pages, 200);
        assert_eq!(s.spilled_pages, 50);
    }

    #[test]
    fn test_dbstats_fragmentation_ratio() {
        // cite: backend.Stats (free/allocated)
        let s = DBStats { allocated_pages: 1000, free_pages: 200, ..Default::default() };
        assert!((s.fragmentation_ratio() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn test_dbstats_is_fragmented_threshold() {
        let s = DBStats { allocated_pages: 100, free_pages: 30, ..Default::default() };
        assert!(s.is_fragmented(0.2));
        assert!(!s.is_fragmented(0.5));
    }

    // ── BucketRegistry ────────────────────────────────────────────────

    #[test]
    fn test_bucket_upsert_and_get() {
        // cite: backend.Bucket.Stats()
        let r = BucketRegistry::new();
        r.upsert("key", BucketStats { key_count: 100, bytes_in_use: 1000, branch_pages: 5, leaf_pages: 20 });
        let s = r.get("key").unwrap();
        assert_eq!(s.key_count, 100);
    }

    #[test]
    fn test_bucket_avg_key_size() {
        let s = BucketStats { key_count: 10, bytes_in_use: 1000, ..Default::default() };
        assert_eq!(s.avg_key_size(), 100);
    }

    #[test]
    fn test_bucket_avg_key_size_empty() {
        let s = BucketStats::default();
        assert_eq!(s.avg_key_size(), 0);
    }

    #[test]
    fn test_bucket_total_bytes() {
        let r = BucketRegistry::new();
        r.upsert("a", BucketStats { bytes_in_use: 100, ..Default::default() });
        r.upsert("b", BucketStats { bytes_in_use: 200, ..Default::default() });
        assert_eq!(r.total_bytes_in_use(), 300);
    }

    #[test]
    fn test_bucket_total_keys() {
        let r = BucketRegistry::new();
        r.upsert("a", BucketStats { key_count: 1, ..Default::default() });
        r.upsert("b", BucketStats { key_count: 2, ..Default::default() });
        assert_eq!(r.total_key_count(), 3);
    }

    #[test]
    fn test_bucket_names_sorted() {
        let r = BucketRegistry::new();
        for n in ["c", "a", "b"] { r.upsert(n, BucketStats::default()); }
        assert_eq!(r.names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_bucket_delete() {
        let r = BucketRegistry::new();
        r.upsert("a", BucketStats::default());
        assert!(r.delete("a"));
        assert!(r.get("a").is_none());
    }

    // ── SnapshotTracker ───────────────────────────────────────────────

    #[test]
    fn test_snapshot_begin_advance_finish() {
        let t = SnapshotTracker::new();
        t.begin(1, 1000).unwrap();
        t.advance(1, 500).unwrap();
        let p = t.progress(1).unwrap();
        assert_eq!(p.bytes_sent, 500);
        assert!((p.percent() - 50.0).abs() < 1e-9);
        let done = t.finish(1).unwrap();
        assert_eq!(done.bytes_sent, 500);
    }

    #[test]
    fn test_snapshot_begin_twice_errors() {
        let t = SnapshotTracker::new();
        t.begin(1, 100).unwrap();
        assert_eq!(t.begin(1, 100).unwrap_err(), MaintError::SnapshotAlreadyInProgress);
    }

    #[test]
    fn test_snapshot_advance_unknown_errors() {
        let t = SnapshotTracker::new();
        assert_eq!(t.advance(1, 50).unwrap_err(), MaintError::SnapshotNotInProgress);
    }

    #[test]
    fn test_snapshot_advance_overshoot_errors() {
        let t = SnapshotTracker::new();
        t.begin(1, 100).unwrap();
        match t.advance(1, 200).unwrap_err() {
            MaintError::BadProgress(m) => assert!(m.contains("bytes_sent")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_snapshot_advance_regress_errors() {
        // cite: progress monotonic
        let t = SnapshotTracker::new();
        t.begin(1, 100).unwrap();
        t.advance(1, 50).unwrap();
        match t.advance(1, 25).unwrap_err() {
            MaintError::BadProgress(m) => assert!(m.contains("regressed")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_snapshot_is_complete() {
        let t = SnapshotTracker::new();
        t.begin(1, 100).unwrap();
        t.advance(1, 100).unwrap();
        assert!(t.progress(1).unwrap().is_complete());
    }

    #[test]
    fn test_snapshot_cancel_returns_true_when_in_flight() {
        let t = SnapshotTracker::new();
        t.begin(1, 100).unwrap();
        assert!(t.cancel(1));
        assert!(t.progress(1).is_none());
    }

    #[test]
    fn test_snapshot_cancel_returns_false_when_idle() {
        let t = SnapshotTracker::new();
        assert!(!t.cancel(1));
    }

    #[test]
    fn test_snapshot_in_flight_lists_all() {
        let t = SnapshotTracker::new();
        t.begin(1, 100).unwrap();
        t.begin(2, 200).unwrap();
        assert_eq!(t.in_flight().len(), 2);
    }

    // ── CompactionDispatcher ──────────────────────────────────────────

    #[test]
    fn test_compaction_first_call_returns_zero_prev() {
        let d = CompactionDispatcher::new();
        assert_eq!(d.compact(100, CompactionRequester::AdminApi).unwrap(), 0);
        assert_eq!(d.last_compacted(), 100);
    }

    #[test]
    fn test_compaction_advances_revision() {
        let d = CompactionDispatcher::new();
        d.compact(100, CompactionRequester::AdminApi).unwrap();
        assert_eq!(d.compact(200, CompactionRequester::AutoSchedule).unwrap(), 100);
        assert_eq!(d.last_compacted(), 200);
    }

    #[test]
    fn test_compaction_backwards_rejected() {
        // cite: store_compaction.go (compact rev must be monotone)
        let d = CompactionDispatcher::new();
        d.compact(100, CompactionRequester::AdminApi).unwrap();
        assert!(d.compact(50, CompactionRequester::AdminApi).is_err());
    }

    #[test]
    fn test_compaction_records_requester() {
        let d = CompactionDispatcher::new();
        d.compact(100, CompactionRequester::LeaderTransfer).unwrap();
        let (req, _) = d.last_request().unwrap();
        assert_eq!(req, CompactionRequester::LeaderTransfer);
    }

    #[test]
    fn test_compaction_total_counter() {
        let d = CompactionDispatcher::new();
        d.compact(100, CompactionRequester::AdminApi).unwrap();
        d.compact(200, CompactionRequester::AdminApi).unwrap();
        d.compact(300, CompactionRequester::AdminApi).unwrap();
        assert_eq!(d.total(), 3);
    }
}
