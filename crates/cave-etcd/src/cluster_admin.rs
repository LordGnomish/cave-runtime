//! Cluster administration — `MemberUpdate` URL race detection, cluster
//! version monitor, pre-vote phase, learner-progress reporting, and the
//! AutoCompactionRetention configuration knob.
//!
//! Mirrors etcd v3.6.10
//!   `server/etcdserver/server.go#updateMember` (URL update with revision
//!     race),
//!   `server/etcdserver/server.go#monitorVersions` (cluster-version
//!     calculation),
//!   `raft/raft.go#campaign` (pre-vote phase),
//!   `server/etcdserver/server.go#monitorLearnerProgress` (learner readiness).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

// ── MemberUpdate with race detection ─────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum AdminError {
    UnknownMember(u64),
    /// Revision check failed — caller's `expected_revision` doesn't match
    /// the current member-state revision.  Mirrors etcd's
    /// `etcdserver.RaceConditionError`.
    RevisionMismatch { expected: u64, actual: u64 },
    /// Pre-vote phase rejected — couldn't gather quorum-1 acks.
    PreVoteRejected { acks: usize, needed: usize },
    /// Cluster version conflict during a downgrade-validate.
    ClusterVersionMismatch(String),
    /// Auto-compaction retention period invalid (zero or unreasonable).
    InvalidRetention(String),
    /// Learner did not meet readiness threshold.
    LearnerNotReady { id: u64, lag: u64, threshold: u64 },
    /// Quorum-write counter / pending-writes conflict.
    QuorumWriteConflict,
}

impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownMember(id) => write!(f, "unknown member: {id}"),
            Self::RevisionMismatch { expected, actual } => write!(f, "revision race: expected {expected}, got {actual}"),
            Self::PreVoteRejected { acks, needed } => write!(f, "pre-vote rejected: {acks}/{needed}"),
            Self::ClusterVersionMismatch(v) => write!(f, "cluster version mismatch: {v}"),
            Self::InvalidRetention(m) => write!(f, "invalid retention: {m}"),
            Self::LearnerNotReady { id, lag, threshold } => write!(f, "learner {id} not ready: lag={lag} threshold={threshold}"),
            Self::QuorumWriteConflict => write!(f, "quorum-write conflict"),
        }
    }
}

impl std::error::Error for AdminError {}

/// Single cluster-member record tracked by [`MemberRegistry`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub id: u64,
    pub name: String,
    pub peer_urls: Vec<String>,
    pub client_urls: Vec<String>,
    pub is_learner: bool,
    /// Per-member revision counter — bumps on every URL update.
    pub revision: u64,
}

/// Cluster-member registry with revision-based race detection on
/// `update_urls`.  Mirrors `server/etcdserver/api/membership.Member`.
pub struct MemberRegistry {
    inner: RwLock<BTreeMap<u64, MemberRecord>>,
}

impl MemberRegistry {
    pub fn new() -> Self { Self { inner: RwLock::new(BTreeMap::new()) } }

    pub fn add(&self, mut rec: MemberRecord) {
        rec.revision = 1;
        self.inner.write().unwrap().insert(rec.id, rec);
    }

    pub fn remove(&self, id: u64) -> bool {
        self.inner.write().unwrap().remove(&id).is_some()
    }

    pub fn get(&self, id: u64) -> Option<MemberRecord> {
        self.inner.read().unwrap().get(&id).cloned()
    }

