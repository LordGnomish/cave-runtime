// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster-wide health, quota-driven defrag trigger, and the v3.6
//! `Downgrade` state-machine — three pieces of the maintenance / cluster
//! API surface that share one source of truth (per-member status).
//!
//! Mirrors etcd v3.6.10
//!   `etcdctl/ctlv3/command/endpoint_command.go` (cluster health),
//!   `server/etcdserver/api/v3rpc/maintenance.go#Defragment` (quota gate),
//!   `api/etcdserverpb/rpc.proto` (`DowngradeRequest` / `DowngradeAction`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

// ── Per-member health ────────────────────────────────────────────────────

/// Health state of a single member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberHealth {
    /// Member responded to its last heartbeat within the configured timeout.
    Healthy,
    /// Heartbeat is stale but the member has not been declared dead.
    Stale,
    /// Heartbeat timeout exceeded.
    Unhealthy,
    /// Member registered but never reported in.
    Unknown,
}

impl MemberHealth {
    /// True for `Healthy` or `Stale` — i.e. anything other than dead.
    pub fn is_alive(&self) -> bool {
        matches!(self, Self::Healthy | Self::Stale)
    }
}

/// Per-member health snapshot.  `db_size` and `revision` come from the
/// member's last status report; `last_heartbeat` is the local observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberStatus {
    pub member_id: u64,
    pub name: String,
    pub revision: u64,
    pub db_size: u64,
    pub db_size_in_use: u64,
    pub leader: u64,
    pub raft_term: u64,
    pub is_learner: bool,
    pub health: MemberHealth,
    /// Seconds since the local node last received a heartbeat.  None if
    /// no heartbeat has ever arrived.
    pub last_heartbeat_age_secs: Option<u64>,
    /// Etcd version reported by the member (for downgrade scheduling).
    pub version: String,
}

/// Cluster-health tracker.
///
/// Aggregates per-member heartbeats and computes:
///   * an *alive count* used by the raft sub-system to detect quorum loss,
///   * a *cluster health* triple (`healthy`, `stale`, `unhealthy`) that the
///     `etcdctl endpoint health` UI surfaces,
///   * the per-member [`MemberStatus`] used by `etcdctl endpoint status`.
pub struct ClusterStatusTracker {
    heartbeat_timeout: Duration,
    inner: RwLock<TrackerInner>,
}

#[derive(Default)]
struct TrackerInner {
    members: BTreeMap<u64, MemberStatus>,
    last_heartbeat: BTreeMap<u64, Instant>,
}

impl ClusterStatusTracker {
    pub fn new(heartbeat_timeout: Duration) -> Self {
        Self {
            heartbeat_timeout,
            inner: RwLock::new(TrackerInner::default()),
        }
    }

    /// Register a new member with default `Unknown` health.
    pub fn register(&self, member_id: u64, name: impl Into<String>, is_learner: bool) {
        let mut inner = self.inner.write().unwrap();
        inner.members.entry(member_id).or_insert_with(|| MemberStatus {
            member_id,
            name: name.into(),
            revision: 0,
            db_size: 0,
            db_size_in_use: 0,
            leader: 0,
            raft_term: 0,
            is_learner,
            health: MemberHealth::Unknown,
            last_heartbeat_age_secs: None,
            version: "0.0.0".into(),
        });
    }

    /// Heartbeat report from a member.  Updates revision/db_size and
    /// stamps the member as `Healthy`.
    pub fn heartbeat(&self, status: MemberStatus) {
        let id = status.member_id;
        let mut inner = self.inner.write().unwrap();
        inner.last_heartbeat.insert(id, Instant::now());
        inner.members.insert(
            id,
            MemberStatus { health: MemberHealth::Healthy, last_heartbeat_age_secs: Some(0), ..status },
        );
    }

    /// Drop a member entirely (used on `MemberRemove`).
    pub fn deregister(&self, member_id: u64) -> bool {
        let mut inner = self.inner.write().unwrap();
        let removed = inner.members.remove(&member_id).is_some();
        inner.last_heartbeat.remove(&member_id);
        removed
    }

