//! PLEG — PodLifecycleEventGenerator.
//!
//! Upstream: kubernetes/kubernetes@v1.36.0
//!   - pkg/kubelet/pleg/pleg.go
//!   - pkg/kubelet/pleg/generic.go
//!   - pkg/kubelet/pleg/generic_test.go
//!
//! PLEG diffs successive runtime pod snapshots and emits
//! [`PodLifecycleEvent`]s for container state transitions. The
//! diff state machine is a line-by-line port of upstream's
//! `generateEvents` (generic.go:214-235):
//!
//! ```text
//! old → new
//!   *  → running     : ContainerStarted
//!   *  → exited      : ContainerDied
//!   *  → unknown     : ContainerChanged
//!   exited → ø       : ContainerRemoved
//!   *      → ø       : ContainerDied + ContainerRemoved
//! ```
//!
//! When the configured channel capacity is exceeded on a single
//! relist, surplus events are dropped (upstream's blocking channel
//! semantics with the dropped counter exposed by metric
//! `pleg_events_dropped_total`).
//!
//! ## Scope
//!
//! This module is a pure state machine. It owns no I/O — the
//! caller (`agent::sync_loop` or a test) drives it by calling
//! [`GenericPleg::relist`] with a fresh `Vec<PodSnapshot>` per
//! tick. Wiring to a real `cave-cri` adapter that produces the
//! snapshots is intentionally separate.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::collections::{HashMap, HashSet};

/// Mirrors upstream `PodLifeCycleEventType` (pleg.go:26-34).
///
/// `NetworkSetupCompleted` and `ConditionMet` are reserved for
/// the cri-bridge integration that fires sandbox-network and
/// readiness-condition cross-cutting signals; the diff state
/// machine itself never emits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PodLifecycleEventType {
    /// A container transitioned into the Running state.
    ContainerStarted,
    /// A container transitioned into the Exited state.
    ContainerDied,
    /// A container disappeared from the pod (after Died or directly).
    ContainerRemoved,
    /// A container transitioned into the Unknown state.
    ContainerChanged,
    /// Sandbox network plumbing is ready (reserved).
    NetworkSetupCompleted,
    /// A pod-level condition met (reserved).
    ConditionMet,
    /// Forced sync of the pod (reserved).
    PodSync,
}

/// Mirrors upstream `PodLifecycleEvent` (pleg.go:36-48).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PodLifecycleEvent {
    /// Pod UID this event applies to (upstream `ID` field).
    pub pod_uid: String,
    /// Event discriminant.
    pub event_type: PodLifecycleEventType,
    /// Container ID, if applicable. Upstream stuffs this into
    /// `Data interface{}` for the four diff events.
    pub container_id: Option<String>,
    /// Extra event payload (upstream `Data` field). For
    /// generateEvents-emitted events this duplicates `container_id`.
    pub data: Option<String>,
}

/// Mirrors upstream `kubecontainer.State` (container/runtime.go).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerState {
    /// Container process is running.
    Running,
    /// Container is created but not started (treated like Unknown
    /// for diff purposes — upstream comment, generic.go:200).
    Created,
    /// Container process has exited.
    Exited,
    /// Container state could not be determined.
    Unknown,
}

/// Internal PLEG state — upstream uses an unexported
/// `plegContainerState`. Modeled as a separate enum that includes
/// `NonExistent` because the diff comparison needs to express
/// "container is gone from this pod".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PlegState {
    Running,
    Exited,
    Unknown,
    NonExistent,
}

impl PlegState {
    /// Mirrors upstream `convertState` (generic.go:196-211).
    fn from_container(s: ContainerState) -> Self {
        match s {
            ContainerState::Running => PlegState::Running,
            ContainerState::Exited => PlegState::Exited,
            // Upstream treats Created as Unknown — kubelet doesn't
            // use Created yet (generic.go:200 comment).
            ContainerState::Created | ContainerState::Unknown => PlegState::Unknown,
        }
    }
}