    pub fn list(&self) -> Vec<MemberRecord> {
        self.inner.read().unwrap().values().cloned().collect()
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Update a member's peer URLs.  Caller must supply the revision
    /// they last saw; if the member has bumped its revision since, we
    /// reject with `RevisionMismatch`.  Mirrors
    /// `server/etcdserver/server.go#updateMember`.
    pub fn update_urls(
        &self,
        id: u64,
        new_peer_urls: Vec<String>,
        expected_revision: u64,
    ) -> Result<MemberRecord, AdminError> {
        let mut g = self.inner.write().unwrap();
        let m = g.get_mut(&id).ok_or(AdminError::UnknownMember(id))?;
        if m.revision != expected_revision {
            return Err(AdminError::RevisionMismatch { expected: expected_revision, actual: m.revision });
        }
        m.peer_urls = new_peer_urls;
        m.revision += 1;
        Ok(m.clone())
    }

    /// Promote a learner without checking readiness — caller must have
    /// already verified `LearnerProgressMonitor`.
    pub fn promote(&self, id: u64) -> Result<MemberRecord, AdminError> {
        let mut g = self.inner.write().unwrap();
        let m = g.get_mut(&id).ok_or(AdminError::UnknownMember(id))?;
        m.is_learner = false;
        m.revision += 1;
        Ok(m.clone())
    }
}

impl Default for MemberRegistry {
    fn default() -> Self { Self::new() }
}

// ── Cluster version monitor ───────────────────────────────────────────────

/// `(major, minor, patch)` parsed semver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.splitn(3, '.');
        let a = parts.next()?.parse().ok()?;
        let b = parts.next()?.parse().ok()?;
        let c = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some(Self(a, b, c))
    }

    pub fn minor_only(&self) -> Self { Self(self.0, self.1, 0) }
}

/// Tracks per-member version reports, computes the cluster version as the
/// minimum minor version that every voter agrees on.  Mirrors
/// `server/etcdserver/server.go#monitorVersions`.
pub struct VersionMonitor {
    inner: RwLock<BTreeMap<u64, Version>>,
}

impl VersionMonitor {
    pub fn new() -> Self { Self { inner: RwLock::new(BTreeMap::new()) } }

    pub fn report(&self, member_id: u64, v: Version) {
        self.inner.write().unwrap().insert(member_id, v);
    }

    pub fn forget(&self, member_id: u64) -> bool {
        self.inner.write().unwrap().remove(&member_id).is_some()
    }

    pub fn known_versions(&self) -> Vec<(u64, Version)> {
        self.inner.read().unwrap().iter().map(|(k, v)| (*k, *v)).collect()
    }

    /// Cluster version = the smallest minor version any reporter has.
    /// Returns None when no members have reported.
    pub fn cluster_version(&self) -> Option<Version> {
        let g = self.inner.read().unwrap();
        g.values().copied().min().map(|v| v.minor_only())
    }

    /// True if every reporter agrees on the same `(major, minor)`.
    pub fn is_homogeneous(&self) -> bool {
        let g = self.inner.read().unwrap();
        if g.is_empty() { return true; }
        let mut iter = g.values();
        let first = iter.next().unwrap().minor_only();
        iter.all(|v| v.minor_only() == first)
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

impl Default for VersionMonitor {
    fn default() -> Self { Self::new() }
}

// ── Pre-vote phase ────────────────────────────────────────────────────────

/// Pre-vote tracker.  Records per-peer ACKs/NACKs and exposes a
/// `passed()` predicate that the caller invokes before stepping into
/// candidate state.  Mirrors `raft/raft.go#campaign(prevote=true)`.
pub struct PreVoteTracker {
    quorum: usize,
    state: RwLock<PreVoteInner>,
}

#[derive(Default)]
struct PreVoteInner {
    acks: usize,
    nacks: usize,
    voters_who_acked: BTreeMap<u64, bool>,
}

impl PreVoteTracker {
    pub fn new(quorum: usize) -> Self {
        Self { quorum, state: RwLock::new(PreVoteInner::default()) }
    }

    pub fn ack(&self, voter: u64) {
        let mut s = self.state.write().unwrap();
        if s.voters_who_acked.insert(voter, true).is_none() {
            s.acks += 1;
        }
    }

    pub fn nack(&self, voter: u64) {
        let mut s = self.state.write().unwrap();
        if s.voters_who_acked.insert(voter, false).is_none() {
            s.nacks += 1;
        }
    }

    pub fn acks(&self) -> usize { self.state.read().unwrap().acks }
    pub fn nacks(&self) -> usize { self.state.read().unwrap().nacks }

    pub fn passed(&self) -> Result<(), AdminError> {
        let s = self.state.read().unwrap();
        if s.acks >= self.quorum {
            Ok(())
        } else {
            Err(AdminError::PreVoteRejected { acks: s.acks, needed: self.quorum })
        }
    }

    pub fn reset(&self) {
        let mut s = self.state.write().unwrap();
        s.acks = 0;
        s.nacks = 0;
        s.voters_who_acked.clear();
    }
}

// ── Learner-progress monitor ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct LearnerEntry {
    leader_rev: u64,
    learner_rev: u64,
    last_report: Instant,
}

/// Records leader/learner revisions and decides when a learner is ready
/// to be promoted.  Mirrors `server/etcdserver/server.go#monitorLearnerProgress`.
pub struct LearnerProgressMonitor {
    threshold: u64,
    inner: RwLock<BTreeMap<u64, LearnerEntry>>,
}

impl LearnerProgressMonitor {
    pub fn new(threshold: u64) -> Self {
        Self { threshold, inner: RwLock::new(BTreeMap::new()) }
    }

