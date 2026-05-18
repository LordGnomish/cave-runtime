// SPDX-License-Identifier: AGPL-3.0-or-later
//! Snapshot streaming — chunked transmission with resume support, plus
//! the *learner-bootstrap* state machine that drives a new member from
//! `Joining` → `CatchingUp` → `Healthy`.
//!
//! Mirrors etcd v3.6.10
//!   `server/etcdserver/api/rafthttp/snapshot_sender.go` (chunked send),
//!   `server/etcdserver/api/rafthttp/peer.go` (learner-bootstrap loop).

use crate::error::{EtcdError, EtcdResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;

// ── Streaming receive buffer ──────────────────────────────────────────────

/// Streaming-snapshot reassembler.
///
/// Designed for the receiver side: chunks arrive in any order, the
/// receiver buffers them keyed by sequence number until every chunk in
/// `[0, total)` has arrived, then `assemble()` returns the concatenated
/// payload bytes.
pub struct SnapshotReceiver {
    total: u64,
    chunk_size: usize,
    received: RwLock<BTreeMap<u64, Vec<u8>>>,
    bytes_received: AtomicUsize,
    completed: AtomicU64,
}

impl SnapshotReceiver {
    pub fn new(total_chunks: u64, chunk_size: usize) -> Self {
        Self {
            total: total_chunks,
            chunk_size,
            received: RwLock::new(BTreeMap::new()),
            bytes_received: AtomicUsize::new(0),
            completed: AtomicU64::new(0),
        }
    }

    pub fn total_chunks(&self) -> u64 { self.total }
    pub fn chunk_size(&self) -> usize { self.chunk_size }

    pub fn received_count(&self) -> u64 {
        self.received.read().unwrap().len() as u64
    }

    pub fn bytes_received(&self) -> usize {
        self.bytes_received.load(Ordering::SeqCst)
    }

    pub fn is_complete(&self) -> bool {
        self.received_count() == self.total
    }

    /// Add a chunk.  Idempotent: re-supplying the same sequence is a no-op.
    /// Returns `true` if the chunk is new.
    pub fn add(&self, sequence: u64, payload: Vec<u8>) -> EtcdResult<bool> {
        if sequence >= self.total {
            return Err(EtcdError::SnapshotDecode(format!(
                "sequence {sequence} >= total {}", self.total
            )));
        }
        let mut g = self.received.write().unwrap();
        if g.contains_key(&sequence) { return Ok(false); }
        self.bytes_received.fetch_add(payload.len(), Ordering::SeqCst);
        g.insert(sequence, payload);
        if g.len() as u64 == self.total {
            self.completed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(true)
    }

    /// List of sequences that have not yet arrived.  Used by the receiver
    /// to ask the sender for retransmission.
    pub fn missing(&self) -> Vec<u64> {
        let g = self.received.read().unwrap();
        (0..self.total).filter(|s| !g.contains_key(s)).collect()
    }

    /// Concatenate all chunks (in sequence order) into a single buffer.
    /// Errors if any chunk is missing.
    pub fn assemble(&self) -> EtcdResult<Vec<u8>> {
        let g = self.received.read().unwrap();
        if (g.len() as u64) != self.total {
            return Err(EtcdError::SnapshotDecode(format!(
                "missing {} chunks", self.total - g.len() as u64
            )));
        }
        let mut out = Vec::with_capacity(self.bytes_received());
        for s in 0..self.total {
            let chunk = g.get(&s).ok_or_else(|| EtcdError::SnapshotDecode(format!("missing seq {s}")))?;
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }
}

// ── Snapshot chunker (sender side) ───────────────────────────────────────

/// Chunk a payload into fixed-size pieces, ready for the wire.
pub fn chunk(payload: &[u8], chunk_size: usize) -> Vec<(u64, Vec<u8>)> {
    if chunk_size == 0 || payload.is_empty() { return Vec::new(); }
    let mut out = Vec::new();
    let mut seq = 0u64;
    let mut p = 0usize;
    while p < payload.len() {
        let end = (p + chunk_size).min(payload.len());
        out.push((seq, payload[p..end].to_vec()));
        seq += 1;
        p = end;
    }
    out
}

// ── Learner bootstrap state machine ──────────────────────────────────────

/// Stages a learner traverses on its way to becoming a healthy voter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnerStage {
    /// `MemberAdd` accepted; the new node has not contacted the leader yet.
    Joining,
    /// Leader has begun shipping snapshot chunks.
    SnapshotInFlight,
    /// Snapshot delivered; learner is replaying log entries.
    CatchingUp,
    /// Within the configured `learner-max-lag` revision window.
    Healthy,
    /// Promotion via `MemberPromote` accepted; learner is now a voter.
    Promoted,
}

#[derive(Debug)]
pub enum LearnerError {
    /// Tried to promote before `Healthy`.
    NotReady(LearnerStage),
    /// Stage transitioned illegally (e.g. Promoted → Joining).
    InvalidTransition { from: LearnerStage, to: LearnerStage },
    /// Lag exceeded the configured ceiling.
    LagTooHigh { lag: u64, ceiling: u64 },
}

impl std::fmt::Display for LearnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady(s) => write!(f, "not ready: stage={s:?}"),
            Self::InvalidTransition { from, to } => write!(f, "invalid transition {from:?} → {to:?}"),
            Self::LagTooHigh { lag, ceiling } => write!(f, "lag {lag} > ceiling {ceiling}"),
        }
    }
}