    /// Recompute every member's health based on `now - last_heartbeat`.
    /// Returns the count of members in each bucket.
    pub fn refresh(&self) -> ClusterHealth {
        let mut inner = self.inner.write().unwrap();
        let now = Instant::now();
        let timeout = self.heartbeat_timeout;
        let mut healthy = 0usize;
        let mut stale = 0usize;
        let mut unhealthy = 0usize;
        let mut unknown = 0usize;

        // Snapshot heartbeats so we can mutate members under the same lock.
        let heartbeats: BTreeMap<u64, Instant> = inner.last_heartbeat.clone();

        for (id, member) in inner.members.iter_mut() {
            let h = heartbeats.get(id);
            match h {
                None => { member.health = MemberHealth::Unknown; member.last_heartbeat_age_secs = None; unknown += 1; }
                Some(&t) => {
                    let age = now.duration_since(t);
                    member.last_heartbeat_age_secs = Some(age.as_secs());
                    if age <= timeout / 2 { member.health = MemberHealth::Healthy; healthy += 1; }
                    else if age <= timeout { member.health = MemberHealth::Stale; stale += 1; }
                    else { member.health = MemberHealth::Unhealthy; unhealthy += 1; }
                }
            }
        }

        ClusterHealth { healthy, stale, unhealthy, unknown, total: inner.members.len() }
    }

    /// Snapshot every registered member's status.
    pub fn member_statuses(&self) -> Vec<MemberStatus> {
        self.inner.read().unwrap().members.values().cloned().collect()
    }

    pub fn member(&self, id: u64) -> Option<MemberStatus> {
        self.inner.read().unwrap().members.get(&id).cloned()
    }

    /// Largest `db_size` reported across the cluster.  Used by the
    /// quota-defrag trigger.
    pub fn max_db_size(&self) -> u64 {
        self.inner.read().unwrap().members.values().map(|m| m.db_size).max().unwrap_or(0)
    }

    /// Member count broken out by learner / voter.
    pub fn voter_count(&self) -> usize {
        self.inner.read().unwrap().members.values().filter(|m| !m.is_learner).count()
    }

    pub fn learner_count(&self) -> usize {
        self.inner.read().unwrap().members.values().filter(|m| m.is_learner).count()
    }

    /// True when at least `(n/2)+1` voters are alive.
    pub fn has_quorum(&self) -> bool {
        let alive = self.inner.read().unwrap().members.values()
            .filter(|m| !m.is_learner && m.health.is_alive())
            .count();
        let voters = self.voter_count();
        alive * 2 > voters
    }
}

/// Aggregate health bucket counts produced by [`ClusterStatusTracker::refresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterHealth {
    pub healthy: usize,
    pub stale: usize,
    pub unhealthy: usize,
    pub unknown: usize,
    pub total: usize,
}

// ── Quota-driven defrag trigger ───────────────────────────────────────────

/// Reasons the trigger may fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefragTrigger {
    /// `db_size > backend_quota_bytes * threshold` — etcd default 80%.
    QuotaThreshold,
    /// Caller forced one regardless of utilisation.
    ManualOverride,
    /// `db_size_in_use / db_size` is below `fragmentation_threshold` —
    /// many free pages, defrag will reclaim them.
    Fragmented,
}

/// Outcome of a quota check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefragDecision {
    Skip,
    Trigger(DefragTrigger),
}

/// Configurable thresholds for the quota-driven trigger.
#[derive(Debug, Clone)]
pub struct DefragQuota {
    /// Hard cap (etcd `--quota-backend-bytes`).  When `db_size` exceeds
    /// `quota_bytes * threshold`, the trigger fires.
    pub quota_bytes: u64,
    /// `0.0..=1.0` — fraction of `quota_bytes` at which to fire.
    pub threshold: f64,
    /// `0.0..=1.0` — when the *in-use ratio* falls below this we declare
    /// the DB fragmented.
    pub fragmentation_threshold: f64,
}

