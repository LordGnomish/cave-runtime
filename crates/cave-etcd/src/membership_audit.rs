//! Membership-change audit log.
//!
//! Records every cluster-membership transition (`add`, `remove`,
//! `update`, `promote`, `enter_joint`, `leave_joint`) as a structured
//! event so admins can reconstruct the cluster's history without
//! replaying the raft log.
//!
//! Mirrors etcd v3.6.10
//!   `server/etcdserver/api/membership/cluster.go#applyConfChange`
//!   `server/etcdserver/server.go#configurationChangeApplied` (audit
//!   sink).

use crate::models::{JointConfig, Member};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Action discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MembershipAction {
    Add,
    Remove,
    Update,
    Promote,
    EnterJoint,
    LeaveJoint,
}

/// One audit entry — fully self-contained so a downstream consumer can
/// render the transition without consulting other state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipAuditEvent {
    pub seq: u64,
    pub timestamp: DateTime<Utc>,
    pub action: MembershipAction,
    /// Member-id the change targeted (or 0 for joint-config events).
    pub member_id: u64,
    /// Snapshot of the *member set* before the change.
    pub before: Vec<Member>,
    /// Snapshot of the *member set* after the change.
    pub after: Vec<Member>,
    /// Joint-config snapshot at the time of the event (if any).
    pub joint: Option<JointConfig>,
    /// Free-form note (e.g. "auto-leave triggered by commit_index ≥ N").
    pub note: Option<String>,
}

/// In-memory audit log.  Supports an optional max-size cap that ring-
/// buffers older entries off the front.  `seq` is monotonic across the
/// process lifetime regardless of pruning.
pub struct MembershipAuditLog {
    inner: RwLock<Vec<MembershipAuditEvent>>,
    next_seq: std::sync::atomic::AtomicU64,
    /// `0` ⇒ unbounded.
    max_entries: std::sync::atomic::AtomicUsize,
}

impl MembershipAuditLog {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
            next_seq: std::sync::atomic::AtomicU64::new(1),
            max_entries: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn with_cap(cap: usize) -> Self {
        let log = Self::new();
        log.set_cap(cap);
        log
    }

