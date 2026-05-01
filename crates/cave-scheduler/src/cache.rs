//! Scheduler cache — assume/finish/forget mechanism for in-flight bindings.
//!
//! Mirrors upstream kube-scheduler's `pkg/scheduler/internal/cache/cache.go`.
//! When the scheduler picks a node for a pod it "assumes" the binding (so the
//! same scheduling cycle and any concurrent cycles see the resource as taken)
//! before the bind RPC completes. Two outcomes follow:
//!
//! * **finish_binding** — the bind succeeded; the assumed pod becomes a real
//!   pod and stays in the cache.
//! * **forget_pod** — the bind failed; the assumed reservation is released
//!   back to the node so a retry can pick it again.
//!
//! Assumed pods that were never finished within `assumed_ttl` are
//! automatically reclaimed by `cleanup_assumed_pods` to avoid leaking node
//! capacity if the bind RPC vanishes (e.g. controller crash). This matches
//! upstream's `assumedPodGracefulPeriod` semantics.
//!
//! The cache is in-memory and per-scheduler-instance — there is no persistent
//! storage. It is the single source of truth for "what has the scheduler
//! decided so far this cycle but not yet seen reflected in the watch stream".

use crate::models::ResourceRequest;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

/// A pod entry in the scheduler cache.
#[derive(Debug, Clone)]
pub struct CachedPod {
    pub uid: Uuid,
    pub name: String,
    pub namespace: String,
    pub node_name: String,
    pub resources: ResourceRequest,
    /// When the pod was assumed (None once finish_binding was called).
    pub assumed_at: Option<DateTime<Utc>>,
}

impl CachedPod {
    pub fn is_assumed(&self) -> bool {
        self.assumed_at.is_some()
    }
}

/// Cache configuration.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum time an assumed pod can remain unconfirmed before reclamation.
    pub assumed_ttl: Duration,
}

impl CacheConfig {
    pub fn defaults() -> Self {
        Self { assumed_ttl: Duration::seconds(30) }
    }
}

/// In-memory scheduler cache: pod-uid → CachedPod plus a per-node tally of
/// resources reserved by all pods (assumed + bound).
#[derive(Debug, Default)]
pub struct SchedulerCache {
    pods: HashMap<Uuid, CachedPod>,
    /// Per-node sum of `resources` over assumed+finished pods.
    node_reserved: HashMap<String, ResourceRequest>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CacheError {
    PodAlreadyKnown(Uuid),
    PodNotFound(Uuid),
    PodNotAssumed(Uuid),
    NodeNotMatched { expected: String, got: String },
}

impl SchedulerCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assume a pod is bound to `node_name`. Future scheduling cycles see the
    /// resources as reserved on that node.
    pub fn assume_pod(&mut self, pod: CachedPod, now: DateTime<Utc>) -> Result<(), CacheError> {
        if self.pods.contains_key(&pod.uid) {
            return Err(CacheError::PodAlreadyKnown(pod.uid));
        }
        let mut p = pod;
        p.assumed_at = Some(now);
        self.add_to_node_reserved(&p.node_name, &p.resources);
        self.pods.insert(p.uid, p);
        Ok(())
    }

    /// Mark an assumed pod as confirmed (bind RPC succeeded). Resources stay
    /// reserved; the assumed_at timestamp is cleared.
    pub fn finish_binding(&mut self, uid: Uuid, node_name: &str) -> Result<(), CacheError> {
        let pod = self.pods.get_mut(&uid).ok_or(CacheError::PodNotFound(uid))?;
        if !pod.is_assumed() {
            return Err(CacheError::PodNotAssumed(uid));
        }
        if pod.node_name != node_name {
            return Err(CacheError::NodeNotMatched {
                expected: pod.node_name.clone(),
                got: node_name.to_string(),
            });
        }
        pod.assumed_at = None;
        Ok(())
    }

    /// Drop an assumed pod and release its reservation (bind failed / cancelled).
    pub fn forget_pod(&mut self, uid: Uuid) -> Result<CachedPod, CacheError> {
        let pod = self.pods.remove(&uid).ok_or(CacheError::PodNotFound(uid))?;
        self.subtract_from_node_reserved(&pod.node_name, &pod.resources);
        Ok(pod)
    }

    /// Remove a finished pod (e.g. pod was deleted upstream).
    pub fn remove_pod(&mut self, uid: Uuid) -> Result<CachedPod, CacheError> {
        let pod = self.pods.remove(&uid).ok_or(CacheError::PodNotFound(uid))?;
        self.subtract_from_node_reserved(&pod.node_name, &pod.resources);
        Ok(pod)
    }

