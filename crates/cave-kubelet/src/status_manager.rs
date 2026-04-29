//! Status manager — kubelet pod status sync to apiserver.
//!
//! Mirrors `pkg/kubelet/status/status_manager.go`: in-memory cache of pod
//! status, version-stamped updates, conflict detection (via UID+ResourceVersion),
//! batched sync to apiserver, retry on transient failures, and graceful-deletion
//! coordination (status updates blocked once a pod is being deleted).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodCondition {
    pub kind: String,
    pub status: bool,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub name: String,
    pub ready: bool,
    pub restart_count: u32,
    pub state: ContainerStateLite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerStateLite {
    Waiting,
    Running,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodStatus {
    pub phase: PodPhase,
    pub conditions: Vec<PodCondition>,
    pub container_statuses: Vec<ContainerStatus>,
    pub message: Option<String>,
    pub reason: Option<String>,
    /// Internal kubelet version (monotonic) — distinct from apiserver
    /// `metadata.resourceVersion`.
    pub version: u64,
}

impl PodStatus {
    pub fn new() -> Self {
        Self {
            phase: PodPhase::Pending,
            conditions: Vec::new(),
            container_statuses: Vec::new(),
            message: None,
            reason: None,
            version: 0,
        }
    }
}

impl Default for PodStatus {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedStatus {
    pub status: PodStatus,
    /// Apiserver-side resourceVersion last observed.
    pub api_resource_version: String,
    /// Pod UID at the time this status was cached.
    pub pod_uid: String,
    /// Whether this status has been synced to apiserver.
    pub synced: bool,
    /// Whether the pod is being deleted (deletionTimestamp set).
    pub deletion_in_progress: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StatusError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: pod uid changed")]
    UidConflict,
    #[error("forbidden: pod is being deleted")]
    PodDeleted,
    #[error("transient: {0}")]
    Transient(String),
}

pub type StatusResult<T> = Result<T, StatusError>;

#[derive(Debug, Default)]
pub struct StatusManager {
    cache: BTreeMap<String, CachedStatus>,
    /// Sync queue: pod UIDs that need to be flushed to apiserver.
    pending: Vec<String>,
    /// Number of failed flush attempts per pod.
    retry_count: BTreeMap<String, u32>,
}

impl StatusManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.cache.len()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn retry_count(&self, pod_uid: &str) -> u32 {
        self.retry_count.get(pod_uid).copied().unwrap_or(0)
    }

    /// Update kubelet's view of a pod's status. The version monotonically
    /// increases. If the pod is already marked deletion-in-progress, only
    /// terminal phases are accepted.
    pub fn set_pod_status(
        &mut self,
        pod_uid: &str,
        api_resource_version: &str,
        new_status: PodStatus,
    ) -> StatusResult<()> {
        if pod_uid.is_empty() {
            return Err(StatusError::NotFound("empty pod uid".into()));
        }
        match self.cache.get_mut(pod_uid) {
            Some(entry) => {
                if entry.deletion_in_progress
                    && !matches!(new_status.phase, PodPhase::Succeeded | PodPhase::Failed)
                {
                    return Err(StatusError::PodDeleted);
                }
                if !needs_update(&entry.status, &new_status) {
                    return Ok(());
                }
                let next_version = entry.status.version + 1;
                entry.status = PodStatus {
                    version: next_version,
                    ..new_status
                };
                entry.api_resource_version = api_resource_version.to_string();
                entry.synced = false;
            }
            None => {
                self.cache.insert(
                    pod_uid.to_string(),
                    CachedStatus {
                        status: PodStatus { version: 1, ..new_status },
                        api_resource_version: api_resource_version.to_string(),
                        pod_uid: pod_uid.to_string(),
                        synced: false,
                        deletion_in_progress: false,
                    },
                );
            }
        }
        if !self.pending.contains(&pod_uid.to_string()) {
            self.pending.push(pod_uid.to_string());
        }
        Ok(())
    }

    pub fn get(&self, pod_uid: &str) -> Option<&CachedStatus> {
        self.cache.get(pod_uid)
    }

    pub fn mark_deletion(&mut self, pod_uid: &str) -> StatusResult<()> {
        let entry = self
            .cache
            .get_mut(pod_uid)
            .ok_or_else(|| StatusError::NotFound(pod_uid.into()))?;
        entry.deletion_in_progress = true;
        Ok(())
    }

    pub fn drain_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// Mark a pod as successfully synced. Resets retry counter.
    pub fn mark_synced(&mut self, pod_uid: &str, observed_resource_version: &str) -> StatusResult<()> {
        let entry = self
            .cache
            .get_mut(pod_uid)
            .ok_or_else(|| StatusError::NotFound(pod_uid.into()))?;
        entry.api_resource_version = observed_resource_version.to_string();
        entry.synced = true;
        self.retry_count.remove(pod_uid);
        Ok(())
    }

    /// Mark a pod's flush attempt as failed; re-enqueues for retry.
    pub fn mark_failed(&mut self, pod_uid: &str) {
        *self.retry_count.entry(pod_uid.to_string()).or_insert(0) += 1;
        if !self.pending.contains(&pod_uid.to_string()) {
            self.pending.push(pod_uid.to_string());
        }
    }

    /// Drop status for a pod that has been removed.
    pub fn remove(&mut self, pod_uid: &str) {
        self.cache.remove(pod_uid);
        self.pending.retain(|u| u != pod_uid);
        self.retry_count.remove(pod_uid);
    }

    /// Validate that the apiserver hasn't replaced the pod (uid mismatch).
    pub fn validate_uid(&self, pod_uid: &str, observed_uid: &str) -> StatusResult<()> {
        match self.cache.get(pod_uid) {
            None => Err(StatusError::NotFound(pod_uid.into())),
            Some(entry) => {
                if entry.pod_uid != observed_uid {
                    Err(StatusError::UidConflict)
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// Whether two statuses differ enough to warrant a sync. Status manager
/// short-circuits no-op updates per upstream `isStatusEqual`.
pub fn needs_update(old: &PodStatus, new: &PodStatus) -> bool {
    if old.phase != new.phase {
        return true;
    }
    if old.message != new.message || old.reason != new.reason {
        return true;
    }
    if old.container_statuses != new.container_statuses {
        return true;
    }
    if old.conditions.len() != new.conditions.len() {
        return true;
    }
    for (a, b) in old.conditions.iter().zip(new.conditions.iter()) {
        if a.kind != b.kind
            || a.status != b.status
            || a.reason != b.reason
            || a.message != b.message
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(phase: PodPhase) -> PodStatus {
        PodStatus {
            phase,
            ..PodStatus::new()
        }
    }

    #[test]
    fn set_status_records_in_cache() {
        let mut m = StatusManager::new();
        m.set_pod_status("p1", "rv1", st(PodPhase::Running)).unwrap();
        assert_eq!(m.count(), 1);
        let c = m.get("p1").unwrap();
        assert_eq!(c.status.phase, PodPhase::Running);
        assert_eq!(c.status.version, 1);
    }

    #[test]
    fn set_status_rejects_empty_uid() {
        let mut m = StatusManager::new();
        assert!(m.set_pod_status("", "rv1", st(PodPhase::Pending)).is_err());
    }

    #[test]
    fn version_monotonic_on_repeated_distinct_updates() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv1", st(PodPhase::Pending)).unwrap();
        m.set_pod_status("p", "rv2", st(PodPhase::Running)).unwrap();
        assert_eq!(m.get("p").unwrap().status.version, 2);
        m.set_pod_status("p", "rv3", st(PodPhase::Succeeded)).unwrap();
        assert_eq!(m.get("p").unwrap().status.version, 3);
    }

    #[test]
    fn no_op_update_does_not_advance_version() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        assert_eq!(m.get("p").unwrap().status.version, 1);
    }

    #[test]
    fn no_op_update_does_not_enqueue() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.drain_pending();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        assert_eq!(m.pending_count(), 0);
    }

    #[test]
    fn distinct_pods_each_have_own_version() {
        let mut m = StatusManager::new();
        m.set_pod_status("a", "r", st(PodPhase::Running)).unwrap();
        m.set_pod_status("b", "r", st(PodPhase::Running)).unwrap();
        m.set_pod_status("a", "r", st(PodPhase::Succeeded)).unwrap();
        assert_eq!(m.get("a").unwrap().status.version, 2);
        assert_eq!(m.get("b").unwrap().status.version, 1);
    }

    #[test]
    fn deletion_blocks_non_terminal_updates() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "r", st(PodPhase::Running)).unwrap();
        m.mark_deletion("p").unwrap();
        let err = m.set_pod_status("p", "r", st(PodPhase::Pending)).unwrap_err();
        assert!(matches!(err, StatusError::PodDeleted));
    }

    #[test]
    fn deletion_allows_terminal_updates() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "r", st(PodPhase::Running)).unwrap();
        m.mark_deletion("p").unwrap();
        m.set_pod_status("p", "r", st(PodPhase::Succeeded)).unwrap();
        m.set_pod_status("q", "r", st(PodPhase::Pending)).unwrap();
        m.mark_deletion("q").unwrap();
        m.set_pod_status("q", "r", st(PodPhase::Failed)).unwrap();
    }

    #[test]
    fn mark_deletion_unknown_pod_errors() {
        let mut m = StatusManager::new();
        assert!(m.mark_deletion("ghost").is_err());
    }

    #[test]
    fn drain_pending_returns_uids_in_insertion_order() {
        let mut m = StatusManager::new();
        m.set_pod_status("p1", "r", st(PodPhase::Pending)).unwrap();
        m.set_pod_status("p2", "r", st(PodPhase::Pending)).unwrap();
        m.set_pod_status("p3", "r", st(PodPhase::Pending)).unwrap();
        let drained = m.drain_pending();
        assert_eq!(drained, vec!["p1".to_string(), "p2".into(), "p3".into()]);
        assert_eq!(m.pending_count(), 0);
    }

    #[test]
    fn drain_pending_dedups_same_pod() {
        let mut m = StatusManager::new();
        m.set_pod_status("p1", "r", st(PodPhase::Pending)).unwrap();
        m.set_pod_status("p1", "r", st(PodPhase::Running)).unwrap();
        let drained = m.drain_pending();
        assert_eq!(drained, vec!["p1".to_string()]);
    }

    #[test]
    fn mark_synced_sets_synced_flag_and_clears_retry() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.mark_failed("p");
        assert_eq!(m.retry_count("p"), 1);
        m.mark_synced("p", "rv2").unwrap();
        assert!(m.get("p").unwrap().synced);
        assert_eq!(m.retry_count("p"), 0);
        assert_eq!(m.get("p").unwrap().api_resource_version, "rv2");
    }

    #[test]
    fn mark_synced_unknown_pod_errors() {
        let mut m = StatusManager::new();
        assert!(m.mark_synced("ghost", "rv").is_err());
    }

    #[test]
    fn mark_failed_increments_retry_and_re_enqueues() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.drain_pending();
        m.mark_failed("p");
        m.mark_failed("p");
        assert_eq!(m.retry_count("p"), 2);
        let drained = m.drain_pending();
        assert_eq!(drained, vec!["p".to_string()]);
    }

    #[test]
    fn remove_clears_all_state_for_pod() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.mark_failed("p");
        m.remove("p");
        assert!(m.get("p").is_none());
        assert_eq!(m.pending_count(), 0);
        assert_eq!(m.retry_count("p"), 0);
    }

    #[test]
    fn validate_uid_match_ok() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.validate_uid("p", "p").unwrap();
    }

    #[test]
    fn validate_uid_mismatch_errors() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        let err = m.validate_uid("p", "different-uid").unwrap_err();
        assert!(matches!(err, StatusError::UidConflict));
    }

    #[test]
    fn validate_uid_unknown_pod_errors() {
        let m = StatusManager::new();
        assert!(matches!(
            m.validate_uid("ghost", "ghost"),
            Err(StatusError::NotFound(_))
        ));
    }

    #[test]
    fn needs_update_phase_change() {
        let a = st(PodPhase::Pending);
        let b = st(PodPhase::Running);
        assert!(needs_update(&a, &b));
    }

    #[test]
    fn needs_update_message_change() {
        let a = PodStatus { message: Some("a".into()), ..st(PodPhase::Running) };
        let b = PodStatus { message: Some("b".into()), ..st(PodPhase::Running) };
        assert!(needs_update(&a, &b));
    }

    #[test]
    fn needs_update_reason_change() {
        let a = PodStatus { reason: None, ..st(PodPhase::Running) };
        let b = PodStatus { reason: Some("OOMKilled".into()), ..st(PodPhase::Running) };
        assert!(needs_update(&a, &b));
    }

    #[test]
    fn needs_update_container_statuses_change() {
        let a = st(PodPhase::Running);
        let b = PodStatus {
            container_statuses: vec![ContainerStatus {
                name: "c".into(),
                ready: true,
                restart_count: 0,
                state: ContainerStateLite::Running,
            }],
            ..st(PodPhase::Running)
        };
        assert!(needs_update(&a, &b));
    }

    #[test]
    fn needs_update_conditions_count_diff() {
        let a = st(PodPhase::Running);
        let b = PodStatus {
            conditions: vec![PodCondition {
                kind: "Ready".into(),
                status: true,
                reason: None,
                message: None,
                last_transition_time: Utc::now(),
            }],
            ..st(PodPhase::Running)
        };
        assert!(needs_update(&a, &b));
    }

    #[test]
    fn needs_update_condition_field_diff() {
        let now = Utc::now();
        let a = PodStatus {
            conditions: vec![PodCondition {
                kind: "Ready".into(),
                status: true,
                reason: None,
                message: None,
                last_transition_time: now,
            }],
            ..st(PodPhase::Running)
        };
        let b = PodStatus {
            conditions: vec![PodCondition {
                kind: "Ready".into(),
                status: false,
                reason: None,
                message: None,
                last_transition_time: now,
            }],
            ..st(PodPhase::Running)
        };
        assert!(needs_update(&a, &b));
    }

    #[test]
    fn needs_update_identical_returns_false() {
        let s = st(PodPhase::Running);
        assert!(!needs_update(&s, &s));
    }

    #[test]
    fn needs_update_ignores_version_field() {
        // Version is bumped by manager, not part of equality.
        let mut a = st(PodPhase::Running);
        let mut b = st(PodPhase::Running);
        a.version = 1;
        b.version = 99;
        assert!(!needs_update(&a, &b));
    }

    #[test]
    fn pending_queue_drained_resets_to_empty() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "r", st(PodPhase::Running)).unwrap();
        assert_eq!(m.pending_count(), 1);
        m.drain_pending();
        assert_eq!(m.pending_count(), 0);
    }

    #[test]
    fn retry_count_zero_for_unknown_pod() {
        let m = StatusManager::new();
        assert_eq!(m.retry_count("ghost"), 0);
    }

    #[test]
    fn cached_status_synced_starts_false() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Pending)).unwrap();
        assert!(!m.get("p").unwrap().synced);
    }

    #[test]
    fn pod_status_default_is_pending_zero_version() {
        let s = PodStatus::default();
        assert_eq!(s.phase, PodPhase::Pending);
        assert_eq!(s.version, 0);
    }

    #[test]
    fn deletion_flag_persists_across_terminal_updates() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.mark_deletion("p").unwrap();
        m.set_pod_status("p", "rv", st(PodPhase::Succeeded)).unwrap();
        assert!(m.get("p").unwrap().deletion_in_progress);
    }

    #[test]
    fn count_returns_cache_size() {
        let mut m = StatusManager::new();
        assert_eq!(m.count(), 0);
        m.set_pod_status("a", "r", st(PodPhase::Running)).unwrap();
        m.set_pod_status("b", "r", st(PodPhase::Running)).unwrap();
        assert_eq!(m.count(), 2);
    }

    #[test]
    fn version_does_not_increment_for_no_op_after_failed() {
        let mut m = StatusManager::new();
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        m.mark_failed("p");
        m.set_pod_status("p", "rv", st(PodPhase::Running)).unwrap();
        assert_eq!(m.get("p").unwrap().status.version, 1);
    }
}
