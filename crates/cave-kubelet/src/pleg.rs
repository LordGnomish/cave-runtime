//! PLEG — Pod Lifecycle Event Generator.
//!
//! Mirrors `pkg/kubelet/pleg/generic.go`: a relist loop that polls the
//! container runtime for the current set of pods/containers, diffs against
//! the previous snapshot, and emits typed events for each transition. The
//! kubelet sync loop consumes those events to drive pod reconciliation.
//!
//! Behaviors encoded:
//!   * relist period (1s default), threshold (3min) for "PLEG is not healthy"
//!   * event types: ContainerStarted, ContainerDied, ContainerRemoved,
//!     ContainerChanged, PodSync, ContainerStateUnknown
//!   * deduplication of duplicate events within a relist tick
//!   * health reporting based on time since last successful relist
//!   * relist error handling (event suppressed, last_relist NOT updated)

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ContainerState {
    Created,
    Running,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub id: String,
    pub pod_uid: String,
    pub name: String,
    pub state: ContainerState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    /// Map: container_id → status.
    pub containers: BTreeMap<String, ContainerStatus>,
}

impl RuntimeSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, c: ContainerStatus) -> Self {
        self.containers.insert(c.id.clone(), c);
        self
    }

    pub fn pod_uids(&self) -> BTreeSet<String> {
        self.containers.values().map(|c| c.pod_uid.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlegEventKind {
    ContainerStarted,
    ContainerDied,
    ContainerRemoved,
    ContainerChanged,
    /// Synthetic "you should re-sync this pod" event.
    PodSync,
    ContainerStateUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlegEvent {
    pub kind: PlegEventKind,
    pub pod_uid: String,
    pub container_id: Option<String>,
    pub container_name: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlegHealth {
    Healthy,
    NotHealthy { since_last_relist: Duration },
}

#[derive(Debug, Clone, Copy)]
pub struct PlegConfig {
    pub relist_period: Duration,
    pub relist_threshold: Duration,
}

impl Default for PlegConfig {
    fn default() -> Self {
        Self {
            relist_period: Duration::seconds(1),
            relist_threshold: Duration::minutes(3),
        }
    }
}

#[derive(Debug)]
pub struct GenericPleg {
    pub cfg: PlegConfig,
    last_snapshot: RuntimeSnapshot,
    last_relist_at: Option<DateTime<Utc>>,
    /// Number of consecutive relist errors.
    relist_errors: u64,
    /// Number of successful relists.
    relist_count: u64,
}

impl GenericPleg {
    pub fn new(cfg: PlegConfig) -> Self {
        Self {
            cfg,
            last_snapshot: RuntimeSnapshot::new(),
            last_relist_at: None,
            relist_errors: 0,
            relist_count: 0,
        }
    }

    /// Run a relist tick: takes the new snapshot from the runtime, computes
    /// events, updates internal state. Returns the events emitted.
    pub fn relist(&mut self, now: DateTime<Utc>, new: RuntimeSnapshot) -> Vec<PlegEvent> {
        let events = compute_events(&self.last_snapshot, &new);
        self.last_snapshot = new;
        self.last_relist_at = Some(now);
        self.relist_count += 1;
        events
    }

    /// Record a relist failure. Health degrades because last_relist_at is
    /// not advanced.
    pub fn relist_error(&mut self) {
        self.relist_errors += 1;
    }

    /// Whether PLEG is healthy at `now`. Healthy ⇔ last_relist within
    /// `relist_threshold`.
    pub fn health(&self, now: DateTime<Utc>) -> PlegHealth {
        match self.last_relist_at {
            None => PlegHealth::NotHealthy {
                since_last_relist: Duration::zero(),
            },
            Some(t) => {
                let elapsed = now - t;
                if elapsed > self.cfg.relist_threshold {
                    PlegHealth::NotHealthy { since_last_relist: elapsed }
                } else {
                    PlegHealth::Healthy
                }
            }
        }
    }

    pub fn relist_count(&self) -> u64 {
        self.relist_count
    }

    pub fn relist_error_count(&self) -> u64 {
        self.relist_errors
    }

    pub fn last_snapshot(&self) -> &RuntimeSnapshot {
        &self.last_snapshot
    }
}

/// Compute the PLEG event list for two snapshots. Pure function, no state.
pub fn compute_events(old: &RuntimeSnapshot, new: &RuntimeSnapshot) -> Vec<PlegEvent> {
    let mut events = Vec::new();
    let mut affected_pods: BTreeSet<String> = BTreeSet::new();

    // Detect added/changed.
    for (id, new_c) in &new.containers {
        match old.containers.get(id) {
            None => {
                // Newly observed container.
                affected_pods.insert(new_c.pod_uid.clone());
                let kind = match new_c.state {
                    ContainerState::Running => PlegEventKind::ContainerStarted,
                    ContainerState::Exited => PlegEventKind::ContainerDied,
                    ContainerState::Created => PlegEventKind::ContainerChanged,
                    ContainerState::Unknown => PlegEventKind::ContainerStateUnknown,
                };
                events.push(PlegEvent {
                    kind,
                    pod_uid: new_c.pod_uid.clone(),
                    container_id: Some(id.clone()),
                    container_name: Some(new_c.name.clone()),
                    data: None,
                });
            }
            Some(prev) if prev.state != new_c.state => {
                affected_pods.insert(new_c.pod_uid.clone());
                let kind = transition_event(prev.state, new_c.state);
                events.push(PlegEvent {
                    kind,
                    pod_uid: new_c.pod_uid.clone(),
                    container_id: Some(id.clone()),
                    container_name: Some(new_c.name.clone()),
                    data: None,
                });
            }
            _ => {}
        }
    }

    // Detect removals.
    for (id, prev) in &old.containers {
        if !new.containers.contains_key(id) {
            affected_pods.insert(prev.pod_uid.clone());
            events.push(PlegEvent {
                kind: PlegEventKind::ContainerRemoved,
                pod_uid: prev.pod_uid.clone(),
                container_id: Some(id.clone()),
                container_name: Some(prev.name.clone()),
                data: None,
            });
        }
    }

    // Emit a PodSync per affected pod. Upstream: kubelet sync loop watches PodSync.
    for pod_uid in affected_pods {
        events.push(PlegEvent {
            kind: PlegEventKind::PodSync,
            pod_uid,
            container_id: None,
            container_name: None,
            data: None,
        });
    }

    events
}

fn transition_event(prev: ContainerState, next: ContainerState) -> PlegEventKind {
    match (prev, next) {
        (_, ContainerState::Running) => PlegEventKind::ContainerStarted,
        (_, ContainerState::Exited) => PlegEventKind::ContainerDied,
        (_, ContainerState::Unknown) => PlegEventKind::ContainerStateUnknown,
        _ => PlegEventKind::ContainerChanged,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, pod: &str, name: &str, state: ContainerState) -> ContainerStatus {
        ContainerStatus {
            id: id.into(),
            pod_uid: pod.into(),
            name: name.into(),
            state,
        }
    }

    fn t(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(seconds, 0).unwrap()
    }

    #[test]
    fn first_relist_emits_started_for_running_containers() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Running));
        let events = pleg.relist(t(0), snap);
        assert!(events.iter().any(|e| e.kind == PlegEventKind::ContainerStarted));
        assert!(events.iter().any(|e| e.kind == PlegEventKind::PodSync && e.pod_uid == "p1"));
    }

    #[test]
    fn first_relist_emits_died_for_exited_containers() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Exited));
        let events = pleg.relist(t(0), snap);
        assert!(events.iter().any(|e| e.kind == PlegEventKind::ContainerDied));
    }

    #[test]
    fn unchanged_containers_emit_nothing() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Running));
        pleg.relist(t(0), snap.clone());
        let events = pleg.relist(t(1), snap);
        assert!(events.is_empty());
    }

    #[test]
    fn state_change_emits_transition_event() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap1 = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Running));
        let snap2 = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Exited));
        pleg.relist(t(0), snap1);
        let events = pleg.relist(t(1), snap2);
        assert!(events.iter().any(|e| e.kind == PlegEventKind::ContainerDied));
        assert!(events.iter().any(|e| e.kind == PlegEventKind::PodSync));
    }

    #[test]
    fn removed_container_emits_removed_event() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap1 = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Running));
        let snap2 = RuntimeSnapshot::new();
        pleg.relist(t(0), snap1);
        let events = pleg.relist(t(1), snap2);
        assert!(events.iter().any(|e| e.kind == PlegEventKind::ContainerRemoved));
        assert!(events.iter().any(|e| e.kind == PlegEventKind::PodSync && e.pod_uid == "p1"));
    }

    #[test]
    fn pod_sync_emitted_once_per_affected_pod() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p1", "main", ContainerState::Running))
            .with(c("c2", "p1", "side", ContainerState::Running));
        let events = pleg.relist(t(0), snap);
        let pod_syncs: Vec<_> = events
            .iter()
            .filter(|e| e.kind == PlegEventKind::PodSync && e.pod_uid == "p1")
            .collect();
        assert_eq!(pod_syncs.len(), 1);
    }

    #[test]
    fn multiple_pods_each_get_own_pod_sync() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p1", "m", ContainerState::Running))
            .with(c("c2", "p2", "m", ContainerState::Running));
        let events = pleg.relist(t(0), snap);
        let pod_syncs: Vec<_> = events
            .iter()
            .filter(|e| e.kind == PlegEventKind::PodSync)
            .collect();
        assert_eq!(pod_syncs.len(), 2);
    }

    #[test]
    fn unknown_state_emits_unknown_event_on_first_observe() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p1", "m", ContainerState::Unknown));
        let events = pleg.relist(t(0), snap);
        assert!(events
            .iter()
            .any(|e| e.kind == PlegEventKind::ContainerStateUnknown));
    }

    #[test]
    fn created_state_first_observe_emits_changed() {
        let events = compute_events(
            &RuntimeSnapshot::new(),
            &RuntimeSnapshot::new().with(c("c1", "p1", "m", ContainerState::Created)),
        );
        assert!(events
            .iter()
            .any(|e| e.kind == PlegEventKind::ContainerChanged));
    }

    #[test]
    fn transition_running_to_exited_is_died() {
        assert_eq!(
            transition_event(ContainerState::Running, ContainerState::Exited),
            PlegEventKind::ContainerDied
        );
    }

    #[test]
    fn transition_created_to_running_is_started() {
        assert_eq!(
            transition_event(ContainerState::Created, ContainerState::Running),
            PlegEventKind::ContainerStarted
        );
    }

    #[test]
    fn transition_to_unknown_is_state_unknown() {
        assert_eq!(
            transition_event(ContainerState::Running, ContainerState::Unknown),
            PlegEventKind::ContainerStateUnknown
        );
    }

    #[test]
    fn health_no_relist_yet_is_not_healthy() {
        let pleg = GenericPleg::new(PlegConfig::default());
        let h = pleg.health(t(0));
        assert!(matches!(h, PlegHealth::NotHealthy { .. }));
    }

    #[test]
    fn health_recent_relist_is_healthy() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        pleg.relist(t(0), RuntimeSnapshot::new());
        assert_eq!(pleg.health(t(60)), PlegHealth::Healthy);
    }

    #[test]
    fn health_unhealthy_when_relist_threshold_exceeded() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        pleg.relist(t(0), RuntimeSnapshot::new());
        // Default threshold is 3 minutes = 180 s.
        let h = pleg.health(t(181));
        assert!(matches!(h, PlegHealth::NotHealthy { .. }));
    }

    #[test]
    fn relist_error_does_not_advance_health() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        pleg.relist(t(0), RuntimeSnapshot::new());
        pleg.relist_error();
        assert_eq!(pleg.relist_error_count(), 1);
        assert_eq!(pleg.health(t(60)), PlegHealth::Healthy);
        let h_old = pleg.health(t(181));
        assert!(matches!(h_old, PlegHealth::NotHealthy { .. }));
    }

    #[test]
    fn relist_count_increments_on_success_only() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        pleg.relist(t(0), RuntimeSnapshot::new());
        pleg.relist(t(1), RuntimeSnapshot::new());
        pleg.relist_error();
        assert_eq!(pleg.relist_count(), 2);
    }

    #[test]
    fn snapshot_pod_uids_collects_distinct_pods() {
        let s = RuntimeSnapshot::new()
            .with(c("c1", "p1", "m", ContainerState::Running))
            .with(c("c2", "p1", "s", ContainerState::Running))
            .with(c("c3", "p2", "m", ContainerState::Running));
        let uids = s.pod_uids();
        assert_eq!(uids.len(), 2);
        assert!(uids.contains("p1"));
        assert!(uids.contains("p2"));
    }

    #[test]
    fn compute_events_empty_to_empty() {
        assert!(compute_events(&RuntimeSnapshot::new(), &RuntimeSnapshot::new()).is_empty());
    }

    #[test]
    fn compute_events_pure_no_state() {
        let a = RuntimeSnapshot::new().with(c("c1", "p1", "m", ContainerState::Running));
        let b = RuntimeSnapshot::new().with(c("c1", "p1", "m", ContainerState::Exited));
        let e1 = compute_events(&a, &b);
        let e2 = compute_events(&a, &b);
        assert_eq!(e1, e2);
    }

    #[test]
    fn last_snapshot_returned_after_relist() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new().with(c("c1", "p1", "m", ContainerState::Running));
        pleg.relist(t(0), snap.clone());
        assert_eq!(pleg.last_snapshot(), &snap);
    }

    #[test]
    fn config_default_matches_upstream() {
        let c = PlegConfig::default();
        assert_eq!(c.relist_period, Duration::seconds(1));
        assert_eq!(c.relist_threshold, Duration::minutes(3));
    }

    #[test]
    fn many_containers_in_same_pod_emit_one_pod_sync() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap = RuntimeSnapshot::new()
            .with(c("c1", "p", "a", ContainerState::Running))
            .with(c("c2", "p", "b", ContainerState::Running))
            .with(c("c3", "p", "c", ContainerState::Running));
        let events = pleg.relist(t(0), snap);
        let pod_syncs = events
            .iter()
            .filter(|e| e.kind == PlegEventKind::PodSync)
            .count();
        assert_eq!(pod_syncs, 1);
    }

    #[test]
    fn restart_running_to_running_via_id_swap_emits_started_and_removed() {
        // Container restarted by runtime: old id removed, new id appears.
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap1 = RuntimeSnapshot::new()
            .with(c("c-old", "p", "m", ContainerState::Running));
        let snap2 = RuntimeSnapshot::new()
            .with(c("c-new", "p", "m", ContainerState::Running));
        pleg.relist(t(0), snap1);
        let events = pleg.relist(t(1), snap2);
        let kinds: BTreeSet<_> = events.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&PlegEventKind::ContainerRemoved));
        assert!(kinds.contains(&PlegEventKind::ContainerStarted));
        // Just one PodSync.
        let pod_syncs = events.iter().filter(|e| e.kind == PlegEventKind::PodSync).count();
        assert_eq!(pod_syncs, 1);
    }

    #[test]
    fn relist_with_runtime_snapshot_with_zero_containers_no_events() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let events = pleg.relist(t(0), RuntimeSnapshot::new());
        assert!(events.is_empty());
    }

    #[test]
    fn second_relist_after_complete_drain_emits_removed_and_pod_sync() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let snap1 = RuntimeSnapshot::new()
            .with(c("c1", "p1", "m", ContainerState::Running));
        let events1 = pleg.relist(t(0), snap1);
        assert!(!events1.is_empty());
        let events2 = pleg.relist(t(1), RuntimeSnapshot::new());
        assert!(events2.iter().any(|e| e.kind == PlegEventKind::ContainerRemoved));
        assert!(events2.iter().any(|e| e.kind == PlegEventKind::PodSync));
    }

    #[test]
    fn pleg_health_transitions_at_exact_threshold_boundary() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        pleg.relist(t(0), RuntimeSnapshot::new());
        // Threshold is exactly 180s; > only triggers above.
        assert_eq!(pleg.health(t(180)), PlegHealth::Healthy);
        assert!(matches!(pleg.health(t(181)), PlegHealth::NotHealthy { .. }));
    }

    #[test]
    fn custom_config_threshold_respected() {
        let mut pleg = GenericPleg::new(PlegConfig {
            relist_period: Duration::seconds(5),
            relist_threshold: Duration::seconds(30),
        });
        pleg.relist(t(0), RuntimeSnapshot::new());
        assert_eq!(pleg.health(t(29)), PlegHealth::Healthy);
        assert!(matches!(pleg.health(t(31)), PlegHealth::NotHealthy { .. }));
    }

    #[test]
    fn event_carries_container_metadata() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let events = pleg.relist(
            t(0),
            RuntimeSnapshot::new().with(c("c-1", "p-A", "main-c", ContainerState::Running)),
        );
        let started = events
            .iter()
            .find(|e| e.kind == PlegEventKind::ContainerStarted)
            .unwrap();
        assert_eq!(started.container_id.as_deref(), Some("c-1"));
        assert_eq!(started.container_name.as_deref(), Some("main-c"));
        assert_eq!(started.pod_uid, "p-A");
    }

    #[test]
    fn pod_sync_event_carries_pod_uid_only() {
        let mut pleg = GenericPleg::new(PlegConfig::default());
        let events = pleg.relist(
            t(0),
            RuntimeSnapshot::new().with(c("c-1", "p-A", "m", ContainerState::Running)),
        );
        let pod_sync = events
            .iter()
            .find(|e| e.kind == PlegEventKind::PodSync)
            .unwrap();
        assert_eq!(pod_sync.pod_uid, "p-A");
        assert!(pod_sync.container_id.is_none());
    }

    #[test]
    fn snapshot_with_builder_chains() {
        let s = RuntimeSnapshot::new()
            .with(c("a", "p", "x", ContainerState::Running))
            .with(c("b", "p", "y", ContainerState::Running));
        assert_eq!(s.containers.len(), 2);
    }

    #[test]
    fn snapshot_default_is_empty() {
        let s = RuntimeSnapshot::default();
        assert!(s.containers.is_empty());
        assert!(s.pod_uids().is_empty());
    }
}