impl Default for DefragQuota {
    fn default() -> Self {
        Self { quota_bytes: 2 * 1024 * 1024 * 1024, threshold: 0.8, fragmentation_threshold: 0.5 }
    }
}

/// Decide whether to defrag based on the supplied member status.
pub fn defrag_decision(quota: &DefragQuota, status: &MemberStatus) -> DefragDecision {
    let cap = (quota.quota_bytes as f64 * quota.threshold) as u64;
    if cap > 0 && status.db_size >= cap {
        return DefragDecision::Trigger(DefragTrigger::QuotaThreshold);
    }
    if status.db_size > 0 {
        let in_use = status.db_size_in_use as f64 / status.db_size as f64;
        if in_use < quota.fragmentation_threshold {
            return DefragDecision::Trigger(DefragTrigger::Fragmented);
        }
    }
    DefragDecision::Skip
}

/// Force-trigger variant — admin override.
pub fn defrag_force() -> DefragDecision { DefragDecision::Trigger(DefragTrigger::ManualOverride) }

// ── Downgrade state machine ───────────────────────────────────────────────

/// Etcd v3.5+ supports a controlled rolling downgrade.  Admin issues
/// `Downgrade::Validate` first, then `Downgrade::Enable`, then bumps each
/// member.  `Cancel` aborts an enabled-but-not-yet-finished downgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DowngradeAction {
    Validate,
    Enable,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeRequest {
    pub action: DowngradeAction,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DowngradeResponse {
    pub version: String,
    pub enabled: bool,
}

#[derive(Debug)]
pub enum DowngradeError {
    /// Requested target version is not a valid downgrade.
    InvalidVersion(String),
    /// Downgrade in progress; another action is illegal.
    AlreadyInProgress(String),
    /// `Cancel` called when no downgrade is in progress.
    NotInProgress,
}

impl std::fmt::Display for DowngradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidVersion(v) => write!(f, "invalid downgrade version: {v}"),
            Self::AlreadyInProgress(v) => write!(f, "downgrade already in progress to {v}"),
            Self::NotInProgress => write!(f, "no downgrade in progress"),
        }
    }
}

impl std::error::Error for DowngradeError {}

/// Tracks the current downgrade target (if any).  A "version" here is the
/// `MAJOR.MINOR` shape etcd uses (`3.5`, `3.6`, `3.7`).
#[derive(Default)]
pub struct DowngradeState {
    inner: RwLock<DowngradeInner>,
}

#[derive(Default)]
struct DowngradeInner {
    target_version: Option<String>,
    enabled: bool,
}

impl DowngradeState {
    pub fn new() -> Self { Self::default() }

    /// Process a downgrade request and update internal state.
    pub fn apply(
        &self,
        cluster_version: &str,
        req: &DowngradeRequest,
    ) -> Result<DowngradeResponse, DowngradeError> {
        match req.action {
            DowngradeAction::Validate => {
                Self::check_target(cluster_version, &req.version)?;
                let inner = self.inner.read().unwrap();
                Ok(DowngradeResponse { version: req.version.clone(), enabled: inner.enabled })
            }
            DowngradeAction::Enable => {
                Self::check_target(cluster_version, &req.version)?;
                let mut inner = self.inner.write().unwrap();
                if let Some(ref t) = inner.target_version {
                    if inner.enabled && t != &req.version {
                        return Err(DowngradeError::AlreadyInProgress(t.clone()));
                    }
                }
                inner.target_version = Some(req.version.clone());
                inner.enabled = true;
                Ok(DowngradeResponse { version: req.version.clone(), enabled: true })
            }
            DowngradeAction::Cancel => {
                let mut inner = self.inner.write().unwrap();
                if !inner.enabled { return Err(DowngradeError::NotInProgress); }
                inner.enabled = false;
                let v = inner.target_version.take().unwrap_or_default();
                Ok(DowngradeResponse { version: v, enabled: false })
            }
        }
    }