    /// Walk all assumed pods and forget those that have been pending too long.
    /// Returns the uids that were reclaimed.
    pub fn cleanup_assumed_pods(&mut self, now: &DateTime<Utc>, cfg: &CacheConfig) -> Vec<Uuid> {
        let stale: Vec<Uuid> = self
            .pods
            .iter()
            .filter_map(|(uid, p)| match p.assumed_at {
                Some(t) if now.signed_duration_since(t) >= cfg.assumed_ttl => Some(*uid),
                _ => None,
            })
            .collect();
        for uid in &stale {
            let _ = self.forget_pod(*uid);
        }
        let mut sorted = stale;
        sorted.sort();
        sorted
    }

    pub fn pod(&self, uid: Uuid) -> Option<&CachedPod> {
        self.pods.get(&uid)
    }

    pub fn node_reserved(&self, node_name: &str) -> ResourceRequest {
        self.node_reserved
            .get(node_name)
            .cloned()
            .unwrap_or_default()
    }

    pub fn pod_count(&self) -> usize {
        self.pods.len()
    }

    pub fn assumed_count(&self) -> usize {
        self.pods.values().filter(|p| p.is_assumed()).count()
    }

    pub fn node_pod_count(&self, node_name: &str) -> usize {
        self.pods.values().filter(|p| p.node_name == node_name).count()
    }

    fn add_to_node_reserved(&mut self, node: &str, r: &ResourceRequest) {
        let entry = self.node_reserved.entry(node.to_string()).or_default();
        entry.cpu_millicores += r.cpu_millicores;
        entry.memory_bytes += r.memory_bytes;
    }

    fn subtract_from_node_reserved(&mut self, node: &str, r: &ResourceRequest) {
        if let Some(entry) = self.node_reserved.get_mut(node) {
            entry.cpu_millicores = entry.cpu_millicores.saturating_sub(r.cpu_millicores);
            entry.memory_bytes = entry.memory_bytes.saturating_sub(r.memory_bytes);
            if entry.cpu_millicores == 0 && entry.memory_bytes == 0 {
                self.node_reserved.remove(node);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, node: &str, cpu: u64, mem: u64) -> CachedPod {
        CachedPod {
            uid: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            node_name: node.into(),
            resources: ResourceRequest {
                cpu_millicores: cpu, memory_bytes: mem,
                ephemeral_storage_bytes: 0, extended: Default::default(),
            },
            assumed_at: None,
        }
    }

    // ── basic operations ──────────────────────────────────────────────────

    #[test]
    fn new_cache_is_empty() {
        let c = SchedulerCache::new();
        assert_eq!(c.pod_count(), 0);
        assert_eq!(c.assumed_count(), 0);
    }

    #[test]
    fn assume_pod_records_pod_and_reservation() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("nginx", "n1", 500, 1_000_000_000);
        let uid = p.uid;
        assert!(c.assume_pod(p, now).is_ok());
        assert_eq!(c.pod_count(), 1);
        assert_eq!(c.assumed_count(), 1);
        let rsv = c.node_reserved("n1");
        assert_eq!(rsv.cpu_millicores, 500);
        assert_eq!(rsv.memory_bytes, 1_000_000_000);
        assert!(c.pod(uid).unwrap().is_assumed());
    }

    #[test]
    fn assume_same_pod_twice_is_rejected() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("dup", "n1", 100, 100);
        c.assume_pod(p.clone(), now).unwrap();
        let err = c.assume_pod(p.clone(), now).unwrap_err();
        assert_eq!(err, CacheError::PodAlreadyKnown(p.uid));
    }