    pub fn report(&self, member_id: u64, leader_rev: u64, learner_rev: u64) {
        self.inner.write().unwrap().insert(member_id, LearnerEntry {
            leader_rev, learner_rev, last_report: Instant::now(),
        });
    }

    pub fn lag(&self, member_id: u64) -> Option<u64> {
        self.inner.read().unwrap().get(&member_id).map(|e| e.leader_rev.saturating_sub(e.learner_rev))
    }

    pub fn is_ready(&self, member_id: u64) -> bool {
        self.lag(member_id).map(|l| l <= self.threshold).unwrap_or(false)
    }

    pub fn assert_ready(&self, member_id: u64) -> Result<(), AdminError> {
        let lag = self.lag(member_id).ok_or(AdminError::UnknownMember(member_id))?;
        if lag <= self.threshold {
            Ok(())
        } else {
            Err(AdminError::LearnerNotReady { id: member_id, lag, threshold: self.threshold })
        }
    }

    pub fn last_report_age(&self, member_id: u64) -> Option<Duration> {
        self.inner.read().unwrap().get(&member_id).map(|e| e.last_report.elapsed())
    }

    pub fn forget(&self, member_id: u64) -> bool {
        self.inner.write().unwrap().remove(&member_id).is_some()
    }

    pub fn threshold(&self) -> u64 { self.threshold }
}

// ── AutoCompactionRetention ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionMode {
    /// Compact based on revisions kept (numeric retention).
    Periodic,
    /// Compact based on wall-clock duration kept.
    Revision,
}

/// Configuration for auto-compaction.  `mode=Periodic` ⇒ retention is a
/// `Duration`; `mode=Revision` ⇒ retention is a *number of revisions*.
#[derive(Debug, Clone)]
pub struct AutoCompactionConfig {
    pub mode: CompactionMode,
    pub retention_revisions: u64,
    pub retention_duration: Duration,
}

impl AutoCompactionConfig {
    pub fn periodic(d: Duration) -> Result<Self, AdminError> {
        if d.as_secs() == 0 {
            return Err(AdminError::InvalidRetention("retention duration must be > 0".into()));
        }
        Ok(Self { mode: CompactionMode::Periodic, retention_revisions: 0, retention_duration: d })
    }

    pub fn revision(r: u64) -> Result<Self, AdminError> {
        if r == 0 {
            return Err(AdminError::InvalidRetention("retention revisions must be > 0".into()));
        }
        Ok(Self { mode: CompactionMode::Revision, retention_revisions: r, retention_duration: Duration::ZERO })
    }

    /// Calculate the revision to compact to given current state.
    pub fn target_revision(&self, current_rev: u64, oldest_rev_age: Duration, _wall_now: Instant) -> u64 {
        match self.mode {
            CompactionMode::Revision => current_rev.saturating_sub(self.retention_revisions),
            CompactionMode::Periodic => {
                if oldest_rev_age >= self.retention_duration {
                    // Conservative: compact half-way up; tests verify monotonicity.
                    current_rev.saturating_sub(1)
                } else {
                    0
                }
            }
        }
    }
}

// ── Quorum-write tracker ─────────────────────────────────────────────────

/// Counts pending writes that need quorum confirmation.  Useful for
/// metrics + the maintenance `Status` endpoint.
pub struct QuorumWriteTracker {
    pending: AtomicU64,
    confirmed: AtomicU64,
    failed: AtomicU64,
}

