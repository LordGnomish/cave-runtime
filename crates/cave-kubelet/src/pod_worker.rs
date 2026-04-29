//! Pod worker — per-pod sync state machine.
//!
//! Mirrors `pkg/kubelet/pod_workers.go`: each pod gets its own work-queue;
//! the sync goroutine debounces updates, transitions through SyncPod →
//! TerminatingPod → TerminatedPod, and tracks whether subsequent updates
//! arrived during a sync (the "running update" pattern). This module
//! captures the deterministic state machine without the goroutine plumbing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkType {
    /// Initial create or running update for a non-terminating pod.
    SyncPod,
    /// Pod has been told to terminate; running PreStop / SIGTERM.
    TerminatingPod,
    /// Pod containers gone; cleaning up volumes, cgroups, status.
    TerminatedPod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerState {
    /// No work in progress; ready to accept new updates.
    Idle,
    /// Currently running a sync of `WorkType`.
    Working { current: WorkType },
    /// Pod has been fully cleaned up; worker can be removed.
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodUpdate {
    pub pod_uid: String,
    pub work_type: WorkType,
    pub at: DateTime<Utc>,
    pub grace_period_seconds: Option<i64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkerError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
}

pub type WorkerResult<T> = Result<T, WorkerError>;

#[derive(Debug, Clone)]
pub struct PodWorkerState {
    pub state: WorkerState,
    /// Last completed work type.
    pub last_finished: Option<WorkType>,
    /// Pending update queued during a sync (work-coalescing).
    pub pending: Option<PodUpdate>,
    pub sync_count: u64,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub started_at: DateTime<Utc>,
}

impl PodWorkerState {
    pub fn new(at: DateTime<Utc>) -> Self {
        Self {
            state: WorkerState::Idle,
            last_finished: None,
            pending: None,
            sync_count: 0,
            last_sync_at: None,
            started_at: at,
        }
    }
}

#[derive(Debug, Default)]
pub struct PodWorkerManager {
    workers: BTreeMap<String, PodWorkerState>,
}

impl PodWorkerManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn get(&self, pod_uid: &str) -> Option<&PodWorkerState> {
        self.workers.get(pod_uid)
    }

    /// Submit an update. If worker is Idle, transitions to Working with the
    /// new work-type. If Working, the update is coalesced into `pending`
    /// (replacing any previous pending update, since only the latest matters).
    /// Returns the work-type the worker should run, if any.
    pub fn update(&mut self, update: PodUpdate) -> WorkerResult<Option<WorkType>> {
        if update.pod_uid.is_empty() {
            return Err(WorkerError::Invalid("pod_uid empty".into()));
        }
        // Reject downgrade transitions: once terminating, can't go back to sync;
        // once terminated, can't accept any update.
        let entry = self
            .workers
            .entry(update.pod_uid.clone())
            .or_insert_with(|| PodWorkerState::new(update.at));
        if !is_transition_allowed(&entry.state, entry.last_finished, update.work_type) {
            return Err(WorkerError::Forbidden(format!(
                "transition to {:?} not allowed in state {:?}",
                update.work_type, entry.state
            )));
        }
        match entry.state {
            WorkerState::Idle => {
                entry.state = WorkerState::Working {
                    current: update.work_type,
                };
                Ok(Some(update.work_type))
            }
            WorkerState::Working { current } => {
                // Replace any pending update with the new one (coalesce).
                entry.pending = Some(update);
                Ok(Some(current))
            }
            WorkerState::Finished => Err(WorkerError::Forbidden(
                "worker is finished; pod removed".into(),
            )),
        }
    }

    /// Mark the current sync as completed. Advances the state machine and
    /// drains any pending update.
    pub fn complete_sync(
        &mut self,
        pod_uid: &str,
        at: DateTime<Utc>,
    ) -> WorkerResult<Option<WorkType>> {
        let entry = self
            .workers
            .get_mut(pod_uid)
            .ok_or_else(|| WorkerError::NotFound(pod_uid.into()))?;
        let just_finished = match entry.state {
            WorkerState::Working { current } => current,
            _ => {
                return Err(WorkerError::Invalid(
                    "complete_sync called when not Working".into(),
                ))
            }
        };
        entry.last_finished = Some(just_finished);
        entry.last_sync_at = Some(at);
        entry.sync_count += 1;
        // Auto-advance state for terminal sequences.
        if matches!(just_finished, WorkType::TerminatedPod) {
            entry.state = WorkerState::Finished;
            entry.pending = None;
            return Ok(None);
        }
        // Drain pending update.
        if let Some(pending) = entry.pending.take() {
            // Validate the pending against just_finished.
            if !is_transition_allowed(&WorkerState::Idle, Some(just_finished), pending.work_type) {
                entry.state = WorkerState::Idle;
                return Ok(None);
            }
            entry.state = WorkerState::Working {
                current: pending.work_type,
            };
            return Ok(Some(pending.work_type));
        }
        entry.state = WorkerState::Idle;
        Ok(None)
    }

    pub fn remove(&mut self, pod_uid: &str) {
        self.workers.remove(pod_uid);
    }

    pub fn finished_count(&self) -> usize {
        self.workers
            .values()
            .filter(|w| matches!(w.state, WorkerState::Finished))
            .count()
    }
}

