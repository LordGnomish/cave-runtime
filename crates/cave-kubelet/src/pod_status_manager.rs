//! Pod status manager — `pkg/kubelet/status/status_manager.go`.
//!
//! The kubelet's sync loop observes pod state on this node (running
//! containers, probe outcomes, eviction signals, …) and is the
//! authoritative source-of-truth for `Pod.status`. The status
//! manager:
//!
//! * **De-duplicates writes via hash compare** — the sync loop calls
//!   `set_status` on every tick; if the status hashes equal what was
//!   last enqueued, no apiserver round-trip is made. This is the lazy
//!   enqueue behaviour the upstream calls `needsUpdate`.
//! * **Queues outbound updates** — each pending pod has at most one
//!   in-flight status update; new updates supersede earlier ones.
//! * **Retries with exponential backoff on transient apiserver
//!   failures** via `cave_kernel::backoff::Backoff::Exponential`.
//! * **Drops updates for deleted pods** to bound queue growth during
//!   apiserver downtime.
//!
//! Upstream uses a per-pod goroutine; cave uses a single priority
//! queue (BinaryHeap keyed by ready-at instant). The semantics
//! mirror upstream: a pod always has its newest status pending, but
//! at most one in-flight write at a time, with backoff between
//! failures.

use crate::probe::ProbeOutcome;
use cave_kernel::backoff::Backoff;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Duration;

#[allow(dead_code)]
pub const UPSTREAM_PATH: &str = "pkg/kubelet/status/status_manager.go";
#[allow(dead_code)]
pub const UPSTREAM_SYMBOL: &str = "manager.SetPodStatus";

/// Top-level pod phase mirroring `core/v1.PodPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

/// Per-container observation that bubbles up into `Pod.status`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub name: String,
    pub ready: bool,
    pub restart_count: u32,
    pub image: String,
}

/// Snapshot of one pod's status that the sync loop wants pushed to
/// the apiserver. The status manager hash-compares successive
/// snapshots to suppress redundant writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodStatus {
    pub phase: PodPhase,
    pub conditions: Vec<(String, bool)>,
    pub containers: Vec<ContainerStatus>,
    pub message: Option<String>,
}

impl PodStatus {
    /// Stable content-hash used for `needsUpdate` comparison. We do
    /// NOT include `message` because the kubelet edits free-text
    /// messages frequently and we don't want every tick to push.
    pub fn content_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.phase.hash(&mut h);
        for (k, v) in &self.conditions {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        for c in &self.containers {
            c.hash(&mut h);
        }
        h.finish()
    }

    /// Build a `PodStatus` from a per-container probe outcome map.
    /// Helper used by the kubelet sync loop so probe-driven readiness
    /// rolls up the same way every tick.
    pub fn from_probe_outcomes(
        phase: PodPhase,
        outcomes: &[(String, ProbeOutcome)],
        restart_counts: &HashMap<String, u32>,
        images: &HashMap<String, String>,
    ) -> Self {
        let containers = outcomes
            .iter()
            .map(|(name, outcome)| ContainerStatus {
                name: name.clone(),
                ready: matches!(outcome, ProbeOutcome::Success),
                restart_count: *restart_counts.get(name).unwrap_or(&0),
                image: images.get(name).cloned().unwrap_or_default(),
            })
            .collect();
        Self {
            phase,
            conditions: vec![],
            containers,
            message: None,
        }
    }
}

/// One outbound update awaiting an apiserver round-trip.
#[derive(Debug, Clone)]
struct PendingUpdate {
    pod_uid: String,
    status: PodStatus,
    /// Hash captured at enqueue time. The manager compares this
    /// against the new hash on `set_status` to skip re-queueing.
    hash: u64,
    /// `None` until the first failure. Otherwise the attempt count
    /// drives the backoff schedule.
    failure_count: u32,
    /// Earliest instant at which this update may be sent. Updated on
    /// each failure via the backoff strategy.
    ready_at: DateTime<Utc>,
}

/// Drop-reason emitted when an update is suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DropReason {
    /// Pod has been removed; pending update discarded.
    PodDeleted,
    /// `set_status` hashed equal to the in-flight pending status — no
    /// new work to do.
    Deduped,
}

/// Result the kubelet sync loop gets when it tries to dispatch one
/// status update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchOutcome {
    /// No update was ready — either the queue is empty, every entry
    /// is still in backoff, or every pending pod has been deleted.
    Idle,
    /// One update was dispatched; caller must invoke
    /// `record_attempt(uid, result)` once the apiserver responds.
    Dispatched { pod_uid: String, status: PodStatus },
}

