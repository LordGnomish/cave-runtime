// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Segment-based append-only log + Bookkeeper-style replication ensemble.
//!
//! ## Storage layer
//!
//! `SegmentLog` is a per-topic-partition append-only log split into
//! fixed-size segments.  Each segment carries its base offset; a new
//! segment rolls when the active one's byte size exceeds the configured
//! `max_segment_bytes`.  Reads return entries via `(offset, max_bytes)`.
//!
//! Mirrors:
//!   - Apache Kafka 4.2.0 `core/src/main/scala/kafka/log/UnifiedLog.scala`
//!   - Apache Pulsar/BookKeeper `bookkeeper-server/.../bookie/Bookie.java`
//!
//! ## Replication
//!
//! `Ensemble` models a Bookkeeper-style write quorum: an entry counts as
//! committed once `ack_quorum` of `ensemble_size` bookies have written it.
//! Cave Streams implements bookies in-process for the unit tests; the same
//! `Ensemble` API is reused by the network layer in production.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{StreamsError, StreamsResult};

/// One entry in the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub offset: u64,
    pub timestamp_ms: i64,
    pub payload: Vec<u8>,
}

/// One segment file (in-memory representation).
#[derive(Debug)]
pub struct Segment {
    pub base_offset: u64,
    pub byte_size: u64,
    pub entries: Vec<LogEntry>,
    pub closed: bool,
}

impl Segment {
    fn new(base_offset: u64) -> Self {
        Self {
            base_offset,
            byte_size: 0,
            entries: Vec::new(),
            closed: false,
        }
    }
}

/// Append-only log split into rolling segments.
pub struct SegmentLog {
    /// Maximum bytes in an active segment before rolling.
    max_segment_bytes: u64,
    /// Maximum total bytes retained (oldest segments evicted).  0 = no cap.
    retention_bytes: AtomicU64,
    /// Active and closed segments, oldest first.
    segments: Mutex<Vec<Segment>>,
    next_offset: AtomicU64,
    /// Earliest offset still readable; advances on retention eviction.
    log_start: AtomicU64,
}

impl SegmentLog {
    pub fn new(max_segment_bytes: u64) -> Self {
        Self {
            max_segment_bytes,
            retention_bytes: AtomicU64::new(0),
            segments: Mutex::new(vec![Segment::new(0)]),
            next_offset: AtomicU64::new(0),
            log_start: AtomicU64::new(0),
        }
    }

    pub fn with_retention(max_segment_bytes: u64, retention_bytes: u64) -> Self {
        let s = Self::new(max_segment_bytes);
        s.set_retention_bytes(retention_bytes);
        s
    }

    pub fn set_retention_bytes(&self, b: u64) {
        self.retention_bytes.store(b, Ordering::SeqCst);
    }

    pub fn next_offset(&self) -> u64 {
        self.next_offset.load(Ordering::SeqCst)
    }

    pub fn log_start_offset(&self) -> u64 {
        self.log_start.load(Ordering::SeqCst)
    }

    /// Append a single record, returning its assigned offset.
    pub fn append(&self, payload: Vec<u8>, timestamp_ms: i64) -> StreamsResult<u64> {
        let mut segs = self.segments.lock().unwrap();
        let active_idx = segs.len() - 1;
        // Roll if active segment would exceed max_segment_bytes after this
        // append.
        let needed = payload.len() as u64;
        let active_byte_size = segs[active_idx].byte_size;
        if active_byte_size > 0 && active_byte_size + needed > self.max_segment_bytes {
            segs[active_idx].closed = true;
            let next_base = self.next_offset();
            segs.push(Segment::new(next_base));
        }
        let active_idx = segs.len() - 1;
        let offset = self.next_offset.fetch_add(1, Ordering::SeqCst);
        let entry = LogEntry {
            offset,
            timestamp_ms,
            payload,
        };
        segs[active_idx].byte_size += entry.payload.len() as u64;
        segs[active_idx].entries.push(entry);

        // Apply retention if configured.
        let retention = self.retention_bytes.load(Ordering::SeqCst);
        if retention > 0 {
            let total: u64 = segs.iter().map(|s| s.byte_size).sum();
            if total > retention {
                // Drop closed segments from the front while the remaining
                // total still exceeds the cap.  Never evict the active.
                let mut total = total;
                while segs.len() > 1 && total > retention {
                    let head = &segs[0];
                    if !head.closed {
                        break;
                    }
                    total = total.saturating_sub(head.byte_size);
                    let head_last = head
                        .entries
                        .last()
                        .map(|e| e.offset + 1)
                        .unwrap_or(head.base_offset);
                    self.log_start.store(head_last, Ordering::SeqCst);
                    segs.remove(0);
                }
            }
        }
        Ok(offset)
    }