/// A single container's identity and state in a pod snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContainerStatus {
    /// Container ID (upstream `kubecontainer.ContainerID.ID`).
    pub id: String,
    /// Current container state.
    pub state: ContainerState,
}

/// A pod snapshot produced by the runtime — drives one relist tick.
///
/// Mirrors upstream `kubecontainer.Pod` (container/runtime.go).
/// Both regular containers and sandboxes are diffed against the
/// previous snapshot using the same state machine
/// (generic.go::getContainerState — searches containers, then
/// sandboxes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodSnapshot {
    /// Pod UID.
    pub uid: String,
    /// Regular containers in this pod.
    pub containers: Vec<ContainerStatus>,
    /// Pod sandbox containers (the pause container in upstream).
    pub sandboxes: Vec<ContainerStatus>,
    /// Pod IP address (or first IP) as last reported by the runtime.
    pub ip: Option<String>,
}

/// Output of a single [`GenericPleg::relist`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelistOutcome {
    /// Events emitted from this relist (already bounded by capacity).
    pub events: Vec<PodLifecycleEvent>,
    /// Events that were generated but dropped because the channel
    /// capacity was exhausted. Mirrors upstream's
    /// `pleg_events_dropped_total` metric counter.
    pub dropped: usize,
}

/// Default `relistThreshold` (3 min) — upstream's
/// `RelistThreshold` (generic.go:50). After this much wall-clock
/// time without a relist, [`GenericPleg::healthy`] returns false.
const RELIST_THRESHOLD: ChronoDuration = ChronoDuration::minutes(3);

/// Per-pod cached snapshot record — mirrors upstream `podRecord`
/// (generic.go:88-93) although upstream tracks both `old` and
/// `current` while we keep just the last accepted snapshot.
#[derive(Debug, Clone)]
struct PodRecord {
    snapshot: PodSnapshot,
}

/// Generic PLEG implementation — pure state machine.
///
/// Concurrency: not internally synchronised — wrap in a `Mutex`
/// when sharing across tasks (upstream's `GenericPLEG` holds an
/// internal `relistLock`).
#[derive(Debug)]
pub struct GenericPleg {
    /// Per-pod last-accepted snapshot. Mirrors `podRecords` map.
    cache: HashMap<String, PodRecord>,
    /// Buffer capacity — events past this in a single relist are
    /// dropped (mirrors `eventChannel cap` upstream).
    channel_capacity: usize,
    /// Wall-clock timestamp of the last `relist` call (upstream
    /// `relistTime`). Drives `Healthy`.
    last_relist_time: Option<DateTime<Utc>>,
}