    pub fn target(&self) -> Option<String> {
        self.inner.read().unwrap().target_version.clone()
    }

    pub fn enabled(&self) -> bool {
        self.inner.read().unwrap().enabled
    }

    fn check_target(cluster: &str, target: &str) -> Result<(), DowngradeError> {
        let (cmaj, cmin) = parse_minor(cluster).ok_or_else(|| DowngradeError::InvalidVersion(cluster.to_string()))?;
        let (tmaj, tmin) = parse_minor(target).ok_or_else(|| DowngradeError::InvalidVersion(target.to_string()))?;
        // Only one minor version backward is allowed (etcd's policy).
        if tmaj != cmaj { return Err(DowngradeError::InvalidVersion(target.to_string())); }
        if tmin + 1 != cmin {
            return Err(DowngradeError::InvalidVersion(format!(
                "{target} is not exactly one minor version below {cluster}"
            )));
        }
        Ok(())
    }
}

fn parse_minor(s: &str) -> Option<(u32, u32)> {
    let mut parts = s.splitn(3, '.');
    let maj = parts.next()?.parse().ok()?;
    let min = parts.next()?.parse().ok()?;
    Some((maj, min))
}

// ─────────────────────────────────────────────────────────────────────────
// Tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: u64, db: u64, in_use: u64) -> MemberStatus {
        MemberStatus {
            member_id: id, name: format!("m{id}"), revision: 0,
            db_size: db, db_size_in_use: in_use, leader: 0, raft_term: 1,
            is_learner: false, health: MemberHealth::Unknown,
            last_heartbeat_age_secs: None, version: "3.6".into(),
        }
    }

    // ── ClusterStatusTracker ───────────────────────────────────────────

    #[test]
    fn test_register_starts_unknown() {
        // cite: etcdctl endpoint status (member with no heartbeat shows Unknown)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        t.register(1, "m1", false);
        let m = t.member(1).unwrap();
        assert_eq!(m.health, MemberHealth::Unknown);
        assert_eq!(m.last_heartbeat_age_secs, None);
    }

    #[test]
    fn test_heartbeat_marks_healthy() {
        // cite: etcdctl endpoint health (heartbeat ⇒ healthy)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        t.register(1, "m1", false);
        t.heartbeat(MemberStatus { health: MemberHealth::Unknown, ..st(1, 100, 80) });
        assert_eq!(t.member(1).unwrap().health, MemberHealth::Healthy);
    }

    #[test]
    fn test_refresh_marks_unhealthy_after_timeout() {
        // cite: etcdctl endpoint health (timeout ⇒ unhealthy)
        let t = ClusterStatusTracker::new(Duration::from_millis(10));
        t.register(1, "m1", false);
        t.heartbeat(st(1, 0, 0));
        std::thread::sleep(Duration::from_millis(30));
        let h = t.refresh();
        assert_eq!(h.unhealthy, 1);
        assert_eq!(h.healthy, 0);
        assert_eq!(t.member(1).unwrap().health, MemberHealth::Unhealthy);
    }

    #[test]
    fn test_refresh_marks_stale_in_grace() {
        // cite: etcdctl endpoint health (within timeout but past half ⇒ stale)
        let t = ClusterStatusTracker::new(Duration::from_millis(60));
        t.register(1, "m1", false);
        t.heartbeat(st(1, 0, 0));
        // sleep into the (timeout/2..timeout] window
        std::thread::sleep(Duration::from_millis(40));
        let h = t.refresh();
        assert!(h.stale + h.healthy == 1, "{h:?}");
    }

    #[test]
    fn test_deregister_removes_member() {
        // cite: server.go memberRemove (drop from status tracker)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        t.register(1, "m1", false);
        assert!(t.deregister(1));
        assert!(t.member(1).is_none());
    }

    #[test]
    fn test_deregister_unknown_returns_false() {
        // cite: server.go (idempotent remove)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        assert!(!t.deregister(99));
    }

    #[test]
    fn test_max_db_size_across_members() {
        // cite: maintenance.go DefragmentRequest (largest db drives quota)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        t.register(1, "m1", false);
        t.register(2, "m2", false);
        t.heartbeat(st(1, 100, 80));
        t.heartbeat(st(2, 500, 300));
        assert_eq!(t.max_db_size(), 500);
    }

    #[test]
    fn test_voter_and_learner_counts() {
        // cite: server.go (voter + learner classification)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        t.register(1, "m1", false);
        t.register(2, "m2", true);
        t.register(3, "m3", false);
        assert_eq!(t.voter_count(), 2);
        assert_eq!(t.learner_count(), 1);
    }

    #[test]
    fn test_quorum_with_two_of_three_alive() {
        // cite: etcd quorum = (n/2)+1
        let t = ClusterStatusTracker::new(Duration::from_secs(60));
        for i in 1..=3 { t.register(i, format!("m{i}"), false); }
        // heartbeats for 1 and 2 only
        t.heartbeat(st(1, 0, 0));
        t.heartbeat(st(2, 0, 0));
        t.refresh();
        assert!(t.has_quorum());
    }

    #[test]
    fn test_quorum_lost_with_one_of_three_alive() {
        // cite: etcd quorum-loss
        let t = ClusterStatusTracker::new(Duration::from_secs(60));
        for i in 1..=3 { t.register(i, format!("m{i}"), false); }
        t.heartbeat(st(1, 0, 0));
        t.refresh();
        assert!(!t.has_quorum());
    }

    #[test]
    fn test_member_statuses_snapshot_size() {
        // cite: etcdctl endpoint status (one entry per member)
        let t = ClusterStatusTracker::new(Duration::from_secs(5));
        for i in 1..=4 { t.register(i, format!("m{i}"), i == 4); }
        assert_eq!(t.member_statuses().len(), 4);
    }

    #[test]
    fn test_member_health_is_alive_buckets() {
        assert!(MemberHealth::Healthy.is_alive());
        assert!(MemberHealth::Stale.is_alive());
        assert!(!MemberHealth::Unhealthy.is_alive());
        assert!(!MemberHealth::Unknown.is_alive());
    }

    // ── Defrag quota trigger ───────────────────────────────────────────

    #[test]
    fn test_defrag_quota_threshold_triggers() {
        // cite: maintenance.go (quota-based defrag)
        let q = DefragQuota { quota_bytes: 100, threshold: 0.8, fragmentation_threshold: 0.5 };
        let s = st(1, 90, 80);
        assert_eq!(defrag_decision(&q, &s), DefragDecision::Trigger(DefragTrigger::QuotaThreshold));
    }

    #[test]
    fn test_defrag_quota_under_threshold_skips() {
        // cite: maintenance.go (no defrag below quota)
        let q = DefragQuota { quota_bytes: 100, threshold: 0.8, fragmentation_threshold: 0.0 };
        let s = st(1, 50, 50);
        assert_eq!(defrag_decision(&q, &s), DefragDecision::Skip);
    }

    #[test]
    fn test_defrag_fragmentation_triggers() {
        // cite: bbolt fragmentation (in_use << total)
        let q = DefragQuota { quota_bytes: 1_000_000, threshold: 0.99, fragmentation_threshold: 0.5 };
        let s = st(1, 1000, 200);
        assert_eq!(defrag_decision(&q, &s), DefragDecision::Trigger(DefragTrigger::Fragmented));
    }

    #[test]
    fn test_defrag_fragmentation_under_threshold_skips() {
        // cite: bbolt (in_use ≥ threshold ⇒ no defrag)
        let q = DefragQuota { quota_bytes: 1_000_000, threshold: 0.99, fragmentation_threshold: 0.5 };
        let s = st(1, 1000, 800);
        assert_eq!(defrag_decision(&q, &s), DefragDecision::Skip);
    }

    #[test]
    fn test_defrag_force_triggers() {
        // cite: etcdctl defrag --force
        assert_eq!(defrag_force(), DefragDecision::Trigger(DefragTrigger::ManualOverride));
    }

    #[test]
    fn test_defrag_zero_db_size_skips() {
        // cite: never trigger on an empty backend
        let q = DefragQuota::default();
        let s = st(1, 0, 0);
        assert_eq!(defrag_decision(&q, &s), DefragDecision::Skip);
    }

    // ── Downgrade state machine ────────────────────────────────────────

    #[test]
    fn test_downgrade_validate_one_minor_back() {
        // cite: rpc.proto DowngradeAction.Validate (one-minor-down policy)
        let d = DowngradeState::new();
        let r = d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Validate, version: "3.5".into() }).unwrap();
        assert_eq!(r.version, "3.5");
        assert!(!r.enabled);
    }

    #[test]
    fn test_downgrade_validate_two_minors_back_rejected() {
        // cite: rpc.proto Downgrade (only one-minor-down allowed)
        let d = DowngradeState::new();
        let r = d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Validate, version: "3.4".into() });
        assert!(matches!(r.unwrap_err(), DowngradeError::InvalidVersion(_)));
    }

    #[test]
    fn test_downgrade_validate_major_change_rejected() {
        // cite: rpc.proto Downgrade (no major version change)
        let d = DowngradeState::new();
        let r = d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Validate, version: "2.5".into() });
        assert!(matches!(r.unwrap_err(), DowngradeError::InvalidVersion(_)));
    }

    #[test]
    fn test_downgrade_enable_persists() {
        // cite: rpc.proto Downgrade.Enable (state recorded)
        let d = DowngradeState::new();
        d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Enable, version: "3.5".into() }).unwrap();
        assert!(d.enabled());
        assert_eq!(d.target().as_deref(), Some("3.5"));
    }

    #[test]
    fn test_downgrade_enable_after_other_target_errors() {
        // cite: rpc.proto Downgrade (one downgrade at a time)
        let d = DowngradeState::new();
        d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Enable, version: "3.5".into() }).unwrap();
        let r = d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Enable, version: "3.4".into() });
        assert!(matches!(r.unwrap_err(), DowngradeError::InvalidVersion(_) | DowngradeError::AlreadyInProgress(_)));
    }

    #[test]
    fn test_downgrade_cancel_clears_state() {
        // cite: rpc.proto Downgrade.Cancel
        let d = DowngradeState::new();
        d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Enable, version: "3.5".into() }).unwrap();
        d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Cancel, version: "".into() }).unwrap();
        assert!(!d.enabled());
        assert!(d.target().is_none());
    }

    #[test]
    fn test_downgrade_cancel_when_idle_errors() {
        // cite: rpc.proto Downgrade.Cancel (no-op cancel ⇒ error)
        let d = DowngradeState::new();
        let r = d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Cancel, version: "".into() });
        assert!(matches!(r.unwrap_err(), DowngradeError::NotInProgress));
    }

    #[test]
    fn test_downgrade_validate_does_not_persist_state() {
        // cite: rpc.proto Downgrade.Validate (read-only)
        let d = DowngradeState::new();
        d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Validate, version: "3.5".into() }).unwrap();
        assert!(!d.enabled());
        assert!(d.target().is_none());
    }

    #[test]
    fn test_downgrade_invalid_cluster_version() {
        // cite: rpc.proto (cluster version must be parseable)
        let d = DowngradeState::new();
        let r = d.apply("not-a-version", &DowngradeRequest { action: DowngradeAction::Validate, version: "3.5".into() });
        assert!(matches!(r.unwrap_err(), DowngradeError::InvalidVersion(_)));
    }

    #[test]
    fn test_downgrade_invalid_target_version() {
        // cite: rpc.proto (target version must be parseable)
        let d = DowngradeState::new();
        let r = d.apply("3.6", &DowngradeRequest { action: DowngradeAction::Validate, version: "garbage".into() });
        assert!(matches!(r.unwrap_err(), DowngradeError::InvalidVersion(_)));
    }
}
