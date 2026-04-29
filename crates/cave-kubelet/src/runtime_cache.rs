//! Runtime cache — TTL-based pod-and-container state cache.
//!
//! Mirrors `pkg/kubelet/container/runtime_cache.go`: caches the result of
//! the (relatively expensive) `RuntimeService.ListPodSandbox` /
//! `ListContainers` calls and serves stale results within a configurable
//! `min_cache_update_period`. ForceUpdate bypasses the TTL.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxState {
    Ready,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodSandbox {
    pub id: String,
    pub pod_uid: String,
    pub name: String,
    pub namespace: String,
    pub state: SandboxState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerRuntimeState {
    Created,
    Running,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedContainer {
    pub id: String,
    pub pod_sandbox_id: String,
    pub pod_uid: String,
    pub name: String,
    pub state: ContainerRuntimeState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedPod {
    pub uid: String,
    pub sandboxes: Vec<PodSandbox>,
    pub containers: Vec<CachedContainer>,
}

impl CachedPod {
    pub fn ready_sandboxes(&self) -> usize {
        self.sandboxes
            .iter()
            .filter(|s| s.state == SandboxState::Ready)
            .count()
    }

    pub fn running_containers(&self) -> usize {
        self.containers
            .iter()
            .filter(|c| c.state == ContainerRuntimeState::Running)
            .count()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RuntimeCacheError {
    #[error("not found: {0}")]
    NotFound(String),
}

pub type RcResult<T> = Result<T, RuntimeCacheError>;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeCacheConfig {
    /// Maximum age the kubelet sync loop accepts for cached data.
    pub min_cache_update_period: Duration,
}

impl Default for RuntimeCacheConfig {
    fn default() -> Self {
        Self {
            min_cache_update_period: Duration::seconds(2),
        }
    }
}

#[derive(Debug)]
pub struct RuntimeCache {
    pub cfg: RuntimeCacheConfig,
    pods: BTreeMap<String, CachedPod>,
    last_update_at: Option<DateTime<Utc>>,
    update_count: u64,
}

impl RuntimeCache {
    pub fn new(cfg: RuntimeCacheConfig) -> Self {
        Self {
            cfg,
            pods: BTreeMap::new(),
            last_update_at: None,
            update_count: 0,
        }
    }

    pub fn pod_count(&self) -> usize {
        self.pods.len()
    }

    pub fn last_update_at(&self) -> Option<DateTime<Utc>> {
        self.last_update_at
    }

    pub fn update_count(&self) -> u64 {
        self.update_count
    }

    /// Whether the cache is fresh enough that a `get_pods` call should
    /// return cached data instead of forcing a refresh.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        match self.last_update_at {
            None => false,
            Some(t) => (now - t) < self.cfg.min_cache_update_period,
        }
    }

    /// Replace the cache contents from a fresh runtime listing.
    pub fn update(
        &mut self,
        now: DateTime<Utc>,
        sandboxes: Vec<PodSandbox>,
        containers: Vec<CachedContainer>,
    ) {
        let mut by_pod: BTreeMap<String, CachedPod> = BTreeMap::new();
        for s in sandboxes {
            let entry = by_pod.entry(s.pod_uid.clone()).or_default();
            entry.uid = s.pod_uid.clone();
            entry.sandboxes.push(s);
        }
        for c in containers {
            let entry = by_pod.entry(c.pod_uid.clone()).or_default();
            entry.uid = c.pod_uid.clone();
            entry.containers.push(c);
        }
        self.pods = by_pod;
        self.last_update_at = Some(now);
        self.update_count += 1;
    }

    /// Return cached pods. Always returns the cached snapshot; caller is
    /// responsible for invoking `update` first.
    pub fn list_pods(&self) -> Vec<CachedPod> {
        self.pods.values().cloned().collect()
    }

    pub fn get_pod(&self, pod_uid: &str) -> Option<&CachedPod> {
        self.pods.get(pod_uid)
    }

    pub fn list_running_containers(&self) -> Vec<&CachedContainer> {
        self.pods
            .values()
            .flat_map(|p| p.containers.iter())
            .filter(|c| c.state == ContainerRuntimeState::Running)
            .collect()
    }

    pub fn count_sandboxes(&self) -> usize {
        self.pods.values().map(|p| p.sandboxes.len()).sum()
    }

    pub fn count_containers(&self) -> usize {
        self.pods.values().map(|p| p.containers.len()).sum()
    }

    pub fn invalidate(&mut self) {
        self.last_update_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(s, 0).unwrap()
    }

    fn sb(id: &str, pod: &str, name: &str, state: SandboxState) -> PodSandbox {
        PodSandbox {
            id: id.into(),
            pod_uid: pod.into(),
            name: name.into(),
            namespace: "default".into(),
            state,
            created_at: t(0),
        }
    }

    fn cn(id: &str, sb_id: &str, pod: &str, name: &str, state: ContainerRuntimeState) -> CachedContainer {
        CachedContainer {
            id: id.into(),
            pod_sandbox_id: sb_id.into(),
            pod_uid: pod.into(),
            name: name.into(),
            state,
            created_at: t(0),
        }
    }

    #[test]
    fn fresh_returns_false_when_never_updated() {
        let c = RuntimeCache::new(RuntimeCacheConfig::default());
        assert!(!c.is_fresh(t(0)));
    }

    #[test]
    fn fresh_after_update_within_period() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![], vec![]);
        assert!(c.is_fresh(t(1)));
    }

    #[test]
    fn stale_after_period_elapsed() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![], vec![]);
        // default period 2s
        assert!(!c.is_fresh(t(3)));
    }

    #[test]
    fn update_replaces_full_state() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![sb("s1", "p", "n", SandboxState::Ready)], vec![]);
        assert_eq!(c.pod_count(), 1);
        c.update(t(1), vec![], vec![]);
        assert_eq!(c.pod_count(), 0);
    }

    #[test]
    fn update_groups_sandboxes_by_pod_uid() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![
                sb("s1", "p1", "n", SandboxState::Ready),
                sb("s2", "p2", "n", SandboxState::Ready),
                sb("s3", "p1", "n", SandboxState::NotReady),
            ],
            vec![],
        );
        assert_eq!(c.pod_count(), 2);
        let p1 = c.get_pod("p1").unwrap();
        assert_eq!(p1.sandboxes.len(), 2);
    }

    #[test]
    fn update_groups_containers_by_pod_uid() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![],
            vec![
                cn("c1", "s1", "p1", "main", ContainerRuntimeState::Running),
                cn("c2", "s1", "p1", "side", ContainerRuntimeState::Running),
                cn("c3", "s2", "p2", "main", ContainerRuntimeState::Running),
            ],
        );
        assert_eq!(c.pod_count(), 2);
        assert_eq!(c.get_pod("p1").unwrap().containers.len(), 2);
    }

    #[test]
    fn list_running_containers_filters_state() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![],
            vec![
                cn("c1", "s", "p", "a", ContainerRuntimeState::Running),
                cn("c2", "s", "p", "b", ContainerRuntimeState::Exited),
                cn("c3", "s", "p", "c", ContainerRuntimeState::Running),
            ],
        );
        assert_eq!(c.list_running_containers().len(), 2);
    }

    #[test]
    fn ready_sandboxes_count() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![
                sb("s1", "p", "n", SandboxState::Ready),
                sb("s2", "p", "n", SandboxState::Ready),
                sb("s3", "p", "n", SandboxState::NotReady),
            ],
            vec![],
        );
        assert_eq!(c.get_pod("p").unwrap().ready_sandboxes(), 2);
    }

    #[test]
    fn running_containers_count() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![],
            vec![
                cn("c1", "s", "p", "a", ContainerRuntimeState::Running),
                cn("c2", "s", "p", "b", ContainerRuntimeState::Exited),
            ],
        );
        assert_eq!(c.get_pod("p").unwrap().running_containers(), 1);
    }

    #[test]
    fn list_pods_returns_all_pods() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![
                sb("s1", "p1", "n", SandboxState::Ready),
                sb("s2", "p2", "n", SandboxState::Ready),
            ],
            vec![],
        );
        assert_eq!(c.list_pods().len(), 2);
    }

    #[test]
    fn get_pod_unknown_returns_none() {
        let c = RuntimeCache::new(RuntimeCacheConfig::default());
        assert!(c.get_pod("ghost").is_none());
    }

    #[test]
    fn count_sandboxes_aggregates() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![
                sb("s1", "p1", "a", SandboxState::Ready),
                sb("s2", "p2", "b", SandboxState::Ready),
                sb("s3", "p1", "c", SandboxState::NotReady),
            ],
            vec![],
        );
        assert_eq!(c.count_sandboxes(), 3);
    }

    #[test]
    fn count_containers_aggregates() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![],
            vec![
                cn("c1", "s", "p1", "a", ContainerRuntimeState::Running),
                cn("c2", "s", "p2", "b", ContainerRuntimeState::Running),
                cn("c3", "s", "p2", "c", ContainerRuntimeState::Exited),
            ],
        );
        assert_eq!(c.count_containers(), 3);
    }

    #[test]
    fn update_count_increments() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![], vec![]);
        c.update(t(1), vec![], vec![]);
        assert_eq!(c.update_count(), 2);
    }

    #[test]
    fn invalidate_marks_stale() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![], vec![]);
        c.invalidate();
        assert!(!c.is_fresh(t(1)));
    }

    #[test]
    fn last_update_at_recorded() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(42), vec![], vec![]);
        assert_eq!(c.last_update_at(), Some(t(42)));
    }

    #[test]
    fn config_default_is_two_seconds() {
        let cfg = RuntimeCacheConfig::default();
        assert_eq!(cfg.min_cache_update_period, Duration::seconds(2));
    }

    #[test]
    fn fresh_threshold_is_strict_less_than() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![], vec![]);
        // Exactly equal → not fresh.
        assert!(!c.is_fresh(t(2)));
        assert!(c.is_fresh(t(1)));
    }

    #[test]
    fn empty_cache_lists_empty() {
        let c = RuntimeCache::new(RuntimeCacheConfig::default());
        assert!(c.list_pods().is_empty());
        assert!(c.list_running_containers().is_empty());
    }

    #[test]
    fn pod_with_only_containers_no_sandboxes_still_recorded() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![],
            vec![cn("c", "s", "p", "n", ContainerRuntimeState::Running)],
        );
        assert!(c.get_pod("p").is_some());
    }

    #[test]
    fn pod_with_only_sandboxes_no_containers_still_recorded() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![sb("s", "p", "n", SandboxState::Ready)], vec![]);
        assert!(c.get_pod("p").is_some());
    }

    #[test]
    fn pods_sorted_by_uid_in_list() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(
            t(0),
            vec![
                sb("s1", "z", "n", SandboxState::Ready),
                sb("s2", "a", "n", SandboxState::Ready),
                sb("s3", "m", "n", SandboxState::Ready),
            ],
            vec![],
        );
        let pods = c.list_pods();
        assert_eq!(pods[0].uid, "a");
        assert_eq!(pods[1].uid, "m");
        assert_eq!(pods[2].uid, "z");
    }

    #[test]
    fn sandbox_serde_round_trip() {
        let s = sb("s", "p", "n", SandboxState::Ready);
        let ser = serde_json::to_string(&s).unwrap();
        let s2: PodSandbox = serde_json::from_str(&ser).unwrap();
        assert_eq!(s, s2);
    }

    #[test]
    fn container_state_unknown_handling() {
        let c1 = cn("c", "s", "p", "n", ContainerRuntimeState::Unknown);
        let mut rc = RuntimeCache::new(RuntimeCacheConfig::default());
        rc.update(t(0), vec![], vec![c1]);
        assert_eq!(rc.list_running_containers().len(), 0);
    }

    #[test]
    fn cached_pod_default_is_empty() {
        let p = CachedPod::default();
        assert!(p.sandboxes.is_empty());
        assert!(p.containers.is_empty());
        assert_eq!(p.ready_sandboxes(), 0);
        assert_eq!(p.running_containers(), 0);
    }

    #[test]
    fn update_resets_pod_set_each_time() {
        let mut c = RuntimeCache::new(RuntimeCacheConfig::default());
        c.update(t(0), vec![sb("s1", "p1", "n", SandboxState::Ready)], vec![]);
        c.update(t(1), vec![sb("s2", "p2", "n", SandboxState::Ready)], vec![]);
        assert!(c.get_pod("p1").is_none());
        assert!(c.get_pod("p2").is_some());
    }
}