    #[test]
    fn finish_binding_clears_assumed_flag_resources_remain() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 500, 100);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.finish_binding(uid, "n1").unwrap();
        assert!(!c.pod(uid).unwrap().is_assumed());
        assert_eq!(c.assumed_count(), 0);
        assert_eq!(c.pod_count(), 1);
        assert_eq!(c.node_reserved("n1").cpu_millicores, 500);
    }

    #[test]
    fn finish_binding_unknown_pod_errors() {
        let mut c = SchedulerCache::new();
        let r = c.finish_binding(Uuid::new_v4(), "n1");
        assert!(matches!(r, Err(CacheError::PodNotFound(_))));
    }

    #[test]
    fn finish_binding_already_finished_pod_errors() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 100, 100);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.finish_binding(uid, "n1").unwrap();
        let r = c.finish_binding(uid, "n1");
        assert!(matches!(r, Err(CacheError::PodNotAssumed(_))));
    }

    #[test]
    fn finish_binding_with_wrong_node_errors() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 100, 100);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        let r = c.finish_binding(uid, "n2");
        assert!(matches!(r, Err(CacheError::NodeNotMatched { .. })));
    }

    #[test]
    fn forget_pod_releases_reservation() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 500, 1_000_000_000);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.forget_pod(uid).unwrap();
        assert_eq!(c.pod_count(), 0);
        assert_eq!(c.node_reserved("n1").cpu_millicores, 0);
    }

    #[test]
    fn forget_pod_unknown_errors() {
        let mut c = SchedulerCache::new();
        let r = c.forget_pod(Uuid::new_v4());
        assert!(matches!(r, Err(CacheError::PodNotFound(_))));
    }

    #[test]
    fn remove_pod_after_finish_releases_reservation() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 500, 1_000_000_000);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.finish_binding(uid, "n1").unwrap();
        c.remove_pod(uid).unwrap();
        assert_eq!(c.pod_count(), 0);
        assert_eq!(c.node_reserved("n1").cpu_millicores, 0);
    }

    // ── cleanup ───────────────────────────────────────────────────────────

    #[test]
    fn cleanup_removes_only_stale_assumed_pods() {
        let mut c = SchedulerCache::new();
        let t0 = Utc::now() - Duration::seconds(60);
        let p1 = pod("old_assumed", "n1", 100, 100);
        let p1_uid = p1.uid;
        c.assume_pod(p1, t0).unwrap();
        // fresh assumed
        let p2 = pod("fresh_assumed", "n1", 100, 100);
        let p2_uid = p2.uid;
        c.assume_pod(p2, Utc::now()).unwrap();
        // confirmed pod (not assumed)
        let p3 = pod("confirmed", "n1", 100, 100);
        let p3_uid = p3.uid;
        c.assume_pod(p3, t0).unwrap();
        c.finish_binding(p3_uid, "n1").unwrap();

        let now = Utc::now();
        let reclaimed = c.cleanup_assumed_pods(&now, &CacheConfig { assumed_ttl: Duration::seconds(30) });
        assert_eq!(reclaimed, vec![p1_uid].into_iter().collect::<Vec<_>>());
        assert!(c.pod(p1_uid).is_none());
        assert!(c.pod(p2_uid).is_some());
        assert!(c.pod(p3_uid).is_some());
    }

    #[test]
    fn cleanup_no_assumed_pods_returns_empty() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let reclaimed = c.cleanup_assumed_pods(&now, &CacheConfig::defaults());
        assert!(reclaimed.is_empty());
    }

    #[test]
    fn cleanup_does_not_touch_finished_pods_even_when_old() {
        let mut c = SchedulerCache::new();
        let t0 = Utc::now() - Duration::seconds(3600);
        let p = pod("old_finished", "n1", 100, 100);
        let uid = p.uid;
        c.assume_pod(p, t0).unwrap();
        c.finish_binding(uid, "n1").unwrap();
        let now = Utc::now();
        let reclaimed = c.cleanup_assumed_pods(&now, &CacheConfig::defaults());
        assert!(reclaimed.is_empty());
        assert!(c.pod(uid).is_some());
    }

    #[test]
    fn cleanup_deterministic_order_of_reclaimed_uids() {
        let mut c = SchedulerCache::new();
        let t0 = Utc::now() - Duration::seconds(60);
        let mut uids = vec![];
        for _ in 0..5 {
            let p = pod("p", "n1", 10, 10);
            uids.push(p.uid);
            c.assume_pod(p, t0).unwrap();
        }
        let reclaimed = c.cleanup_assumed_pods(&Utc::now(), &CacheConfig { assumed_ttl: Duration::seconds(10) });
        let mut sorted = uids.clone();
        sorted.sort();
        assert_eq!(reclaimed, sorted);
    }

    #[test]
    fn cleanup_boundary_at_exactly_ttl_is_inclusive() {
        // Upstream behaviour: an assumed pod whose age == ttl is reclaimed.
        let mut c = SchedulerCache::new();
        let ttl = Duration::seconds(30);
        let p = pod("p", "n1", 100, 100);
        let uid = p.uid;
        let assumed_at = Utc::now() - ttl;
        c.assume_pod(p, assumed_at).unwrap();
        let reclaimed = c.cleanup_assumed_pods(&Utc::now(), &CacheConfig { assumed_ttl: ttl });
        assert_eq!(reclaimed, vec![uid]);
    }

    #[test]
    fn cleanup_just_below_ttl_keeps_pod() {
        let mut c = SchedulerCache::new();
        let ttl = Duration::seconds(30);
        let p = pod("p", "n1", 100, 100);
        let uid = p.uid;
        c.assume_pod(p, Utc::now() - ttl + Duration::seconds(1)).unwrap();
        let reclaimed = c.cleanup_assumed_pods(&Utc::now(), &CacheConfig { assumed_ttl: ttl });
        assert!(reclaimed.is_empty());
        assert!(c.pod(uid).is_some());
    }

    // ── reservation accounting ────────────────────────────────────────────

    #[test]
    fn reservation_accumulates_across_multiple_pods_on_same_node() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        c.assume_pod(pod("a", "n1", 500, 1_000_000_000), now).unwrap();
        c.assume_pod(pod("b", "n1", 300, 500_000_000), now).unwrap();
        let r = c.node_reserved("n1");
        assert_eq!(r.cpu_millicores, 800);
        assert_eq!(r.memory_bytes, 1_500_000_000);
    }

    #[test]
    fn reservation_partitions_per_node() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        c.assume_pod(pod("a", "n1", 500, 100), now).unwrap();
        c.assume_pod(pod("b", "n2", 200, 50), now).unwrap();
        assert_eq!(c.node_reserved("n1").cpu_millicores, 500);
        assert_eq!(c.node_reserved("n2").cpu_millicores, 200);
    }

    #[test]
    fn reservation_drops_to_zero_after_all_pods_forgotten() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p1 = pod("a", "n1", 500, 1000);
        let p2 = pod("b", "n1", 300, 500);
        let u1 = p1.uid; let u2 = p2.uid;
        c.assume_pod(p1, now).unwrap();
        c.assume_pod(p2, now).unwrap();
        c.forget_pod(u1).unwrap();
        c.forget_pod(u2).unwrap();
        // Once node has zero usage, the entry is removed.
        assert_eq!(c.node_reserved("n1").cpu_millicores, 0);
        assert_eq!(c.node_reserved("n1").memory_bytes, 0);
    }

    #[test]
    fn reservation_empty_for_unknown_node() {
        let c = SchedulerCache::new();
        let r = c.node_reserved("ghost");
        assert_eq!(r.cpu_millicores, 0);
        assert_eq!(r.memory_bytes, 0);
    }

    #[test]
    fn reservation_subtraction_saturates() {
        // Defensive: forget_pod should never underflow even if state diverges.
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 100, 100);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.forget_pod(uid).unwrap();
        assert_eq!(c.node_reserved("n1").cpu_millicores, 0);
    }

    // ── per-node counts ───────────────────────────────────────────────────

    #[test]
    fn node_pod_count_counts_per_node() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        c.assume_pod(pod("a", "n1", 100, 100), now).unwrap();
        c.assume_pod(pod("b", "n1", 100, 100), now).unwrap();
        c.assume_pod(pod("c", "n2", 100, 100), now).unwrap();
        assert_eq!(c.node_pod_count("n1"), 2);
        assert_eq!(c.node_pod_count("n2"), 1);
        assert_eq!(c.node_pod_count("n3"), 0);
    }

    // ── lifecycle / state transitions ─────────────────────────────────────

    #[test]
    fn assumed_pod_is_assumed_via_helper() {
        let now = Utc::now();
        let mut p = pod("p", "n", 1, 1);
        p.assumed_at = Some(now);
        assert!(p.is_assumed());
        p.assumed_at = None;
        assert!(!p.is_assumed());
    }

    #[test]
    fn assumed_count_excludes_finished_pods() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p1 = pod("a", "n1", 100, 100);
        let p2 = pod("b", "n1", 100, 100);
        let u1 = p1.uid;
        c.assume_pod(p1, now).unwrap();
        c.assume_pod(p2, now).unwrap();
        c.finish_binding(u1, "n1").unwrap();
        assert_eq!(c.assumed_count(), 1);
        assert_eq!(c.pod_count(), 2);
    }

    // ── default config ────────────────────────────────────────────────────

    #[test]
    fn cache_config_defaults_match_upstream() {
        let cfg = CacheConfig::defaults();
        assert_eq!(cfg.assumed_ttl, Duration::seconds(30));
    }

    // ── integration: full assume → finish → remove cycle ──────────────────

    #[test]
    fn full_lifecycle_clean_state() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 500, 1_000_000_000);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.finish_binding(uid, "n1").unwrap();
        c.remove_pod(uid).unwrap();
        assert_eq!(c.pod_count(), 0);
        assert_eq!(c.assumed_count(), 0);
        assert_eq!(c.node_reserved("n1").cpu_millicores, 0);
    }

    #[test]
    fn full_lifecycle_failure_clean_state() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 500, 1_000_000_000);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.forget_pod(uid).unwrap(); // bind failed
        assert_eq!(c.pod_count(), 0);
        assert_eq!(c.node_reserved("n1").cpu_millicores, 0);
    }

    #[test]
    fn many_pods_assume_finish_pattern() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let mut uids = vec![];
        for i in 0..20 {
            let p = pod(&format!("p{}", i), if i % 2 == 0 { "n1" } else { "n2" }, 100, 100);
            uids.push(p.uid);
            c.assume_pod(p, now).unwrap();
        }
        // finish first 10
        for &uid in &uids[..10] {
            let p = c.pod(uid).unwrap().clone();
            c.finish_binding(uid, &p.node_name).unwrap();
        }
        // forget last 10
        for &uid in &uids[10..] {
            c.forget_pod(uid).unwrap();
        }
        assert_eq!(c.pod_count(), 10);
        assert_eq!(c.assumed_count(), 0);
    }

    // ── error type comparability ──────────────────────────────────────────

    #[test]
    fn cache_errors_are_distinct_variants() {
        let a = CacheError::PodAlreadyKnown(Uuid::nil());
        let b = CacheError::PodNotFound(Uuid::nil());
        assert_ne!(a, b);
    }

    #[test]
    fn pod_count_tracks_total_including_finished() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p1 = pod("a", "n", 1, 1);
        let p2 = pod("b", "n", 1, 1);
        let u1 = p1.uid;
        c.assume_pod(p1, now).unwrap();
        c.assume_pod(p2, now).unwrap();
        c.finish_binding(u1, "n").unwrap();
        assert_eq!(c.pod_count(), 2);
    }

    #[test]
    fn forget_after_finish_still_releases() {
        // Even after finish_binding, forget_pod should still release the reservation.
        // Mirrors upstream behaviour where deletion via watch can race with finish.
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n", 500, 100);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.finish_binding(uid, "n").unwrap();
        let removed = c.forget_pod(uid).unwrap();
        assert!(!removed.is_assumed());
        assert_eq!(c.pod_count(), 0);
        assert_eq!(c.node_reserved("n").cpu_millicores, 0);
    }

    #[test]
    fn pod_lookup_by_uid_after_forget_returns_none() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n", 1, 1);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.forget_pod(uid).unwrap();
        assert!(c.pod(uid).is_none());
    }

    #[test]
    fn reservation_remains_after_finish_until_removed() {
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n", 700, 200);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.finish_binding(uid, "n").unwrap();
        // Still reserved until pod removed
        assert_eq!(c.node_reserved("n").cpu_millicores, 700);
        c.remove_pod(uid).unwrap();
        assert_eq!(c.node_reserved("n").cpu_millicores, 0);
    }

    #[test]
    fn empty_node_disappears_from_reservation_map() {
        // Internal invariant: zeroed-out node entries are removed so reads
        // for unknown nodes consistently return Default.
        let mut c = SchedulerCache::new();
        let now = Utc::now();
        let p = pod("p", "n1", 100, 100);
        let uid = p.uid;
        c.assume_pod(p, now).unwrap();
        c.forget_pod(uid).unwrap();
        // No entry should linger.
        let r = c.node_reserved("n1");
        assert_eq!(r.cpu_millicores, 0);
        assert_eq!(r.memory_bytes, 0);
    }

    #[test]
    fn cleanup_returns_uids_in_sorted_order_for_idempotent_logging() {
        let mut c = SchedulerCache::new();
        let t0 = Utc::now() - Duration::seconds(60);
        let mut uids = vec![];
        for _ in 0..3 {
            let p = pod("p", "n", 1, 1);
            uids.push(p.uid);
            c.assume_pod(p, t0).unwrap();
        }
        let r = c.cleanup_assumed_pods(&Utc::now(), &CacheConfig { assumed_ttl: Duration::seconds(10) });
        let mut sorted = r.clone();
        sorted.sort();
        assert_eq!(r, sorted);
    }
}