impl std::error::Error for LearnerError {}

/// State for one learner in flight.
pub struct LearnerBootstrap {
    member_id: u64,
    /// Maximum revision lag at which the learner is still "Healthy" enough
    /// to be promoted.
    max_lag: u64,
    inner: RwLock<LearnerInner>,
}

#[derive(Default)]
struct LearnerInner {
    stage: Option<LearnerStage>,
    leader_revision: u64,
    learner_revision: u64,
    transitions: Vec<LearnerStage>,
}

impl LearnerBootstrap {
    pub fn new(member_id: u64, max_lag: u64) -> Self {
        let mut inner = LearnerInner::default();
        inner.stage = Some(LearnerStage::Joining);
        inner.transitions.push(LearnerStage::Joining);
        Self { member_id, max_lag, inner: RwLock::new(inner) }
    }

    pub fn member_id(&self) -> u64 { self.member_id }
    pub fn max_lag(&self) -> u64 { self.max_lag }
    pub fn stage(&self) -> LearnerStage { self.inner.read().unwrap().stage.unwrap() }
    pub fn transitions(&self) -> Vec<LearnerStage> {
        self.inner.read().unwrap().transitions.clone()
    }

    /// Snapshot transfer started (Joining → SnapshotInFlight).
    pub fn begin_snapshot(&self) -> Result<(), LearnerError> {
        self.transition(LearnerStage::SnapshotInFlight)
    }

    /// Snapshot delivered (SnapshotInFlight → CatchingUp).
    pub fn finish_snapshot(&self) -> Result<(), LearnerError> {
        self.transition(LearnerStage::CatchingUp)
    }

    /// Update progress.  When lag drops below `max_lag` we transition to
    /// Healthy.
    pub fn report_progress(&self, leader_rev: u64, learner_rev: u64) -> Result<(), LearnerError> {
        let mut inner = self.inner.write().unwrap();
        inner.leader_revision = leader_rev;
        inner.learner_revision = learner_rev;
        let lag = leader_rev.saturating_sub(learner_rev);
        if lag > self.max_lag {
            return Err(LearnerError::LagTooHigh { lag, ceiling: self.max_lag });
        }
        if matches!(inner.stage, Some(LearnerStage::CatchingUp)) {
            inner.stage = Some(LearnerStage::Healthy);
            inner.transitions.push(LearnerStage::Healthy);
        }
        Ok(())
    }

    /// Promote the learner — only valid from Healthy.
    pub fn promote(&self) -> Result<(), LearnerError> {
        let stage = self.stage();
        if stage != LearnerStage::Healthy {
            return Err(LearnerError::NotReady(stage));
        }
        self.transition(LearnerStage::Promoted)
    }

    /// Current lag in revisions.
    pub fn lag(&self) -> u64 {
        let inner = self.inner.read().unwrap();
        inner.leader_revision.saturating_sub(inner.learner_revision)
    }