    /// Read up to `max_bytes` worth of entries starting at `from_offset`.
    pub fn read(&self, from_offset: u64, max_bytes: usize) -> StreamsResult<Vec<LogEntry>> {
        let log_start = self.log_start_offset();
        if from_offset < log_start {
            return Err(StreamsError::OffsetOutOfRange {
                topic: String::new(),
                partition: 0,
                offset: from_offset as i64,
            });
        }
        let segs = self.segments.lock().unwrap();
        let mut out = Vec::new();
        let mut bytes = 0usize;
        for seg in segs.iter() {
            for e in &seg.entries {
                if e.offset < from_offset {
                    continue;
                }
                let n = e.payload.len();
                if !out.is_empty() && bytes + n > max_bytes {
                    return Ok(out);
                }
                out.push(e.clone());
                bytes += n;
            }
        }
        Ok(out)
    }

    pub fn segment_count(&self) -> usize {
        self.segments.lock().unwrap().len()
    }

    /// Total bytes currently retained.
    pub fn total_bytes(&self) -> u64 {
        self.segments
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.byte_size)
            .sum()
    }

    /// Truncate the log forward to `low_watermark`, dropping any entries
    /// strictly below it.  Idempotent; advances `log_start_offset`.
    pub fn truncate_before(&self, low_watermark: u64) {
        if low_watermark <= self.log_start_offset() {
            return;
        }
        let mut segs = self.segments.lock().unwrap();
        // Walk segments from the front, dropping fully-below ones; the
        // first surviving segment may need its tail trimmed.
        loop {
            let drop_first = segs
                .first()
                .map(|s| {
                    s.entries.last().map(|e| e.offset).unwrap_or(s.base_offset) < low_watermark
                })
                .unwrap_or(false);
            if drop_first && segs.len() > 1 {
                segs.remove(0);
                continue;
            }
            break;
        }
        if let Some(first) = segs.first_mut() {
            first.entries.retain(|e| e.offset >= low_watermark);
            first.byte_size = first.entries.iter().map(|e| e.payload.len() as u64).sum();
        }
        self.log_start.store(low_watermark, Ordering::SeqCst);
    }
}

// ── Bookkeeper-style ensemble ─────────────────────────────────────────────

/// Per-bookie in-memory store keyed by entry_id.
#[derive(Debug, Default)]
pub struct InMemoryBookie {
    pub bookie_id: String,
    /// Set of entry_ids written to this bookie.
    pub entries: Mutex<std::collections::BTreeMap<u64, Vec<u8>>>,
    /// `false` while this bookie is fenced (write/read failures injected).
    pub up: Mutex<bool>,
}

impl InMemoryBookie {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            bookie_id: id.into(),
            entries: Mutex::new(std::collections::BTreeMap::new()),
            up: Mutex::new(true),
        }
    }
    pub fn fence(&self) {
        *self.up.lock().unwrap() = false;
    }
    pub fn unfence(&self) {
        *self.up.lock().unwrap() = true;
    }
    pub fn is_up(&self) -> bool {
        *self.up.lock().unwrap()
    }
}

/// Result of a single ensemble write — bookies that ack vs those that didn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsembleWriteResult {
    pub entry_id: u64,
    pub acked: HashSet<String>,
    pub failed: HashSet<String>,
}

/// A write quorum following Bookkeeper's `(ensemble_size, write_quorum,
/// ack_quorum)` triple.  `write_quorum` bookies are written to in parallel
/// and the entry is committed once `ack_quorum` of them ack.
pub struct Ensemble {
    pub ensemble_size: usize,
    pub write_quorum: usize,
    pub ack_quorum: usize,
    pub bookies: Vec<std::sync::Arc<InMemoryBookie>>,
    next_entry_id: AtomicU64,
}

impl Ensemble {
    pub fn new(
        ensemble_size: usize,
        write_quorum: usize,
        ack_quorum: usize,
        bookies: Vec<std::sync::Arc<InMemoryBookie>>,
    ) -> StreamsResult<Self> {
        if bookies.len() != ensemble_size {
            return Err(StreamsError::Internal(format!(
                "bookies.len() ({}) != ensemble_size ({})",
                bookies.len(),
                ensemble_size
            )));
        }
        if !(ack_quorum <= write_quorum && write_quorum <= ensemble_size) {
            return Err(StreamsError::Internal(format!(
                "must have ack_quorum ≤ write_quorum ≤ ensemble_size, got ({}, {}, {})",
                ack_quorum, write_quorum, ensemble_size
            )));
        }
        Ok(Self {
            ensemble_size,
            write_quorum,
            ack_quorum,
            bookies,
            next_entry_id: AtomicU64::new(0),
        })
    }