    pub fn set_cap(&self, cap: usize) {
        self.max_entries.store(cap, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn cap(&self) -> usize {
        self.max_entries.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Record one event.  Returns the assigned sequence number.
    pub fn record(&self, ev: MembershipAuditEvent) -> u64 {
        let seq = ev.seq;
        let mut g = self.inner.write().unwrap();
        g.push(ev);
        let cap = self.cap();
        if cap > 0 {
            while g.len() > cap {
                g.remove(0);
            }
        }
        seq
    }

    /// Build the next event header — caller fills in `before/after/...`.
    pub fn new_event(
        &self,
        action: MembershipAction,
        member_id: u64,
        before: Vec<Member>,
        after: Vec<Member>,
        joint: Option<JointConfig>,
        note: Option<String>,
    ) -> MembershipAuditEvent {
        let seq = self
            .next_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        MembershipAuditEvent {
            seq,
            timestamp: Utc::now(),
            action,
            member_id,
            before,
            after,
            joint,
            note,
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().is_empty()
    }

    /// Snapshot of all entries currently retained.
    pub fn entries(&self) -> Vec<MembershipAuditEvent> {
        self.inner.read().unwrap().clone()
    }

    /// Find the most-recent event matching `pred`.  Returns `None` when
    /// no matching event has ever been recorded *and* survived ring-
    /// buffering.
    pub fn find_last<F>(&self, pred: F) -> Option<MembershipAuditEvent>
    where
        F: Fn(&MembershipAuditEvent) -> bool,
    {
        let g = self.inner.read().unwrap();
        g.iter().rev().find(|e| pred(e)).cloned()
    }

    /// All events for a single `member_id`.
    pub fn entries_for(&self, member_id: u64) -> Vec<MembershipAuditEvent> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.member_id == member_id)
            .cloned()
            .collect()
    }
}

impl Default for MembershipAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Membership-audit tests — feat/cave-etcd-deeper-003
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_member(id: u64, name: &str) -> Member {
        Member {
            id,
            name: name.into(),
            peer_urls: vec![format!("http://m{id}:2380")],
            client_urls: vec![],
            is_learner: false,
        }
    }

    #[test]
    fn test_audit_record_assigns_monotonic_seq() {
        // cite: etcd v3.6.10 (audit log is monotonic)
        let _tenant_id = "ma-001";
        let log = MembershipAuditLog::new();
        let e1 = log.new_event(MembershipAction::Add, 2, vec![], vec![dummy_member(2, "m2")], None, None);
        let s1 = log.record(e1);
        let e2 = log.new_event(MembershipAction::Remove, 2, vec![dummy_member(2, "m2")], vec![], None, None);
        let s2 = log.record(e2);
        assert_eq!(s2, s1 + 1);
    }

    #[test]
    fn test_audit_entries_for_filters_by_member() {
        // cite: etcd v3.6.10 (per-member history filtering)
        let _tenant_id = "ma-002";
        let log = MembershipAuditLog::new();
        log.record(log.new_event(MembershipAction::Add, 5, vec![], vec![], None, None));
        log.record(log.new_event(MembershipAction::Add, 6, vec![], vec![], None, None));
        log.record(log.new_event(MembershipAction::Update, 5, vec![], vec![], None, None));
        let for_5 = log.entries_for(5);
        assert_eq!(for_5.len(), 2);
        assert!(for_5.iter().all(|e| e.member_id == 5));
    }

    #[test]
    fn test_audit_find_last_returns_most_recent() {
        // cite: etcd v3.6.10 (most-recent transition lookup)
        let _tenant_id = "ma-003";
        let log = MembershipAuditLog::new();
        log.record(log.new_event(MembershipAction::Add, 1, vec![], vec![], None, None));
        log.record(log.new_event(MembershipAction::Promote, 1, vec![], vec![], None, None));
        let last = log
            .find_last(|e| matches!(e.action, MembershipAction::Add | MembershipAction::Promote))
            .unwrap();
        assert_eq!(last.action, MembershipAction::Promote);
    }

    #[test]
    fn test_audit_with_cap_truncates_oldest() {
        // cite: etcd v3.6.10 (ring-buffer the oldest)
        let _tenant_id = "ma-004";
        let log = MembershipAuditLog::with_cap(3);
        for i in 0..10 {
            log.record(log.new_event(MembershipAction::Add, i, vec![], vec![], None, None));
        }
        assert_eq!(log.len(), 3);
        let entries = log.entries();
        // After ring-buffering, only the last 3 events survive.
        let ids: Vec<u64> = entries.iter().map(|e| e.member_id).collect();
        assert_eq!(ids, vec![7, 8, 9]);
    }

    #[test]
    fn test_audit_record_carries_before_and_after_snapshots() {
        // cite: etcd v3.6.10 (before/after snapshot for replay)
        let _tenant_id = "ma-005";
        let log = MembershipAuditLog::new();
        let before = vec![dummy_member(1, "m1")];
        let after = vec![dummy_member(1, "m1"), dummy_member(2, "m2")];
        log.record(log.new_event(
            MembershipAction::Add,
            2,
            before.clone(),
            after.clone(),
            None,
            Some("manual add".into()),
        ));
        let entries = log.entries();
        assert_eq!(entries[0].before.len(), 1);
        assert_eq!(entries[0].after.len(), 2);
        assert_eq!(entries[0].note.as_deref(), Some("manual add"));
    }

    #[test]
    fn test_audit_record_joint_snapshot() {
        // cite: etcd v3.6.10 (joint config audit)
        let _tenant_id = "ma-006";
        let log = MembershipAuditLog::new();
        let joint = JointConfig {
            outgoing: vec![1, 2, 3],
            incoming: vec![1, 2, 4],
            learners: vec![5],
        };
        log.record(log.new_event(
            MembershipAction::EnterJoint,
            0,
            vec![],
            vec![],
            Some(joint.clone()),
            None,
        ));
        let entries = log.entries();
        assert_eq!(entries[0].joint, Some(joint));
    }

    #[test]
    fn test_audit_default_cap_is_unbounded() {
        // cite: etcd v3.6.10 (default unbounded; admin opts in)
        let _tenant_id = "ma-007";
        let log = MembershipAuditLog::default();
        for i in 0..1000 {
            log.record(log.new_event(MembershipAction::Add, i, vec![], vec![], None, None));
        }
        assert_eq!(log.len(), 1000);
        assert_eq!(log.cap(), 0);
    }
}