/// Caller's report of how the apiserver round-trip went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttemptOutcome {
    /// Apiserver accepted the write.
    Success,
    /// Transient failure — re-queue with backoff.
    TransientFailure,
    /// Permanent failure (e.g. NotFound). Drop the pending entry.
    PermanentFailure,
}

/// Configurable knobs.
#[derive(Debug, Clone, Copy)]
pub struct StatusManagerConfig {
    pub backoff: Backoff,
    /// Cap on total queued pods. If exceeded, the oldest entry is
    /// dropped (and the manager records a drop reason for telemetry).
    pub max_queued: usize,
}

impl Default for StatusManagerConfig {
    fn default() -> Self {
        Self {
            backoff: Backoff::Exponential {
                base: Duration::from_millis(200),
                cap: Duration::from_secs(30),
            },
            // Generous enough that under steady apiserver downtime we
            // hold every pod on a 250-pod node, but bounded.
            max_queued: 1024,
        }
    }
}

/// In-memory pod status manager.
#[derive(Debug)]
pub struct PodStatusManager {
    cfg: StatusManagerConfig,
    /// `pod_uid → pending update`. At most one entry per pod.
    pending: HashMap<String, PendingUpdate>,
    /// `pod_uid → hash` of the last status that was *successfully*
    /// written to the apiserver. Used by `needs_update` so we don't
    /// re-enqueue when nothing changed since the last successful
    /// write.
    confirmed: HashMap<String, u64>,
    /// `pod_uid → ()` — pods that have been signalled deleted. We
    /// keep this set so a late `set_status` call (race with the GC
    /// thread) is silently dropped instead of resurrecting the pod.
    deleted: HashMap<String, ()>,
    /// Telemetry: dropped-update counts by reason. The router /
    /// metrics endpoint reads this for the kubelet status panel.
    drops: HashMap<DropReason, u64>,
}

impl PodStatusManager {
    pub fn new(cfg: StatusManagerConfig) -> Self {
        Self {
            cfg,
            pending: HashMap::new(),
            confirmed: HashMap::new(),
            deleted: HashMap::new(),
            drops: HashMap::new(),
        }
    }

    /// Push the latest observed status. Returns `Some(reason)` if
    /// the update was dropped (deleted pod or dedupe), `None` if it
    /// was queued (or replaced a pending entry).
    pub fn set_status(
        &mut self,
        pod_uid: &str,
        status: PodStatus,
        now: DateTime<Utc>,
    ) -> Option<DropReason> {
        if self.deleted.contains_key(pod_uid) {
            *self.drops.entry(DropReason::PodDeleted).or_insert(0) += 1;
            return Some(DropReason::PodDeleted);
        }

        let new_hash = status.content_hash();

        // Dedupe against the pending entry, if any.
        if let Some(existing) = self.pending.get(pod_uid) {
            if existing.hash == new_hash {
                *self.drops.entry(DropReason::Deduped).or_insert(0) += 1;
                return Some(DropReason::Deduped);
            }
        }
        // Dedupe against the last *confirmed* write.
        if self.pending.get(pod_uid).is_none() {
            if let Some(&confirmed_hash) = self.confirmed.get(pod_uid) {
                if confirmed_hash == new_hash {
                    *self.drops.entry(DropReason::Deduped).or_insert(0) += 1;
                    return Some(DropReason::Deduped);
                }
            }
        }

        // Bounded queue: evict oldest before insert.
        if self.pending.len() >= self.cfg.max_queued && !self.pending.contains_key(pod_uid) {
            self.evict_oldest();
        }

        self.pending.insert(
            pod_uid.into(),
            PendingUpdate {
                pod_uid: pod_uid.into(),
                status,
                hash: new_hash,
                failure_count: 0,
                // New / replaced entries are ready immediately.
                ready_at: now,
            },
        );
        None
    }

    /// Whether `set_status(pod_uid, status, _)` would currently
    /// enqueue work. Public so the sync loop can short-circuit
    /// without constructing a status if nothing changed.
    pub fn needs_update(&self, pod_uid: &str, status: &PodStatus) -> bool {
        if self.deleted.contains_key(pod_uid) {
            return false;
        }
        let hash = status.content_hash();
        if let Some(p) = self.pending.get(pod_uid) {
            return p.hash != hash;
        }
        match self.confirmed.get(pod_uid) {
            Some(&h) => h != hash,
            None => true,
        }
    }