    pub fn next_entry_id(&self) -> u64 {
        self.next_entry_id.load(Ordering::SeqCst)
    }

    /// Write `payload` to the first `write_quorum` *up* bookies.  Succeeds
    /// when ≥ `ack_quorum` ack; returns `WriteResult` describing both groups.
    pub fn write_entry(&self, payload: Vec<u8>) -> StreamsResult<EnsembleWriteResult> {
        let entry_id = self.next_entry_id.fetch_add(1, Ordering::SeqCst);
        let mut acked: HashSet<String> = HashSet::new();
        let mut failed: HashSet<String> = HashSet::new();
        let mut written = 0usize;

        for b in &self.bookies {
            if written == self.write_quorum {
                break;
            }
            written += 1;
            if b.is_up() {
                b.entries.lock().unwrap().insert(entry_id, payload.clone());
                acked.insert(b.bookie_id.clone());
            } else {
                failed.insert(b.bookie_id.clone());
            }
        }

        if acked.len() < self.ack_quorum {
            return Err(StreamsError::NotEnoughReplicas {
                required: self.ack_quorum as i16,
                available: acked.len() as i16,
            });
        }

        Ok(EnsembleWriteResult {
            entry_id,
            acked,
            failed,
        })
    }

    /// Read `entry_id` by polling bookies; first up-bookie wins.
    pub fn read_entry(&self, entry_id: u64) -> StreamsResult<Vec<u8>> {
        for b in &self.bookies {
            if !b.is_up() {
                continue;
            }
            if let Some(p) = b.entries.lock().unwrap().get(&entry_id).cloned() {
                return Ok(p);
            }
        }
        Err(StreamsError::Internal(format!(
            "entry {entry_id} not found in any up bookie"
        )))
    }

    /// `true` when at least `ack_quorum` bookies are currently up.
    pub fn quorum_available(&self) -> bool {
        self.bookies.iter().filter(|b| b.is_up()).count() >= self.ack_quorum
    }
}