impl GenericPleg {
    /// Construct a new PLEG with the given event-channel capacity.
    /// Upstream defaults to 1000 (`plegChannelCapacity`,
    /// generic.go:55).
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            cache: HashMap::new(),
            channel_capacity,
            last_relist_time: None,
        }
    }

    /// Wall-clock timestamp of the most recent `relist`, if any.
    pub fn last_relist_time(&self) -> Option<DateTime<Utc>> {
        self.last_relist_time
    }

    /// Look up the last-accepted snapshot for a pod.
    pub fn cache_get(&self, uid: &str) -> Option<&PodSnapshot> {
        self.cache.get(uid).map(|r| &r.snapshot)
    }

    /// Health predicate — upstream `Healthy` (generic.go:172-179).
    /// Healthy iff there has been a relist within `RELIST_THRESHOLD`.
    pub fn healthy(&self, now: DateTime<Utc>) -> bool {
        match self.last_relist_time {
            None => false,
            Some(t) => now.signed_duration_since(t) <= RELIST_THRESHOLD,
        }
    }

    /// Diff the supplied snapshot list against the cached pod
    /// records and emit events.
    ///
    /// Mirrors upstream `Relist` (generic.go:253-285). Steps:
    ///   1. Record `relistTime = now`.
    ///   2. For each new pod snapshot, diff against the cached one.
    ///   3. For each cached pod NOT in the new snapshot set,
    ///      treat as full removal (every container disappears).
    ///   4. Apply the IP preservation rule (upstream's
    ///      `updateCache` step: if the new pod has no IP but the
    ///      cached one did, carry the cached IP forward).
    pub fn relist(
        &mut self,
        now: DateTime<Utc>,
        new_pods: Vec<PodSnapshot>,
    ) -> RelistOutcome {
        self.last_relist_time = Some(now);

        let new_uids: HashSet<String> = new_pods.iter().map(|p| p.uid.clone()).collect();
        let mut events: Vec<PodLifecycleEvent> = Vec::new();

        // 1) Diff every incoming pod against its cached predecessor.
        for new_pod in &new_pods {
            let old = self.cache.get(&new_pod.uid).map(|r| &r.snapshot);
            let pod_events = diff_pod(old, new_pod);
            events.extend(pod_events);
        }

        // 2) Pods that have disappeared from the runtime — every
        //    container is now non-existent.
        let gone: Vec<String> = self
            .cache
            .keys()
            .filter(|uid| !new_uids.contains(*uid))
            .cloned()
            .collect();
        for uid in &gone {
            if let Some(record) = self.cache.get(uid) {
                let empty = PodSnapshot {
                    uid: uid.clone(),
                    containers: Vec::new(),
                    sandboxes: Vec::new(),
                    ip: None,
                };
                let pod_events = diff_pod(Some(&record.snapshot), &empty);
                events.extend(pod_events);
            }
        }

        // 3) Apply IP-preservation, then update cache for present pods.
        for new_pod in new_pods {
            let preserved_ip = if new_pod.ip.is_none() {
                self.cache
                    .get(&new_pod.uid)
                    .and_then(|r| r.snapshot.ip.clone())
            } else {
                new_pod.ip.clone()
            };
            let snapshot = PodSnapshot {
                ip: preserved_ip,
                ..new_pod
            };
            self.cache
                .insert(snapshot.uid.clone(), PodRecord { snapshot });
        }

        // 4) Evict cache entries for gone pods (matches upstream
        //    `cleanupOrphanedPodCacheEntries`).
        for uid in &gone {
            self.cache.remove(uid);
        }

        // 5) Channel-capacity cap.
        let dropped = events.len().saturating_sub(self.channel_capacity);
        if dropped > 0 {
            events.truncate(self.channel_capacity);
        }

        RelistOutcome { events, dropped }
    }
}

/// Diff one pod's old vs new snapshot — fans out to per-container
/// `generate_events` over the union of (container, sandbox) IDs.
///
/// Mirrors upstream `Relist`'s inner per-pod loop
/// (generic.go:266-274) combined with `computeEvents`
/// (generic.go:378-389).
fn diff_pod(old: Option<&PodSnapshot>, new: &PodSnapshot) -> Vec<PodLifecycleEvent> {
    let mut events = Vec::new();

    // Union of container IDs (containers).
    let mut container_ids: HashSet<String> = HashSet::new();
    if let Some(o) = old {
        for c in &o.containers {
            container_ids.insert(c.id.clone());
        }
    }
    for c in &new.containers {
        container_ids.insert(c.id.clone());
    }
    for cid in &container_ids {
        let old_state = state_of_container(old, cid);
        let new_state = state_of_container(Some(new), cid);
        events.extend(generate_events(&new.uid, cid, old_state, new_state));
    }

    // Union of sandbox IDs.
    let mut sandbox_ids: HashSet<String> = HashSet::new();
    if let Some(o) = old {
        for c in &o.sandboxes {
            sandbox_ids.insert(c.id.clone());
        }
    }
    for c in &new.sandboxes {
        sandbox_ids.insert(c.id.clone());
    }
    for sid in &sandbox_ids {
        let old_state = state_of_sandbox(old, sid);
        let new_state = state_of_sandbox(Some(new), sid);
        events.extend(generate_events(&new.uid, sid, old_state, new_state));
    }

    // Deterministic order: sort by (event_type discriminant index,
    // container_id) so tests don't have to rely on HashSet iteration.
    events.sort_by(|a, b| {
        let ta = event_order_key(a.event_type);
        let tb = event_order_key(b.event_type);
        ta.cmp(&tb)
            .then_with(|| a.container_id.cmp(&b.container_id))
    });

    events
}

