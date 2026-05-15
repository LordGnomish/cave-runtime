// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Defrag scheduler — coordinates one-at-a-time defragmentation across
//! cluster members with rate limiting and follower-first ordering.
//!
//! Mirrors etcd v3.6.10
//!   `etcdctl/ctlv3/command/defrag_command.go` (CLI orchestration),
//!   `server/etcdserver/api/v3rpc/maintenance.go#Defragment` (per-member
//!     entry point),
//!   the operator pattern from etcd-druid where one operator-process
//!     defragments members in sequence to preserve quorum.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::cluster_status::{DefragDecision, DefragQuota, DefragTrigger, MemberStatus, defrag_decision};

// ── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum SchedulerError {
    /// Tried to start a defrag while another is already running.
    AlreadyInFlight(u64),
    /// Caller asked for the active defrag but none is running.
    NotInFlight,
    /// Rate limiter rejected the request.
    RateLimited { retry_after: Duration },
    /// Member id unknown to the scheduler.
    UnknownMember(u64),
    /// Cluster has no quorum; refusing to defrag.
    QuorumWouldBreak,
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyInFlight(id) => write!(f, "defrag already in flight for member {id}"),
            Self::NotInFlight => write!(f, "no defrag in flight"),
            Self::RateLimited { retry_after } => write!(f, "rate limited; retry after {retry_after:?}"),
            Self::UnknownMember(id) => write!(f, "unknown member: {id}"),
            Self::QuorumWouldBreak => write!(f, "defrag would break quorum"),
        }
    }
}

impl std::error::Error for SchedulerError {}

// ── In-flight tracker ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct InFlight {
    pub member_id: u64,
    pub trigger: DefragTrigger,
    pub started_at: Instant,
}

#[derive(Debug, Clone)]
pub struct DefragHistoryEntry {
    pub member_id: u64,
    pub trigger: DefragTrigger,
    pub started_at: Instant,
    pub finished_at: Instant,
    pub bytes_freed: u64,
}

// ── Scheduler ─────────────────────────────────────────────────────────────

pub struct DefragScheduler {
    quota: Mutex<DefragQuota>,
    in_flight: Mutex<Option<InFlight>>,
    history: Mutex<VecDeque<DefragHistoryEntry>>,
    history_cap: usize,
    /// Minimum interval between defrags on the *same* member.
    per_member_cooldown: Duration,
    last_run: Mutex<std::collections::BTreeMap<u64, Instant>>,
    queued: AtomicU64,
    completed: AtomicU64,
    skipped: AtomicU64,
}