// ─────────────────────────────────────────────────────────────────────────
// segment_log + ensemble tests
// feat/cave-streams-kafka-pulsar-001
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn payload(tenant_id: &str, n: usize) -> Vec<u8> {
        // Tenant-tagged payload so each test's writes are namespaced.
        format!("{}:{:04}", tenant_id, n).into_bytes()
    }

    // ── SegmentLog ────────────────────────────────────────────────────

    #[test]
    fn test_segment_log_append_assigns_offset() {
        // cite: kafka 4.2.0 core/.../UnifiedLog.scala#append (assigns nextOffset)
        let tenant_id = "seg-001";
        let log = SegmentLog::new(1024);
        let o1 = log.append(payload(tenant_id, 0), 1).unwrap();
        let o2 = log.append(payload(tenant_id, 1), 2).unwrap();
        let o3 = log.append(payload(tenant_id, 2), 3).unwrap();
        assert_eq!(o1, 0);
        assert_eq!(o2, 1);
        assert_eq!(o3, 2);
        assert_eq!(log.next_offset(), 3);
    }

    #[test]
    fn test_segment_log_rolls_segments_when_oversized() {
        // cite: kafka 4.2.0 LogSegment.scala#shouldRoll
        let tenant_id = "seg-002";
        let log = SegmentLog::new(20);
        for i in 0..6 {
            log.append(payload(tenant_id, i), 1).unwrap(); // 8 bytes each
        }
        assert!(log.segment_count() >= 2, "log should have rolled segments");
    }

    #[test]
    fn test_segment_log_read_returns_subset_after_max_bytes() {
        // cite: kafka 4.2.0 UnifiedLog.scala#read (max_bytes truncates)
        let tenant_id = "seg-003";
        let log = SegmentLog::new(1024);
        for i in 0..10 {
            log.append(payload(tenant_id, i), 1).unwrap();
        }
        let entries = log.read(0, 16).unwrap();
        // 8-byte entries; first one fits unconditionally + at least one more
        assert!(entries.len() >= 1);
        assert!(entries.len() <= 3);
    }

    #[test]
    fn test_segment_log_retention_evicts_closed_segments() {
        // cite: kafka 4.2.0 UnifiedLog.scala#deleteOldSegments (size retention)
        let tenant_id = "seg-004";
        let log = SegmentLog::with_retention(20, 30);
        for i in 0..10 {
            log.append(payload(tenant_id, i), 1).unwrap(); // 8 bytes each
        }
        // Total written = 80; retention = 30 → some closed segments evicted.
        assert!(log.total_bytes() <= 80);
        assert!(log.log_start_offset() > 0, "log_start should have moved");
    }

    #[test]
    fn test_segment_log_read_below_log_start_errors() {
        // cite: kafka 4.2.0 UnifiedLog.scala#OffsetOutOfRangeException
        let tenant_id = "seg-005";
        let log = SegmentLog::with_retention(20, 30);
        for i in 0..10 {
            log.append(payload(tenant_id, i), 1).unwrap();
        }
        let lso = log.log_start_offset();
        assert!(lso > 0);
        let err = log.read(0, 1024);
        assert!(matches!(err, Err(StreamsError::OffsetOutOfRange { .. })));
    }

    #[test]
    fn test_segment_log_truncate_before_advances_log_start() {
        // cite: kafka 4.2.0 UnifiedLog.scala#truncateFullyAndStartAt
        let tenant_id = "seg-006";
        let log = SegmentLog::new(1024);
        for i in 0..5 {
            log.append(payload(tenant_id, i), 1).unwrap();
        }
        log.truncate_before(3);
        assert_eq!(log.log_start_offset(), 3);
        let entries = log.read(3, 1024).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].offset, 3);
    }

    #[test]
    fn test_segment_log_empty_read_returns_empty_vec() {
        // cite: kafka 4.2.0 UnifiedLog.scala#read (empty log → empty)
        let _tenant_id = "seg-007";
        let log = SegmentLog::new(1024);
        let entries = log.read(0, 1024).unwrap();
        assert!(entries.is_empty());
    }

    // ── Ensemble ──────────────────────────────────────────────────────

    fn ensemble(tenant_id: &str, e: usize, w: usize, q: usize) -> Ensemble {
        let bookies: Vec<Arc<InMemoryBookie>> = (0..e)
            .map(|i| Arc::new(InMemoryBookie::new(format!("{}-bk{}", tenant_id, i))))
            .collect();
        Ensemble::new(e, w, q, bookies).unwrap()
    }

    #[test]
    fn test_ensemble_write_satisfies_quorum() {
        // cite: bookkeeper 4.16 BookKeeper.java#asyncAddEntry (ack quorum)
        let tenant_id = "ens-001";
        let ens = ensemble(tenant_id, 3, 3, 2);
        let res = ens.write_entry(b"hello".to_vec()).unwrap();
        assert_eq!(res.entry_id, 0);
        assert!(res.acked.len() >= 2);
        assert!(res.failed.is_empty());
    }

    #[test]
    fn test_ensemble_write_under_quorum_fails() {
        // cite: bookkeeper 4.16 BKException.BKNotEnoughBookiesException
        let tenant_id = "ens-002";
        let ens = ensemble(tenant_id, 3, 3, 3);
        // Fence two bookies — only 1 will ack.
        ens.bookies[1].fence();
        ens.bookies[2].fence();
        let err = ens.write_entry(b"hello".to_vec());
        assert!(matches!(err, Err(StreamsError::NotEnoughReplicas { .. })));
    }

    #[test]
    fn test_ensemble_read_picks_first_up_bookie() {
        // cite: bookkeeper 4.16 BookKeeper.java#asyncReadEntries
        let tenant_id = "ens-003";
        let ens = ensemble(tenant_id, 3, 3, 2);
        let res = ens.write_entry(b"x".to_vec()).unwrap();
        // Fence the first bookie so the reader has to fall through.
        ens.bookies[0].fence();
        let got = ens.read_entry(res.entry_id).unwrap();
        assert_eq!(got, b"x");
    }

    #[test]
    fn test_ensemble_quorum_available_reflects_fenced_bookies() {
        // cite: bookkeeper 4.16 EnsemblePlacementPolicy (quorum available)
        let tenant_id = "ens-004";
        let ens = ensemble(tenant_id, 3, 3, 2);
        assert!(ens.quorum_available());
        ens.bookies[0].fence();
        assert!(ens.quorum_available()); // 2 still up
        ens.bookies[1].fence();
        assert!(!ens.quorum_available()); // only 1 up
    }

    #[test]
    fn test_ensemble_rejects_invalid_quorum_config() {
        // cite: bookkeeper 4.16 LedgerHandle (q ≤ w ≤ E invariant)
        let tenant_id = "ens-005";
        let bookies: Vec<Arc<InMemoryBookie>> = (0..3)
            .map(|i| Arc::new(InMemoryBookie::new(format!("{}-bk{}", tenant_id, i))))
            .collect();
        // ack=4, write=3, ensemble=3 → invalid
        let err = Ensemble::new(3, 3, 4, bookies);
        assert!(err.is_err());
    }
}