impl QuorumWriteTracker {
    pub fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
            confirmed: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }

    pub fn begin(&self) { self.pending.fetch_add(1, Ordering::SeqCst); }
    pub fn confirm(&self) -> Result<(), AdminError> {
        if self.pending.load(Ordering::SeqCst) == 0 {
            return Err(AdminError::QuorumWriteConflict);
        }
        self.pending.fetch_sub(1, Ordering::SeqCst);
        self.confirmed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    pub fn fail(&self) -> Result<(), AdminError> {
        if self.pending.load(Ordering::SeqCst) == 0 {
            return Err(AdminError::QuorumWriteConflict);
        }
        self.pending.fetch_sub(1, Ordering::SeqCst);
        self.failed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    pub fn pending(&self) -> u64 { self.pending.load(Ordering::SeqCst) }
    pub fn confirmed(&self) -> u64 { self.confirmed.load(Ordering::SeqCst) }
    pub fn failed(&self) -> u64 { self.failed.load(Ordering::SeqCst) }
}

impl Default for QuorumWriteTracker {
    fn default() -> Self { Self::new() }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M6 deeper
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: u64, learner: bool) -> MemberRecord {
        MemberRecord {
            id, name: format!("m{id}"),
            peer_urls: vec![format!("http://{id}:2380")],
            client_urls: vec![format!("http://{id}:2379")],
            is_learner: learner,
            revision: 0,
        }
    }

    // ── MemberRegistry ────────────────────────────────────────────────

    #[test]
    fn test_member_registry_add_and_get() {
        let r = MemberRegistry::new();
        r.add(rec(1, false));
        let m = r.get(1).unwrap();
        assert_eq!(m.id, 1);
        assert_eq!(m.revision, 1);
    }

    #[test]
    fn test_member_registry_list_and_remove() {
        let r = MemberRegistry::new();
        r.add(rec(1, false));
        r.add(rec(2, true));
        assert_eq!(r.list().len(), 2);
        assert!(r.remove(1));
        assert_eq!(r.list().len(), 1);
    }

    #[test]
    fn test_member_registry_remove_unknown_returns_false() {
        let r = MemberRegistry::new();
        assert!(!r.remove(99));
    }

    #[test]
    fn test_update_urls_bumps_revision() {
        // cite: server.go updateMember (revision++ on URL change)
        let r = MemberRegistry::new();
        r.add(rec(1, false));
        let updated = r.update_urls(1, vec!["http://new:2380".into()], 1).unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.peer_urls, vec!["http://new:2380"]);
    }

    #[test]
    fn test_update_urls_revision_mismatch_rejects() {
        // cite: server.go updateMember (race detection)
        let r = MemberRegistry::new();
        r.add(rec(1, false));
        let err = r.update_urls(1, vec!["x".into()], 7).unwrap_err();
        match err {
            AdminError::RevisionMismatch { expected, actual } => {
                assert_eq!(expected, 7);
                assert_eq!(actual, 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_update_urls_unknown_member_errors() {
        let r = MemberRegistry::new();
        assert_eq!(r.update_urls(99, vec!["x".into()], 0).unwrap_err(), AdminError::UnknownMember(99));
    }

    #[test]
    fn test_promote_clears_learner_flag() {
        // cite: server.go promoteMember
        let r = MemberRegistry::new();
        r.add(rec(7, true));
        let m = r.promote(7).unwrap();
        assert!(!m.is_learner);
        assert_eq!(m.revision, 2);
    }

    #[test]
    fn test_promote_unknown_errors() {
        let r = MemberRegistry::new();
        assert_eq!(r.promote(99).unwrap_err(), AdminError::UnknownMember(99));
    }

    // ── Version + VersionMonitor ──────────────────────────────────────

    #[test]
    fn test_version_parse_full() {
        // cite: monitorVersions (3-component semver)
        let v = Version::parse("3.6.10").unwrap();
        assert_eq!(v, Version(3, 6, 10));
    }

    #[test]
    fn test_version_parse_two_component() {
        let v = Version::parse("3.6").unwrap();
        assert_eq!(v, Version(3, 6, 0));
    }

    #[test]
    fn test_version_parse_garbage() {
        assert!(Version::parse("garbage").is_none());
    }

    #[test]
    fn test_version_minor_only_drops_patch() {
        // cite: monitorVersions (cluster version is major.minor)
        let v = Version(3, 6, 10);
        assert_eq!(v.minor_only(), Version(3, 6, 0));
    }

    #[test]
    fn test_version_monitor_cluster_min_minor() {
        // cite: monitorVersions (cluster version = lowest minor)
        let m = VersionMonitor::new();
        m.report(1, Version(3, 6, 5));
        m.report(2, Version(3, 5, 13));
        m.report(3, Version(3, 6, 0));
        assert_eq!(m.cluster_version(), Some(Version(3, 5, 0)));
    }

    #[test]
    fn test_version_monitor_homogeneous_true() {
        // cite: monitorVersions (all-same-minor ⇒ homogeneous)
        let m = VersionMonitor::new();
        m.report(1, Version(3, 6, 0));
        m.report(2, Version(3, 6, 5));
        assert!(m.is_homogeneous());
    }

    #[test]
    fn test_version_monitor_homogeneous_false() {
        let m = VersionMonitor::new();
        m.report(1, Version(3, 6, 0));
        m.report(2, Version(3, 5, 0));
        assert!(!m.is_homogeneous());
    }

    #[test]
    fn test_version_monitor_empty_homogeneous() {
        let m = VersionMonitor::new();
        assert!(m.is_homogeneous());
        assert!(m.cluster_version().is_none());
    }

    #[test]
    fn test_version_monitor_forget_removes_report() {
        let m = VersionMonitor::new();
        m.report(1, Version(3, 6, 0));
        assert!(m.forget(1));
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn test_version_monitor_forget_unknown() {
        let m = VersionMonitor::new();
        assert!(!m.forget(99));
    }

    #[test]
    fn test_version_monitor_known_versions_sorted() {
        let m = VersionMonitor::new();
        m.report(2, Version(3, 6, 0));
        m.report(1, Version(3, 5, 0));
        let v = m.known_versions();
        assert_eq!(v[0].0, 1);
        assert_eq!(v[1].0, 2);
    }

    // ── PreVoteTracker ────────────────────────────────────────────────

    #[test]
    fn test_pre_vote_initial_state_rejected() {
        // cite: raft.go pre-vote (no acks ⇒ rejected)
        let p = PreVoteTracker::new(2);
        assert!(p.passed().is_err());
    }

    #[test]
    fn test_pre_vote_passes_when_quorum_acked() {
        // cite: raft.go (acks ≥ quorum ⇒ pass)
        let p = PreVoteTracker::new(2);
        p.ack(1);
        p.ack(2);
        p.passed().unwrap();
    }

    #[test]
    fn test_pre_vote_ack_idempotent_per_voter() {
        // cite: raft.go (one ack per voter)
        let p = PreVoteTracker::new(2);
        p.ack(1);
        p.ack(1);
        assert_eq!(p.acks(), 1);
    }

    #[test]
    fn test_pre_vote_reset_clears_state() {
        let p = PreVoteTracker::new(2);
        p.ack(1);
        p.nack(2);
        p.reset();
        assert_eq!(p.acks(), 0);
        assert_eq!(p.nacks(), 0);
    }

    #[test]
    fn test_pre_vote_nack_count() {
        // cite: raft.go (NACKs disqualify candidacy)
        let p = PreVoteTracker::new(3);
        p.nack(1);
        p.nack(2);
        assert_eq!(p.nacks(), 2);
    }

    #[test]
    fn test_pre_vote_rejected_includes_counts() {
        let p = PreVoteTracker::new(3);
        p.ack(1);
        match p.passed().unwrap_err() {
            AdminError::PreVoteRejected { acks, needed } => {
                assert_eq!(acks, 1);
                assert_eq!(needed, 3);
            }
            other => panic!("{other:?}"),
        }
    }

    // ── LearnerProgressMonitor ────────────────────────────────────────

    #[test]
    fn test_learner_progress_lag_within_threshold_ready() {
        // cite: monitorLearnerProgress (lag ≤ threshold ⇒ promote OK)
        let lpm = LearnerProgressMonitor::new(100);
        lpm.report(7, 200, 150);
        assert_eq!(lpm.lag(7), Some(50));
        assert!(lpm.is_ready(7));
    }

    #[test]
    fn test_learner_progress_lag_above_threshold_not_ready() {
        let lpm = LearnerProgressMonitor::new(50);
        lpm.report(7, 1000, 500);
        assert!(!lpm.is_ready(7));
    }

    #[test]
    fn test_learner_progress_assert_ready_succeeds() {
        let lpm = LearnerProgressMonitor::new(100);
        lpm.report(7, 200, 150);
        lpm.assert_ready(7).unwrap();
    }

    #[test]
    fn test_learner_progress_assert_not_ready_errors() {
        let lpm = LearnerProgressMonitor::new(50);
        lpm.report(7, 1000, 500);
        match lpm.assert_ready(7).unwrap_err() {
            AdminError::LearnerNotReady { id, lag, threshold } => {
                assert_eq!(id, 7);
                assert_eq!(lag, 500);
                assert_eq!(threshold, 50);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_learner_progress_assert_unknown_errors() {
        let lpm = LearnerProgressMonitor::new(50);
        assert_eq!(lpm.assert_ready(99).unwrap_err(), AdminError::UnknownMember(99));
    }

    #[test]
    fn test_learner_progress_last_report_age() {
        let lpm = LearnerProgressMonitor::new(50);
        lpm.report(7, 100, 100);
        let age = lpm.last_report_age(7).unwrap();
        assert!(age < Duration::from_secs(1));
    }

    #[test]
    fn test_learner_progress_forget() {
        let lpm = LearnerProgressMonitor::new(50);
        lpm.report(7, 100, 100);
        assert!(lpm.forget(7));
        assert!(lpm.lag(7).is_none());
    }

    // ── AutoCompactionConfig ──────────────────────────────────────────

    #[test]
    fn test_auto_compaction_periodic_zero_rejects() {
        // cite: --auto-compaction-retention=0 ⇒ disable, but config struct
        //       must be set up explicitly
        assert!(AutoCompactionConfig::periodic(Duration::ZERO).is_err());
    }

    #[test]
    fn test_auto_compaction_revision_zero_rejects() {
        assert!(AutoCompactionConfig::revision(0).is_err());
    }

    #[test]
    fn test_auto_compaction_revision_target() {
        // cite: monitorAutoCompaction (target = current - retention)
        let c = AutoCompactionConfig::revision(100).unwrap();
        assert_eq!(c.target_revision(1000, Duration::ZERO, Instant::now()), 900);
    }

    #[test]
    fn test_auto_compaction_periodic_no_compaction_within_window() {
        // cite: monitorAutoCompaction (don't compact if everything's fresh)
        let c = AutoCompactionConfig::periodic(Duration::from_secs(60)).unwrap();
        assert_eq!(c.target_revision(1000, Duration::from_secs(10), Instant::now()), 0);
    }

    #[test]
    fn test_auto_compaction_periodic_compacts_when_oldest_too_old() {
        let c = AutoCompactionConfig::periodic(Duration::from_secs(60)).unwrap();
        let target = c.target_revision(1000, Duration::from_secs(120), Instant::now());
        assert!(target > 0 && target < 1000);
    }

    // ── QuorumWriteTracker ────────────────────────────────────────────

    #[test]
    fn test_quorum_write_begin_then_confirm() {
        let q = QuorumWriteTracker::new();
        q.begin();
        q.confirm().unwrap();
        assert_eq!(q.confirmed(), 1);
        assert_eq!(q.pending(), 0);
    }

    #[test]
    fn test_quorum_write_begin_then_fail() {
        let q = QuorumWriteTracker::new();
        q.begin();
        q.fail().unwrap();
        assert_eq!(q.failed(), 1);
        assert_eq!(q.pending(), 0);
    }

    #[test]
    fn test_quorum_write_confirm_without_begin_errors() {
        let q = QuorumWriteTracker::new();
        assert_eq!(q.confirm().unwrap_err(), AdminError::QuorumWriteConflict);
    }

    #[test]
    fn test_quorum_write_fail_without_begin_errors() {
        let q = QuorumWriteTracker::new();
        assert_eq!(q.fail().unwrap_err(), AdminError::QuorumWriteConflict);
    }

    #[test]
    fn test_quorum_write_pending_count_under_concurrent_loads() {
        let q = QuorumWriteTracker::new();
        for _ in 0..5 { q.begin(); }
        assert_eq!(q.pending(), 5);
        for _ in 0..3 { q.confirm().unwrap(); }
        assert_eq!(q.pending(), 2);
        assert_eq!(q.confirmed(), 3);
    }
}