impl DefragScheduler {
    pub fn new(quota: DefragQuota) -> Self {
        Self {
            quota: Mutex::new(quota),
            in_flight: Mutex::new(None),
            history: Mutex::new(VecDeque::new()),
            history_cap: 64,
            per_member_cooldown: Duration::from_secs(300),
            last_run: Mutex::new(std::collections::BTreeMap::new()),
            queued: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }

    pub fn with_cooldown(mut self, d: Duration) -> Self { self.per_member_cooldown = d; self }
    pub fn with_history_cap(mut self, cap: usize) -> Self { self.history_cap = cap; self }
    pub fn set_quota(&self, q: DefragQuota) { *self.quota.lock().unwrap() = q; }
    pub fn quota(&self) -> DefragQuota { self.quota.lock().unwrap().clone() }

    pub fn queued(&self) -> u64 { self.queued.load(Ordering::SeqCst) }
    pub fn completed(&self) -> u64 { self.completed.load(Ordering::SeqCst) }
    pub fn skipped(&self) -> u64 { self.skipped.load(Ordering::SeqCst) }

    pub fn in_flight(&self) -> Option<InFlight> { self.in_flight.lock().unwrap().clone() }

    pub fn history(&self) -> Vec<DefragHistoryEntry> {
        self.history.lock().unwrap().iter().cloned().collect()
    }

    /// Decide whether `member` should defrag right now, honouring quota,
    /// in-flight, cooldown, and quorum constraints.
    pub fn evaluate(&self, member: &MemberStatus, voter_count: usize, alive_count: usize) -> Result<DefragTrigger, SchedulerError> {
        // Quorum check: defragging this member would temporarily reduce
        // the alive set by one; refuse if that breaks quorum.
        let alive_after = alive_count.saturating_sub(1);
        if alive_after * 2 <= voter_count {
            self.skipped.fetch_add(1, Ordering::SeqCst);
            return Err(SchedulerError::QuorumWouldBreak);
        }

        // In-flight gate.
        if let Some(f) = self.in_flight.lock().unwrap().as_ref() {
            self.skipped.fetch_add(1, Ordering::SeqCst);
            return Err(SchedulerError::AlreadyInFlight(f.member_id));
        }

        // Cooldown gate.
        if let Some(last) = self.last_run.lock().unwrap().get(&member.member_id).copied() {
            let elapsed = last.elapsed();
            if elapsed < self.per_member_cooldown {
                self.skipped.fetch_add(1, Ordering::SeqCst);
                return Err(SchedulerError::RateLimited {
                    retry_after: self.per_member_cooldown - elapsed,
                });
            }
        }

        match defrag_decision(&self.quota.lock().unwrap(), member) {
            DefragDecision::Skip => Err(SchedulerError::RateLimited { retry_after: Duration::from_secs(0) }),
            DefragDecision::Trigger(t) => Ok(t),
        }
    }

    /// Mark `member_id` as starting a defrag.
    pub fn start(&self, member_id: u64, trigger: DefragTrigger) -> Result<(), SchedulerError> {
        let mut g = self.in_flight.lock().unwrap();
        if let Some(f) = g.as_ref() {
            return Err(SchedulerError::AlreadyInFlight(f.member_id));
        }
        *g = Some(InFlight { member_id, trigger, started_at: Instant::now() });
        self.queued.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Mark the current defrag as finished, recording its outcome.
    pub fn finish(&self, member_id: u64, bytes_freed: u64) -> Result<DefragHistoryEntry, SchedulerError> {
        let mut g = self.in_flight.lock().unwrap();
        let f = g.take().ok_or(SchedulerError::NotInFlight)?;
        if f.member_id != member_id {
            // Re-insert and bail.
            *g = Some(f);
            return Err(SchedulerError::UnknownMember(member_id));
        }
        let entry = DefragHistoryEntry {
            member_id: f.member_id,
            trigger: f.trigger,
            started_at: f.started_at,
            finished_at: Instant::now(),
            bytes_freed,
        };
        let mut h = self.history.lock().unwrap();
        if h.len() >= self.history_cap { h.pop_front(); }
        h.push_back(entry.clone());
        self.last_run.lock().unwrap().insert(member_id, Instant::now());
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(entry)
    }

    /// Cancel an in-flight defrag (e.g. node went unhealthy mid-run).
    pub fn cancel(&self, member_id: u64) -> Result<(), SchedulerError> {
        let mut g = self.in_flight.lock().unwrap();
        match g.as_ref() {
            Some(f) if f.member_id == member_id => { *g = None; Ok(()) }
            Some(f) => Err(SchedulerError::AlreadyInFlight(f.member_id)),
            None => Err(SchedulerError::NotInFlight),
        }
    }

    /// Build a follower-first defrag order: leader is always last so we
    /// don't disrupt the leader-elected node mid-batch.
    pub fn order_followers_first(members: &[MemberStatus], leader_id: u64) -> Vec<u64> {
        let mut followers: Vec<u64> = members.iter()
            .filter(|m| m.member_id != leader_id && !m.is_learner)
            .map(|m| m.member_id)
            .collect();
        followers.sort();
        if members.iter().any(|m| m.member_id == leader_id) {
            followers.push(leader_id);
        }
        followers
    }

    /// Total bytes freed across history.
    pub fn total_bytes_freed(&self) -> u64 {
        self.history.lock().unwrap().iter().map(|e| e.bytes_freed).sum()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint M14
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster_status::MemberHealth;

    fn member(id: u64, db: u64, in_use: u64) -> MemberStatus {
        MemberStatus {
            member_id: id, name: format!("m{id}"), revision: 0,
            db_size: db, db_size_in_use: in_use, leader: 0, raft_term: 1,
            is_learner: false, health: MemberHealth::Healthy,
            last_heartbeat_age_secs: Some(0), version: "3.6".into(),
        }
    }

    fn quota_strict() -> DefragQuota {
        DefragQuota { quota_bytes: 100, threshold: 0.8, fragmentation_threshold: 0.5 }
    }

    // ── evaluate / start / finish ──────────────────────────────────────

    #[test]
    fn test_evaluate_triggers_on_quota() {
        // cite: maintenance.go (quota threshold)
        let s = DefragScheduler::new(quota_strict());
        let m = member(1, 90, 80);
        assert_eq!(s.evaluate(&m, 3, 3).unwrap(), DefragTrigger::QuotaThreshold);
    }

    #[test]
    fn test_evaluate_triggers_on_fragmentation() {
        // cite: bbolt fragmentation
        let s = DefragScheduler::new(DefragQuota { quota_bytes: 10_000, threshold: 0.99, fragmentation_threshold: 0.5 });
        let m = member(1, 1000, 200);
        assert_eq!(s.evaluate(&m, 3, 3).unwrap(), DefragTrigger::Fragmented);
    }

    #[test]
    fn test_evaluate_skips_under_threshold() {
        let s = DefragScheduler::new(DefragQuota { quota_bytes: 1000, threshold: 0.99, fragmentation_threshold: 0.0 });
        let m = member(1, 100, 90);
        assert!(s.evaluate(&m, 3, 3).is_err());
    }

    #[test]
    fn test_start_and_finish_record_history() {
        // cite: defrag history (admin tooling)
        let s = DefragScheduler::new(quota_strict());
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        let h = s.finish(1, 1024).unwrap();
        assert_eq!(h.bytes_freed, 1024);
        assert_eq!(s.history().len(), 1);
        assert_eq!(s.completed(), 1);
    }

    #[test]
    fn test_start_twice_errors() {
        // cite: maintenance.go (one-at-a-time)
        let s = DefragScheduler::new(quota_strict());
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        match s.start(2, DefragTrigger::QuotaThreshold).unwrap_err() {
            SchedulerError::AlreadyInFlight(id) => assert_eq!(id, 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_finish_without_start_errors() {
        let s = DefragScheduler::new(quota_strict());
        assert_eq!(s.finish(1, 0).unwrap_err(), SchedulerError::NotInFlight);
    }

    #[test]
    fn test_finish_wrong_member_errors() {
        // cite: defensive — finish must match start
        let s = DefragScheduler::new(quota_strict());
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        let err = s.finish(2, 0).unwrap_err();
        assert_eq!(err, SchedulerError::UnknownMember(2));
        // The in-flight slot is not consumed.
        assert!(s.in_flight().is_some());
    }

    #[test]
    fn test_cancel_clears_in_flight() {
        let s = DefragScheduler::new(quota_strict());
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        s.cancel(1).unwrap();
        assert!(s.in_flight().is_none());
    }

    #[test]
    fn test_cancel_wrong_member_errors() {
        let s = DefragScheduler::new(quota_strict());
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        match s.cancel(2).unwrap_err() {
            SchedulerError::AlreadyInFlight(id) => assert_eq!(id, 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_cancel_when_idle_errors() {
        let s = DefragScheduler::new(quota_strict());
        assert_eq!(s.cancel(1).unwrap_err(), SchedulerError::NotInFlight);
    }

    // ── Cooldown ───────────────────────────────────────────────────────

    #[test]
    fn test_cooldown_blocks_quick_repeat() {
        // cite: rate-limit per member
        let s = DefragScheduler::new(quota_strict()).with_cooldown(Duration::from_secs(60));
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        s.finish(1, 0).unwrap();
        let m = member(1, 90, 80);
        match s.evaluate(&m, 3, 3).unwrap_err() {
            SchedulerError::RateLimited { retry_after } => assert!(retry_after.as_secs() < 60),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_cooldown_zero_allows_immediate_repeat() {
        // cite: zero cooldown ⇒ no rate limit
        let s = DefragScheduler::new(quota_strict()).with_cooldown(Duration::from_secs(0));
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        s.finish(1, 0).unwrap();
        let m = member(1, 90, 80);
        assert!(s.evaluate(&m, 3, 3).is_ok());
    }

    // ── Quorum guard ───────────────────────────────────────────────────

    #[test]
    fn test_quorum_guard_blocks_when_quorum_at_min() {
        // cite: defrag MUST NOT break quorum
        let s = DefragScheduler::new(quota_strict());
        let m = member(1, 90, 80);
        // 3 voters, 2 alive ⇒ defrag-of-one would leave 1 alive ⇒ no quorum
        assert_eq!(s.evaluate(&m, 3, 2).unwrap_err(), SchedulerError::QuorumWouldBreak);
    }

    #[test]
    fn test_quorum_guard_allows_when_safe() {
        let s = DefragScheduler::new(quota_strict());
        let m = member(1, 90, 80);
        // 3 voters, 3 alive ⇒ even after defrag of one, 2 ≥ 2 ⇒ safe
        assert!(s.evaluate(&m, 3, 3).is_ok());
    }

    // ── History ────────────────────────────────────────────────────────

    #[test]
    fn test_history_capped_at_history_cap() {
        // cite: bounded history (no unbounded growth)
        let s = DefragScheduler::new(quota_strict()).with_history_cap(2).with_cooldown(Duration::from_secs(0));
        for i in 1..=5u64 {
            s.start(i, DefragTrigger::ManualOverride).unwrap();
            s.finish(i, 1).unwrap();
        }
        assert_eq!(s.history().len(), 2);
    }

    #[test]
    fn test_total_bytes_freed_sums_history() {
        let s = DefragScheduler::new(quota_strict()).with_cooldown(Duration::from_secs(0));
        for (i, b) in [(1, 100u64), (2, 200), (3, 300)] {
            s.start(i, DefragTrigger::ManualOverride).unwrap();
            s.finish(i, b).unwrap();
        }
        assert_eq!(s.total_bytes_freed(), 600);
    }

    #[test]
    fn test_skipped_counter_ticks() {
        let s = DefragScheduler::new(quota_strict());
        let m = member(1, 90, 80);
        let _ = s.evaluate(&m, 3, 1); // breaks quorum
        let _ = s.evaluate(&m, 3, 1);
        assert_eq!(s.skipped(), 2);
    }

    #[test]
    fn test_queued_counter_ticks_on_start() {
        let s = DefragScheduler::new(quota_strict());
        s.start(1, DefragTrigger::QuotaThreshold).unwrap();
        assert_eq!(s.queued(), 1);
    }

    // ── Order followers first ──────────────────────────────────────────

    #[test]
    fn test_order_followers_first_puts_leader_last() {
        // cite: defrag_command.go (leader defrag last)
        let members = vec![member(1, 0, 0), member(2, 0, 0), member(3, 0, 0)];
        let order = DefragScheduler::order_followers_first(&members, 2);
        assert_eq!(order.last().copied(), Some(2));
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn test_order_followers_first_skips_learners() {
        // cite: learners never defragged in operator mode
        let mut members = vec![member(1, 0, 0), member(2, 0, 0), member(3, 0, 0)];
        members[2].is_learner = true;
        let order = DefragScheduler::order_followers_first(&members, 1);
        assert!(!order.contains(&3));
    }

    #[test]
    fn test_order_followers_first_with_unknown_leader_omits_leader() {
        let members = vec![member(1, 0, 0), member(2, 0, 0)];
        let order = DefragScheduler::order_followers_first(&members, 99);
        assert_eq!(order.len(), 2);
        assert!(!order.contains(&99));
    }

    // ── set_quota ──────────────────────────────────────────────────────

    #[test]
    fn test_set_quota_changes_threshold() {
        let s = DefragScheduler::new(quota_strict());
        let m = member(1, 50, 50);
        assert!(s.evaluate(&m, 3, 3).is_err());
        s.set_quota(DefragQuota { quota_bytes: 10, threshold: 0.5, fragmentation_threshold: 0.0 });
        assert!(s.evaluate(&m, 3, 3).is_ok());
    }
}