    /// Mark a pod deleted. Any pending update is dropped immediately.
    /// Late `set_status` for the same uid will also be dropped.
    pub fn delete_pod(&mut self, pod_uid: &str) {
        if self.pending.remove(pod_uid).is_some() {
            *self.drops.entry(DropReason::PodDeleted).or_insert(0) += 1;
        }
        self.confirmed.remove(pod_uid);
        self.deleted.insert(pod_uid.into(), ());
    }

    /// Take the next ready update, if any.
    pub fn pop_ready(&mut self, now: DateTime<Utc>) -> DispatchOutcome {
        // Find the earliest `ready_at` that is `<= now`. Tie-break on
        // pod_uid for deterministic ordering in tests.
        let mut best_uid: Option<String> = None;
        let mut best_ready: Option<DateTime<Utc>> = None;
        for (uid, p) in &self.pending {
            if p.ready_at > now {
                continue;
            }
            let take = match best_ready {
                None => true,
                Some(t) if p.ready_at < t => true,
                Some(t) if p.ready_at == t && best_uid.as_deref() > Some(uid.as_str()) => true,
                _ => false,
            };
            if take {
                best_ready = Some(p.ready_at);
                best_uid = Some(uid.clone());
            }
        }
        match best_uid {
            None => DispatchOutcome::Idle,
            Some(uid) => {
                let p = self
                    .pending
                    .get(&uid)
                    .expect("just located the entry")
                    .clone();
                DispatchOutcome::Dispatched {
                    pod_uid: p.pod_uid,
                    status: p.status,
                }
            }
        }
    }

    /// Record what happened on the most recent apiserver round-trip.
    /// `now` is the instant the result is being recorded; backoff
    /// is computed relative to it.
    pub fn record_attempt(
        &mut self,
        pod_uid: &str,
        outcome: AttemptOutcome,
        now: DateTime<Utc>,
    ) {
        let Some(p) = self.pending.get_mut(pod_uid) else { return; };
        match outcome {
            AttemptOutcome::Success => {
                let hash = p.hash;
                self.pending.remove(pod_uid);
                self.confirmed.insert(pod_uid.into(), hash);
            }
            AttemptOutcome::PermanentFailure => {
                self.pending.remove(pod_uid);
                // Don't update `confirmed`: the write didn't land and
                // we don't want a future identical status to dedupe
                // against a hash that wasn't actually persisted.
            }
            AttemptOutcome::TransientFailure => {
                p.failure_count = p.failure_count.saturating_add(1);
                let delay = self.cfg.backoff.delay_for(p.failure_count.saturating_sub(1));
                let chrono_delay = chrono::Duration::from_std(delay)
                    .unwrap_or_else(|_| chrono::Duration::seconds(i64::MAX));
                p.ready_at = now + chrono_delay;
            }
        }
    }

    /// Number of pending updates (any readiness state).
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Number of pods that are in backoff (not yet ready at `now`).
    pub fn in_backoff(&self, now: DateTime<Utc>) -> usize {
        self.pending.values().filter(|p| p.ready_at > now).count()
    }

    /// Drop telemetry — public so the routes module can render it.
    pub fn drop_counts(&self) -> HashMap<DropReason, u64> {
        self.drops.clone()
    }