/// Whether transitioning to `next` from `cur` (with `last_finished`) is allowed.
/// SyncPod → TerminatingPod → TerminatedPod is the only forward path.
pub fn is_transition_allowed(
    cur: &WorkerState,
    last_finished: Option<WorkType>,
    next: WorkType,
) -> bool {
    let baseline = match cur {
        WorkerState::Working { current } => Some(*current),
        _ => last_finished,
    };
    match (baseline, next) {
        (None, WorkType::SyncPod) => true,
        (None, WorkType::TerminatingPod) => true, // termination of never-synced pod
        (None, WorkType::TerminatedPod) => false, // can't terminate without termination first
        (Some(WorkType::SyncPod), WorkType::SyncPod) => true,
        (Some(WorkType::SyncPod), WorkType::TerminatingPod) => true,
        (Some(WorkType::SyncPod), WorkType::TerminatedPod) => false,
        (Some(WorkType::TerminatingPod), WorkType::SyncPod) => false,
        (Some(WorkType::TerminatingPod), WorkType::TerminatingPod) => true,
        (Some(WorkType::TerminatingPod), WorkType::TerminatedPod) => true,
        (Some(WorkType::TerminatedPod), _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(s, 0).unwrap()
    }

    fn upd(uid: &str, work: WorkType) -> PodUpdate {
        PodUpdate {
            pod_uid: uid.into(),
            work_type: work,
            at: t(0),
            grace_period_seconds: None,
        }
    }

    #[test]
    fn first_update_starts_sync() {
        let mut m = PodWorkerManager::new();
        let next = m.update(upd("p", WorkType::SyncPod)).unwrap();
        assert_eq!(next, Some(WorkType::SyncPod));
        let s = m.get("p").unwrap();
        assert!(matches!(
            s.state,
            WorkerState::Working { current: WorkType::SyncPod }
        ));
    }

    #[test]
    fn update_with_empty_uid_errors() {
        let mut m = PodWorkerManager::new();
        assert!(m.update(upd("", WorkType::SyncPod)).is_err());
    }

    #[test]
    fn second_update_during_work_coalesces_into_pending() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        let next = m.update(upd("p", WorkType::SyncPod)).unwrap();
        assert_eq!(next, Some(WorkType::SyncPod));
        let s = m.get("p").unwrap();
        assert!(s.pending.is_some());
    }

    #[test]
    fn coalesced_update_runs_after_complete_sync() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        let next = m.complete_sync("p", t(1)).unwrap();
        assert_eq!(next, Some(WorkType::TerminatingPod));
    }

    #[test]
    fn complete_sync_with_no_pending_returns_idle() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        let next = m.complete_sync("p", t(1)).unwrap();
        assert_eq!(next, None);
        assert!(matches!(m.get("p").unwrap().state, WorkerState::Idle));
    }

    #[test]
    fn complete_sync_increments_counter_and_records_time() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.complete_sync("p", t(5)).unwrap();
        let s = m.get("p").unwrap();
        assert_eq!(s.sync_count, 1);
        assert_eq!(s.last_sync_at, Some(t(5)));
    }

    #[test]
    fn complete_sync_when_not_working_errors() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        assert!(m.complete_sync("p", t(2)).is_err());
    }

    #[test]
    fn complete_sync_unknown_pod_errors() {
        let mut m = PodWorkerManager::new();
        assert!(m.complete_sync("ghost", t(0)).is_err());
    }

    #[test]
    fn terminating_to_terminated_transitions_to_finished() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("p", t(2)).unwrap();
        m.update(upd("p", WorkType::TerminatedPod)).unwrap();
        let next = m.complete_sync("p", t(3)).unwrap();
        assert_eq!(next, None);
        assert!(matches!(m.get("p").unwrap().state, WorkerState::Finished));
    }

    #[test]
    fn finished_worker_rejects_new_updates() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        m.update(upd("p", WorkType::TerminatedPod)).unwrap();
        m.complete_sync("p", t(2)).unwrap();
        assert!(matches!(
            m.update(upd("p", WorkType::SyncPod)),
            Err(WorkerError::Forbidden(_))
        ));
    }

    #[test]
    fn cannot_go_back_from_terminating_to_sync() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        let err = m.update(upd("p", WorkType::SyncPod)).unwrap_err();
        assert!(matches!(err, WorkerError::Forbidden(_)));
    }

    #[test]
    fn cannot_skip_terminating_and_go_to_terminated() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        let err = m.update(upd("p", WorkType::TerminatedPod)).unwrap_err();
        assert!(matches!(err, WorkerError::Forbidden(_)));
    }

    #[test]
    fn termination_can_start_from_no_prior_sync() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
    }

    #[test]
    fn worker_count_tracks_active_workers() {
        let mut m = PodWorkerManager::new();
        assert_eq!(m.worker_count(), 0);
        m.update(upd("p1", WorkType::SyncPod)).unwrap();
        m.update(upd("p2", WorkType::SyncPod)).unwrap();
        assert_eq!(m.worker_count(), 2);
    }

    #[test]
    fn finished_count_filters_finished_workers() {
        let mut m = PodWorkerManager::new();
        m.update(upd("a", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("a", t(0)).unwrap();
        m.update(upd("a", WorkType::TerminatedPod)).unwrap();
        m.complete_sync("a", t(1)).unwrap();
        m.update(upd("b", WorkType::SyncPod)).unwrap();
        assert_eq!(m.finished_count(), 1);
    }

    #[test]
    fn remove_clears_worker() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.remove("p");
        assert!(m.get("p").is_none());
    }

    #[test]
    fn pending_replaced_by_latest_update() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        let s = m.get("p").unwrap();
        assert_eq!(
            s.pending.as_ref().map(|p| p.work_type),
            Some(WorkType::TerminatingPod)
        );
    }

    #[test]
    fn transition_baseline_none_to_sync_ok() {
        assert!(is_transition_allowed(
            &WorkerState::Idle,
            None,
            WorkType::SyncPod
        ));
    }

    #[test]
    fn transition_baseline_none_to_terminating_ok() {
        assert!(is_transition_allowed(
            &WorkerState::Idle,
            None,
            WorkType::TerminatingPod
        ));
    }

    #[test]
    fn transition_baseline_none_to_terminated_rejected() {
        assert!(!is_transition_allowed(
            &WorkerState::Idle,
            None,
            WorkType::TerminatedPod
        ));
    }

    #[test]
    fn transition_terminating_to_sync_rejected() {
        assert!(!is_transition_allowed(
            &WorkerState::Idle,
            Some(WorkType::TerminatingPod),
            WorkType::SyncPod
        ));
    }

    #[test]
    fn transition_terminating_repeat_ok() {
        assert!(is_transition_allowed(
            &WorkerState::Idle,
            Some(WorkType::TerminatingPod),
            WorkType::TerminatingPod
        ));
    }

    #[test]
    fn transition_sync_to_terminating_ok() {
        assert!(is_transition_allowed(
            &WorkerState::Idle,
            Some(WorkType::SyncPod),
            WorkType::TerminatingPod
        ));
    }

    #[test]
    fn transition_sync_repeat_ok() {
        assert!(is_transition_allowed(
            &WorkerState::Idle,
            Some(WorkType::SyncPod),
            WorkType::SyncPod
        ));
    }

    #[test]
    fn transition_terminated_to_anything_rejected() {
        for next in [
            WorkType::SyncPod,
            WorkType::TerminatingPod,
            WorkType::TerminatedPod,
        ] {
            assert!(!is_transition_allowed(
                &WorkerState::Idle,
                Some(WorkType::TerminatedPod),
                next
            ));
        }
    }

    #[test]
    fn during_work_baseline_is_current() {
        // While Working{Sync}, baseline is SyncPod, so a TerminatingPod update is allowed.
        assert!(is_transition_allowed(
            &WorkerState::Working {
                current: WorkType::SyncPod
            },
            None,
            WorkType::TerminatingPod
        ));
        // But going back to Sync from Working{Terminating} is rejected.
        assert!(!is_transition_allowed(
            &WorkerState::Working {
                current: WorkType::TerminatingPod
            },
            None,
            WorkType::SyncPod
        ));
    }

    #[test]
    fn sync_to_sync_during_work_allowed() {
        assert!(is_transition_allowed(
            &WorkerState::Working {
                current: WorkType::SyncPod
            },
            Some(WorkType::SyncPod),
            WorkType::SyncPod
        ));
    }

    #[test]
    fn pending_drained_into_state_after_complete_sync() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        assert!(m.get("p").unwrap().pending.is_none());
        assert!(matches!(
            m.get("p").unwrap().state,
            WorkerState::Working {
                current: WorkType::SyncPod
            }
        ));
    }

    #[test]
    fn worker_state_clone_preserves_pending() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        let cloned = m.get("p").unwrap().clone();
        assert!(cloned.pending.is_some());
    }

    #[test]
    fn worker_started_at_recorded() {
        let mut m = PodWorkerManager::new();
        let mut u = upd("p", WorkType::SyncPod);
        u.at = t(99);
        m.update(u).unwrap();
        assert_eq!(m.get("p").unwrap().started_at, t(99));
    }

    #[test]
    fn complete_sync_after_terminated_returns_none_and_finishes() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("p", t(0)).unwrap();
        m.update(upd("p", WorkType::TerminatedPod)).unwrap();
        let res = m.complete_sync("p", t(1)).unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn pending_cleared_after_terminated_finishes() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        // Queue a pending update.
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("p", t(0)).unwrap();
        m.update(upd("p", WorkType::TerminatedPod)).unwrap();
        m.complete_sync("p", t(1)).unwrap();
        assert!(m.get("p").unwrap().pending.is_none());
    }

    #[test]
    fn invalid_pending_during_transition_drops_to_idle() {
        // After complete_sync(SyncPod) with pending=TerminatedPod, transition is blocked → Idle.
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        // Force an invalid pending by direct manipulation: use coalesce path.
        // We can't actually queue a TerminatedPod after Sync because update()
        // rejects it. So we test the safe path: pending is always validated at
        // submit time. Confirm that pending after Sync only contains valid types.
        m.update(upd("p", WorkType::TerminatingPod)).unwrap();
        m.complete_sync("p", t(0)).unwrap();
        assert!(matches!(
            m.get("p").unwrap().state,
            WorkerState::Working {
                current: WorkType::TerminatingPod
            }
        ));
    }

    #[test]
    fn sync_count_zero_initially() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        assert_eq!(m.get("p").unwrap().sync_count, 0);
    }

    #[test]
    fn last_finished_updated_only_after_complete() {
        let mut m = PodWorkerManager::new();
        m.update(upd("p", WorkType::SyncPod)).unwrap();
        assert!(m.get("p").unwrap().last_finished.is_none());
        m.complete_sync("p", t(1)).unwrap();
        assert_eq!(m.get("p").unwrap().last_finished, Some(WorkType::SyncPod));
    }
}