fn event_order_key(t: PodLifecycleEventType) -> u8 {
    match t {
        PodLifecycleEventType::ContainerRemoved => 0,
        PodLifecycleEventType::ContainerStarted => 1,
        PodLifecycleEventType::ContainerDied => 2,
        PodLifecycleEventType::ContainerChanged => 3,
        PodLifecycleEventType::NetworkSetupCompleted => 4,
        PodLifecycleEventType::ConditionMet => 5,
        PodLifecycleEventType::PodSync => 6,
    }
}

/// Mirrors upstream `getContainerState` (generic.go:240-251) for
/// regular containers.
fn state_of_container(pod: Option<&PodSnapshot>, cid: &str) -> PlegState {
    match pod {
        None => PlegState::NonExistent,
        Some(p) => p
            .containers
            .iter()
            .find(|c| c.id == cid)
            .map(|c| PlegState::from_container(c.state))
            .unwrap_or(PlegState::NonExistent),
    }
}

fn state_of_sandbox(pod: Option<&PodSnapshot>, sid: &str) -> PlegState {
    match pod {
        None => PlegState::NonExistent,
        Some(p) => p
            .sandboxes
            .iter()
            .find(|c| c.id == sid)
            .map(|c| PlegState::from_container(c.state))
            .unwrap_or(PlegState::NonExistent),
    }
}

/// Mirrors upstream `generateEvents` (generic.go:214-235) verbatim.
fn generate_events(
    pod_uid: &str,
    cid: &str,
    old: PlegState,
    new: PlegState,
) -> Vec<PodLifecycleEvent> {
    if old == new {
        return Vec::new();
    }
    let mk = |ty: PodLifecycleEventType| PodLifecycleEvent {
        pod_uid: pod_uid.to_string(),
        event_type: ty,
        container_id: Some(cid.to_string()),
        data: Some(cid.to_string()),
    };
    match new {
        PlegState::Running => vec![mk(PodLifecycleEventType::ContainerStarted)],
        PlegState::Exited => vec![mk(PodLifecycleEventType::ContainerDied)],
        PlegState::Unknown => vec![mk(PodLifecycleEventType::ContainerChanged)],
        PlegState::NonExistent => match old {
            PlegState::Exited => vec![mk(PodLifecycleEventType::ContainerRemoved)],
            // running / unknown / non_existent (the last is
            // unreachable thanks to the early-return on equal
            // states) — emit Died + Removed pair.
            _ => vec![
                mk(PodLifecycleEventType::ContainerDied),
                mk(PodLifecycleEventType::ContainerRemoved),
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-14T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn convert_state_running_maps_to_running() {
        assert_eq!(
            PlegState::from_container(ContainerState::Running),
            PlegState::Running
        );
    }

    #[test]
    fn convert_state_created_maps_to_unknown() {
        assert_eq!(
            PlegState::from_container(ContainerState::Created),
            PlegState::Unknown
        );
    }

    #[test]
    fn generate_events_no_change_returns_empty() {
        let v = generate_events("p", "c", PlegState::Running, PlegState::Running);
        assert!(v.is_empty());
    }

    #[test]
    fn generate_events_exited_to_nonexistent_is_removed_only() {
        let v = generate_events("p", "c", PlegState::Exited, PlegState::NonExistent);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].event_type, PodLifecycleEventType::ContainerRemoved);
    }

    #[test]
    fn relist_initial_running_emits_started() {
        let mut p = GenericPleg::new(1024);
        let out = p.relist(
            ts(),
            vec![PodSnapshot {
                uid: "p".into(),
                containers: vec![ContainerStatus {
                    id: "c".into(),
                    state: ContainerState::Running,
                }],
                sandboxes: vec![],
                ip: None,
            }],
        );
        assert_eq!(out.events.len(), 1);
        assert_eq!(
            out.events[0].event_type,
            PodLifecycleEventType::ContainerStarted
        );
    }
}