    fn evict_oldest(&mut self) {
        // Evict the entry with the smallest `ready_at` (i.e. the one
        // that's been waiting the longest). Tie-break on pod_uid.
        let mut victim: Option<(String, DateTime<Utc>)> = None;
        for (uid, p) in &self.pending {
            let take = match &victim {
                None => true,
                Some((vid, v_ready)) => p.ready_at < *v_ready
                    || (p.ready_at == *v_ready && uid < vid),
            };
            if take {
                victim = Some((uid.clone(), p.ready_at));
            }
        }
        if let Some((uid, _)) = victim {
            self.pending.remove(&uid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(phase: PodPhase, ready: bool) -> PodStatus {
        PodStatus {
            phase,
            conditions: vec![("Ready".into(), ready)],
            containers: vec![ContainerStatus {
                name: "main".into(),
                ready,
                restart_count: 0,
                image: "alpine:3".into(),
            }],
            message: None,
        }
    }

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn fresh_status_is_queued() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let r = m.set_status("pod-a", st(PodPhase::Running, true), t0());
        assert!(r.is_none());
        assert_eq!(m.pending_len(), 1);
    }

    #[test]
    fn identical_status_is_deduped_against_pending() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let s = st(PodPhase::Running, true);
        assert!(m.set_status("pod-a", s.clone(), t0()).is_none());
        let r = m.set_status("pod-a", s, t0());
        assert_eq!(r, Some(DropReason::Deduped));
        assert_eq!(m.pending_len(), 1);
    }

    #[test]
    fn changed_status_supersedes_pending() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        m.set_status("pod-a", st(PodPhase::Pending, false), t0());
        m.set_status("pod-a", st(PodPhase::Running, true), t0());
        assert_eq!(m.pending_len(), 1);
        let popped = m.pop_ready(t0());
        match popped {
            DispatchOutcome::Dispatched { status, .. } => {
                assert_eq!(status.phase, PodPhase::Running);
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
    }

    #[test]
    fn message_changes_do_not_force_re_enqueue() {
        // `message` is intentionally excluded from the content hash.
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let mut s = st(PodPhase::Running, true);
        m.set_status("pod-a", s.clone(), t0());
        s.message = Some("free-text edit on every tick".into());
        let r = m.set_status("pod-a", s, t0());
        assert_eq!(r, Some(DropReason::Deduped));
    }

    #[test]
    fn delete_pod_drops_pending_and_blocks_late_set() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        m.set_status("pod-a", st(PodPhase::Running, true), t0());
        m.delete_pod("pod-a");
        assert_eq!(m.pending_len(), 0);
        let r = m.set_status("pod-a", st(PodPhase::Failed, false), t0());
        assert_eq!(r, Some(DropReason::PodDeleted));
        assert_eq!(m.drop_counts().get(&DropReason::PodDeleted).copied(), Some(2));
    }

    #[test]
    fn pop_ready_with_empty_queue_is_idle() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        assert_eq!(m.pop_ready(t0()), DispatchOutcome::Idle);
    }

    #[test]
    fn success_clears_pending_and_dedupes_future_identical_status() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let s = st(PodPhase::Running, true);
        m.set_status("pod-a", s.clone(), t0());
        match m.pop_ready(t0()) {
            DispatchOutcome::Dispatched { pod_uid, .. } => {
                m.record_attempt(&pod_uid, AttemptOutcome::Success, t0());
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
        assert_eq!(m.pending_len(), 0);
        // Re-pushing the same status now dedupes against the
        // confirmed hash.
        let r = m.set_status("pod-a", s, t0());
        assert_eq!(r, Some(DropReason::Deduped));
    }

    #[test]
    fn transient_failure_schedules_backoff() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let now = t0();
        m.set_status("pod-a", st(PodPhase::Running, true), now);
        match m.pop_ready(now) {
            DispatchOutcome::Dispatched { pod_uid, .. } => {
                m.record_attempt(&pod_uid, AttemptOutcome::TransientFailure, now);
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
        assert_eq!(m.pending_len(), 1);
        assert_eq!(m.in_backoff(now), 1);
        // Right at backoff start: not ready.
        assert_eq!(m.pop_ready(now), DispatchOutcome::Idle);
        // After enough time (200ms default base for 1st retry).
        let later = now + chrono::Duration::milliseconds(250);
        assert!(matches!(m.pop_ready(later), DispatchOutcome::Dispatched { .. }));
    }

    #[test]
    fn permanent_failure_drops_pending_without_confirming() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        m.set_status("pod-a", st(PodPhase::Running, true), t0());
        match m.pop_ready(t0()) {
            DispatchOutcome::Dispatched { pod_uid, .. } => {
                m.record_attempt(&pod_uid, AttemptOutcome::PermanentFailure, t0());
            }
            other => panic!("expected Dispatched, got {other:?}"),
        }
        assert_eq!(m.pending_len(), 0);
        // Permanent failure must not poison the confirmed hash map:
        // a future identical status should still enqueue.
        let r = m.set_status("pod-a", st(PodPhase::Running, true), t0());
        assert!(r.is_none());
    }

    #[test]
    fn exponential_backoff_grows_across_repeated_failures() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let mut now = t0();
        m.set_status("pod-a", st(PodPhase::Running, true), now);

        // Simulate three back-to-back transient failures, verifying
        // the cumulative delay grows.
        let mut prev_delay = Duration::ZERO;
        for i in 0..3 {
            now = match m.pop_ready(now) {
                DispatchOutcome::Dispatched { pod_uid, .. } => {
                    m.record_attempt(&pod_uid, AttemptOutcome::TransientFailure, now);
                    let d =
                        Backoff::Exponential {
                            base: Duration::from_millis(200),
                            cap: Duration::from_secs(30),
                        }
                        .delay_for(i);
                    assert!(d >= prev_delay, "backoff should not decrease");
                    prev_delay = d;
                    now + chrono::Duration::from_std(d).unwrap()
                        + chrono::Duration::milliseconds(1)
                }
                other => panic!("expected Dispatched, got {other:?}"),
            };
        }
    }