    fn transition(&self, to: LearnerStage) -> Result<(), LearnerError> {
        let mut inner = self.inner.write().unwrap();
        let from = inner.stage.unwrap();
        let ok = match (from, to) {
            (LearnerStage::Joining, LearnerStage::SnapshotInFlight) => true,
            (LearnerStage::SnapshotInFlight, LearnerStage::CatchingUp) => true,
            (LearnerStage::CatchingUp, LearnerStage::Healthy) => true,
            (LearnerStage::Healthy, LearnerStage::Promoted) => true,
            // Allow restart: SnapshotInFlight may retry.
            (LearnerStage::SnapshotInFlight, LearnerStage::SnapshotInFlight) => true,
            _ => false,
        };
        if !ok {
            return Err(LearnerError::InvalidTransition { from, to });
        }
        inner.stage = Some(to);
        inner.transitions.push(to);
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M13
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── chunker ──────────────────────────────────────────────────────

    #[test]
    fn test_chunker_uniform_size() {
        // cite: snapshot_sender.go (fixed-size chunks)
        let payload: Vec<u8> = (0..100).collect();
        let chunks = chunk(&payload, 32);
        assert_eq!(chunks.len(), 4); // 32+32+32+4
        assert_eq!(chunks[0].1.len(), 32);
        assert_eq!(chunks[3].1.len(), 4);
    }

    #[test]
    fn test_chunker_exact_multiple() {
        let payload: Vec<u8> = (0..32).collect();
        let chunks = chunk(&payload, 32);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_chunker_empty_payload() {
        assert!(chunk(b"", 32).is_empty());
    }

    #[test]
    fn test_chunker_zero_chunk_size() {
        // cite: defensive — zero would be infinite-loop
        assert!(chunk(b"abc", 0).is_empty());
    }

    #[test]
    fn test_chunker_sequences_monotonic() {
        let payload: Vec<u8> = (0..100).collect();
        let chunks = chunk(&payload, 16);
        for (i, (seq, _)) in chunks.iter().enumerate() {
            assert_eq!(*seq, i as u64);
        }
    }

    // ── SnapshotReceiver ──────────────────────────────────────────────

    #[test]
    fn test_receiver_add_in_order() {
        // cite: snapshot_sender.go (in-order ⇒ assemble succeeds)
        let r = SnapshotReceiver::new(3, 4);
        r.add(0, b"AAAA".to_vec()).unwrap();
        r.add(1, b"BBBB".to_vec()).unwrap();
        r.add(2, b"CCCC".to_vec()).unwrap();
        assert!(r.is_complete());
        assert_eq!(r.assemble().unwrap(), b"AAAABBBBCCCC");
    }

    #[test]
    fn test_receiver_add_out_of_order_then_assembles() {
        // cite: snapshot_sender.go (chunks may arrive out of order)
        let r = SnapshotReceiver::new(3, 4);
        r.add(2, b"CCCC".to_vec()).unwrap();
        r.add(0, b"AAAA".to_vec()).unwrap();
        r.add(1, b"BBBB".to_vec()).unwrap();
        assert_eq!(r.assemble().unwrap(), b"AAAABBBBCCCC");
    }

    #[test]
    fn test_receiver_duplicate_chunk_no_op() {
        // cite: snapshot_sender.go (idempotent receive)
        let r = SnapshotReceiver::new(2, 4);
        assert!(r.add(0, b"AAAA".to_vec()).unwrap());
        assert!(!r.add(0, b"AAAA".to_vec()).unwrap()); // same seq returns false
    }

    #[test]
    fn test_receiver_out_of_range_errors() {
        // cite: snapshot_sender.go (sequence ≥ total ⇒ reject)
        let r = SnapshotReceiver::new(2, 4);
        assert!(r.add(2, b"x".to_vec()).is_err());
    }

    #[test]
    fn test_receiver_missing_lists_unreceived() {
        // cite: snapshot_sender.go (receiver requests missing seqs)
        let r = SnapshotReceiver::new(5, 4);
        r.add(0, b"x".to_vec()).unwrap();
        r.add(2, b"x".to_vec()).unwrap();
        r.add(4, b"x".to_vec()).unwrap();
        assert_eq!(r.missing(), vec![1, 3]);
    }

    #[test]
    fn test_receiver_assemble_errors_when_incomplete() {
        let r = SnapshotReceiver::new(2, 4);
        r.add(0, b"x".to_vec()).unwrap();
        assert!(r.assemble().is_err());
    }

    #[test]
    fn test_receiver_bytes_received_counter() {
        // cite: metrics: snapshot_receive_bytes_total
        let r = SnapshotReceiver::new(3, 4);
        r.add(0, b"ABCD".to_vec()).unwrap();
        r.add(1, b"EFG".to_vec()).unwrap();
        assert_eq!(r.bytes_received(), 7);
    }

    #[test]
    fn test_receiver_is_complete_only_when_all_present() {
        let r = SnapshotReceiver::new(3, 4);
        r.add(0, b"x".to_vec()).unwrap();
        r.add(2, b"x".to_vec()).unwrap();
        assert!(!r.is_complete());
        r.add(1, b"x".to_vec()).unwrap();
        assert!(r.is_complete());
    }

    #[test]
    fn test_chunk_then_reassemble_round_trip() {
        // cite: snapshot_sender.go round-trip
        let payload: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let chunks = chunk(&payload, 64);
        let r = SnapshotReceiver::new(chunks.len() as u64, 64);
        for (s, b) in chunks { r.add(s, b).unwrap(); }
        assert_eq!(r.assemble().unwrap(), payload);
    }

    // ── LearnerBootstrap ──────────────────────────────────────────────

    #[test]
    fn test_learner_starts_in_joining() {
        // cite: peer.go (new learner ⇒ Joining)
        let b = LearnerBootstrap::new(7, 10_000);
        assert_eq!(b.stage(), LearnerStage::Joining);
        assert_eq!(b.member_id(), 7);
    }

    #[test]
    fn test_learner_full_happy_path() {
        // cite: peer.go (Joining → SnapshotInFlight → CatchingUp → Healthy → Promoted)
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        assert_eq!(b.stage(), LearnerStage::SnapshotInFlight);
        b.finish_snapshot().unwrap();
        assert_eq!(b.stage(), LearnerStage::CatchingUp);
        b.report_progress(100, 50).unwrap(); // lag=50 ≤ 100 ⇒ healthy
        assert_eq!(b.stage(), LearnerStage::Healthy);
        b.promote().unwrap();
        assert_eq!(b.stage(), LearnerStage::Promoted);
    }

    #[test]
    fn test_learner_promote_too_early_errors() {
        // cite: peer.go (promotion before Healthy ⇒ NotReady)
        let b = LearnerBootstrap::new(7, 100);
        match b.promote().unwrap_err() {
            LearnerError::NotReady(s) => assert_eq!(s, LearnerStage::Joining),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_learner_invalid_transition_errors() {
        // cite: peer.go (state machine rejects illegal jumps)
        let b = LearnerBootstrap::new(7, 100);
        // Joining → CatchingUp without snapshot
        match b.finish_snapshot().unwrap_err() {
            LearnerError::InvalidTransition { from, to } => {
                assert_eq!(from, LearnerStage::Joining);
                assert_eq!(to, LearnerStage::CatchingUp);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_learner_snapshot_retry_allowed() {
        // cite: peer.go (snapshot retry on transient error)
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        b.begin_snapshot().unwrap();
        assert_eq!(b.stage(), LearnerStage::SnapshotInFlight);
    }

    #[test]
    fn test_learner_lag_too_high_errors() {
        // cite: peer.go (max-lag enforcement)
        let b = LearnerBootstrap::new(7, 50);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        match b.report_progress(1000, 500).unwrap_err() {
            LearnerError::LagTooHigh { lag, ceiling } => {
                assert_eq!(lag, 500);
                assert_eq!(ceiling, 50);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_learner_lag_returns_current_value() {
        let b = LearnerBootstrap::new(7, u64::MAX);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        b.report_progress(200, 150).unwrap();
        assert_eq!(b.lag(), 50);
    }

    #[test]
    fn test_learner_transitions_recorded_in_order() {
        // cite: audit log of stage transitions
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        b.report_progress(100, 100).unwrap();
        b.promote().unwrap();
        assert_eq!(
            b.transitions(),
            vec![
                LearnerStage::Joining,
                LearnerStage::SnapshotInFlight,
                LearnerStage::CatchingUp,
                LearnerStage::Healthy,
                LearnerStage::Promoted,
            ]
        );
    }

    #[test]
    fn test_learner_progress_lag_zero_keeps_healthy() {
        // cite: peer.go (zero lag ⇒ obviously healthy)
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        b.report_progress(50, 50).unwrap();
        assert_eq!(b.stage(), LearnerStage::Healthy);
        assert_eq!(b.lag(), 0);
    }

    #[test]
    fn test_learner_progress_in_catching_up_within_max_lag_promotes_to_healthy() {
        // cite: peer.go (inside ceiling ⇒ Healthy)
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        b.report_progress(200, 150).unwrap();
        assert_eq!(b.stage(), LearnerStage::Healthy);
    }

    #[test]
    fn test_learner_progress_does_not_regress_from_healthy() {
        // cite: peer.go (Healthy stays Healthy on subsequent progress)
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        b.report_progress(100, 100).unwrap();
        assert_eq!(b.stage(), LearnerStage::Healthy);
        b.report_progress(200, 200).unwrap();
        assert_eq!(b.stage(), LearnerStage::Healthy);
    }

    #[test]
    fn test_learner_promote_after_promote_errors() {
        // cite: peer.go (already-promoted ⇒ no-op or error)
        let b = LearnerBootstrap::new(7, 100);
        b.begin_snapshot().unwrap();
        b.finish_snapshot().unwrap();
        b.report_progress(100, 100).unwrap();
        b.promote().unwrap();
        assert!(b.promote().is_err());
    }
}