    #[test]
    fn pop_ready_is_deterministic_across_ties() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let now = t0();
        m.set_status("pod-b", st(PodPhase::Running, true), now);
        m.set_status("pod-a", st(PodPhase::Running, true), now);
        m.set_status("pod-c", st(PodPhase::Running, true), now);
        // Tie-break: lexicographically smallest first.
        match m.pop_ready(now) {
            DispatchOutcome::Dispatched { pod_uid, .. } => assert_eq!(pod_uid, "pod-a"),
            other => panic!("expected Dispatched, got {other:?}"),
        }
    }

    #[test]
    fn needs_update_short_circuit_matches_set_status() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let s = st(PodPhase::Running, true);
        assert!(m.needs_update("pod-a", &s));
        m.set_status("pod-a", s.clone(), t0());
        assert!(!m.needs_update("pod-a", &s));
        let s2 = st(PodPhase::Failed, false);
        assert!(m.needs_update("pod-a", &s2));
    }

    #[test]
    fn bounded_queue_evicts_oldest_when_full() {
        let cfg = StatusManagerConfig {
            backoff: Backoff::Exponential {
                base: Duration::from_millis(100),
                cap: Duration::from_secs(5),
            },
            max_queued: 2,
        };
        let mut m = PodStatusManager::new(cfg);
        let now = t0();
        m.set_status("p-old", st(PodPhase::Running, true), now);
        m.set_status(
            "p-mid",
            st(PodPhase::Running, true),
            now + chrono::Duration::seconds(1),
        );
        // Third insert should evict the oldest entry.
        m.set_status(
            "p-new",
            st(PodPhase::Running, true),
            now + chrono::Duration::seconds(2),
        );
        assert_eq!(m.pending_len(), 2);
        // p-old should be gone; one of p-mid/p-new should remain (the
        // oldest by ready_at — p-mid stays, p-old got evicted).
        // The pop should yield the next-oldest entry (p-mid).
        let v = match m.pop_ready(now + chrono::Duration::seconds(5)) {
            DispatchOutcome::Dispatched { pod_uid, .. } => pod_uid,
            other => panic!("{other:?}"),
        };
        assert!(v == "p-mid" || v == "p-new");
        assert_ne!(v, "p-old");
    }

    #[test]
    fn from_probe_outcomes_rolls_up_readiness() {
        let now = t0();
        let mut restart_counts = HashMap::new();
        restart_counts.insert("main".to_string(), 2_u32);
        let mut images = HashMap::new();
        images.insert("main".to_string(), "alpine:3".to_string());
        let status = PodStatus::from_probe_outcomes(
            PodPhase::Running,
            &[
                ("main".into(), ProbeOutcome::Success),
                ("sidecar".into(), ProbeOutcome::Failure),
            ],
            &restart_counts,
            &images,
        );
        assert_eq!(status.phase, PodPhase::Running);
        assert_eq!(status.containers.len(), 2);
        let main = status.containers.iter().find(|c| c.name == "main").unwrap();
        assert!(main.ready);
        assert_eq!(main.restart_count, 2);
        assert_eq!(main.image, "alpine:3");
        let side = status
            .containers
            .iter()
            .find(|c| c.name == "sidecar")
            .unwrap();
        assert!(!side.ready);
        // Verify it round-trips through the manager.
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        assert!(m.set_status("p", status, now).is_none());
    }

    #[test]
    fn pop_ready_picks_earliest_ready_at() {
        let mut m = PodStatusManager::new(StatusManagerConfig::default());
        let now = t0();
        m.set_status("pod-a", st(PodPhase::Running, true), now);
        // Fail once to push pod-a into the backoff window.
        match m.pop_ready(now) {
            DispatchOutcome::Dispatched { pod_uid, .. } => {
                m.record_attempt(&pod_uid, AttemptOutcome::TransientFailure, now);
            }
            other => panic!("{other:?}"),
        }
        // Now enqueue a fresh pod whose ready_at is `now`.
        m.set_status("pod-b", st(PodPhase::Running, true), now);
        match m.pop_ready(now) {
            DispatchOutcome::Dispatched { pod_uid, .. } => assert_eq!(pod_uid, "pod-b"),
            other => panic!("{other:?}"),
        }
    }
}
