// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pod garbage collector — sweeps terminated pods on this kubelet instance.
//!
//! Mirrors upstream kubelet's `pkg/kubelet/pod/pod_gc.go` (and the related
//! `kubelet/status/generate.go` lifecycle decisions). The collector keeps the
//! number of *terminated* (Succeeded/Failed) pods on the node bounded so we
//! don't accumulate bookkeeping forever, and respects a minimum-age window so
//! operators have a chance to inspect crash output.
//!
//! Three knobs (mirroring upstream `KubeletConfig.MaxPerPodContainerCount` and
//! `MinAge` semantics applied at the pod level):
//!
//! * **`max_terminated`** — cap on simultaneously kept terminated pods.
//! * **`min_age`**       — terminated pods younger than this are never collected.
//! * **`per_namespace`** — when true, the cap is applied per namespace
//!   (matches `--maximum-dead-containers-per-container=N` per-pod ergonomics).
//!
//! Eviction order: oldest-first by `terminated_at`, then by pod uid (stable tie
//! break). Terminated pods that have not yet aged past `min_age` are skipped
//! even when over the cap — this matches upstream's `evictTerminatedPods`
//! ordering: aged candidates first, never-collect-young.

use crate::models::{ManagedPod, PodPhase};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

/// Configuration for the pod garbage collector.
#[derive(Debug, Clone)]
pub struct PodGcConfig {
    pub max_terminated: usize,
    pub min_age: Duration,
    pub per_namespace: bool,
}

impl PodGcConfig {
    /// Defaults match upstream kubelet defaults (--maximum-dead-pods=...).
    pub fn defaults() -> Self {
        Self {
            max_terminated: 100,
            min_age: Duration::minutes(1),
            per_namespace: false,
        }
    }
}

/// Reason a pod was kept alive (not collected).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepReason {
    /// Pod is still active (Pending, Running, Unknown).
    Active,
    /// Pod is terminated but younger than `min_age`.
    YoungerThanMinAge,
    /// Pod is terminated, aged, but under the keep-cap.
    UnderCap,
}

/// Outcome of a single GC pass.
#[derive(Debug, Clone, Default)]
pub struct GcReport {
    pub collected: Vec<Uuid>,
    pub kept: HashMap<Uuid, KeepReason>,
}

impl GcReport {
    pub fn collected_count(&self) -> usize {
        self.collected.len()
    }
    pub fn kept_count(&self) -> usize {
        self.kept.len()
    }
}

/// Result of a single pod's terminal classification.
fn is_terminated(pod: &ManagedPod) -> bool {
    matches!(pod.status, PodPhase::Succeeded | PodPhase::Failed)
}

/// Best-effort termination timestamp: prefer `started_at` when not set, fall back
/// to `assigned_at`. Upstream uses container `finished_at`; we don't model that
/// per-container at the GC level so we approximate with the pod-level signal.
fn terminated_at(pod: &ManagedPod) -> DateTime<Utc> {
    pod.started_at.unwrap_or(pod.assigned_at)
}

/// Plan a GC pass over the given pod set. Pure function — does not mutate.
/// Returns the uid set to collect plus a per-pod reason for those kept.
pub fn plan_gc(pods: &[ManagedPod], cfg: &PodGcConfig, now: DateTime<Utc>) -> GcReport {
    let mut report = GcReport::default();

    // Partition.
    let mut terminated: Vec<&ManagedPod> = Vec::new();
    for pod in pods {
        if is_terminated(pod) {
            terminated.push(pod);
        } else {
            report.kept.insert(pod.uid, KeepReason::Active);
        }
    }

    // Group by namespace if per_namespace; otherwise one global bucket.
    let bucket_key = |p: &&ManagedPod| -> String {
        if cfg.per_namespace { p.namespace.clone() } else { String::new() }
    };
    let mut buckets: HashMap<String, Vec<&ManagedPod>> = HashMap::new();
    for p in &terminated {
        buckets.entry(bucket_key(p)).or_default().push(p);
    }

    for (_key, mut bucket) in buckets {
        // Stable order: oldest first, then uid.
        bucket.sort_by(|a, b| {
            terminated_at(a)
                .cmp(&terminated_at(b))
                .then_with(|| a.uid.cmp(&b.uid))
        });

        // Apply min_age filter: aged first, young always kept.
        let mut aged: Vec<&ManagedPod> = Vec::new();
        for p in bucket {
            let age = now.signed_duration_since(terminated_at(p));
            if age >= cfg.min_age {
                aged.push(p);
            } else {
                report.kept.insert(p.uid, KeepReason::YoungerThanMinAge);
            }
        }

        // Cap: keep the N newest aged, collect the rest.
        let to_collect = aged.len().saturating_sub(cfg.max_terminated);
        for p in aged.iter().take(to_collect) {
            report.collected.push(p.uid);
        }
        for p in aged.iter().skip(to_collect) {
            report.kept.insert(p.uid, KeepReason::UnderCap);
        }
    }

    // Stable output order for tests / logs.
    report.collected.sort();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContainerState, ManagedContainer};

    fn pod(name: &str, ns: &str, phase: PodPhase, age_secs: i64, now: DateTime<Utc>) -> ManagedPod {
        let term = now - Duration::seconds(age_secs);
        ManagedPod {
            uid: Uuid::new_v4(),
            name: name.into(),
            namespace: ns.into(),
            containers: vec![ManagedContainer {
                name: "c".into(),
                image: "img".into(),
                container_id: None,
                state: ContainerState::Terminated {
                    exit_code: 0,
                    reason: "done".into(),
                    finished_at: term,
                },
                restart_count: 0,
                ready: false,
            }],
            status: phase,
            assigned_at: term - Duration::seconds(60),
            started_at: Some(term),
            node_name: "n1".into(),
        }
    }

    #[test]
    fn config_defaults_match_upstream_kubelet_shape() {
        let c = PodGcConfig::defaults();
        assert_eq!(c.max_terminated, 100);
        assert_eq!(c.min_age, Duration::minutes(1));
        assert!(!c.per_namespace);
    }

    #[test]
    fn empty_input_yields_empty_report() {
        let now = Utc::now();
        let r = plan_gc(&[], &PodGcConfig::defaults(), now);
        assert!(r.collected.is_empty());
        assert!(r.kept.is_empty());
    }

    #[test]
    fn active_pods_are_always_kept_as_active() {
        let now = Utc::now();
        let pods = vec![
            pod("a", "ns1", PodPhase::Running, 0, now),
            pod("b", "ns1", PodPhase::Pending, 0, now),
            pod("c", "ns1", PodPhase::Unknown, 0, now),
        ];
        let r = plan_gc(&pods, &PodGcConfig::defaults(), now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.len(), 3);
        for v in r.kept.values() {
            assert_eq!(v, &KeepReason::Active);
        }
    }

    #[test]
    fn terminated_younger_than_min_age_is_kept() {
        let now = Utc::now();
        let pods = vec![pod("a", "ns", PodPhase::Succeeded, 5, now)];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.values().next(), Some(&KeepReason::YoungerThanMinAge));
    }

    #[test]
    fn terminated_at_or_above_min_age_is_eligible() {
        let now = Utc::now();
        let pods = vec![pod("a", "ns", PodPhase::Succeeded, 60, now)];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected.len(), 1);
    }

    #[test]
    fn under_cap_aged_pods_are_kept() {
        let now = Utc::now();
        let pods = vec![
            pod("a", "ns", PodPhase::Succeeded, 100, now),
            pod("b", "ns", PodPhase::Failed,    200, now),
        ];
        let cfg = PodGcConfig { max_terminated: 5, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.len(), 2);
        for v in r.kept.values() {
            assert_eq!(v, &KeepReason::UnderCap);
        }
    }

    #[test]
    fn over_cap_oldest_terminated_evicted_first() {
        let now = Utc::now();
        let p_old = pod("old", "ns", PodPhase::Succeeded, 1000, now);
        let p_mid = pod("mid", "ns", PodPhase::Succeeded, 500, now);
        let p_new = pod("new", "ns", PodPhase::Succeeded, 100, now);
        let pods = vec![p_old.clone(), p_mid.clone(), p_new.clone()];
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        // Oldest two must be collected, newest kept.
        assert_eq!(r.collected.len(), 2);
        assert!(r.collected.contains(&p_old.uid));
        assert!(r.collected.contains(&p_mid.uid));
        assert_eq!(r.kept.get(&p_new.uid), Some(&KeepReason::UnderCap));
    }

    #[test]
    fn cap_zero_collects_all_aged() {
        let now = Utc::now();
        let p1 = pod("a", "ns", PodPhase::Failed, 600, now);
        let p2 = pod("b", "ns", PodPhase::Succeeded, 700, now);
        let pods = vec![p1.clone(), p2.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected.len(), 2);
    }

    #[test]
    fn per_namespace_cap_applies_independently() {
        let now = Utc::now();
        let a1 = pod("a1", "ns-a", PodPhase::Succeeded, 1000, now);
        let a2 = pod("a2", "ns-a", PodPhase::Succeeded, 500, now);
        let b1 = pod("b1", "ns-b", PodPhase::Succeeded, 1000, now);
        let b2 = pod("b2", "ns-b", PodPhase::Succeeded, 500, now);
        let pods = vec![a1.clone(), a2.clone(), b1.clone(), b2.clone()];
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(10), per_namespace: true };
        let r = plan_gc(&pods, &cfg, now);
        // each namespace should evict its single oldest (a1, b1)
        assert_eq!(r.collected.len(), 2);
        assert!(r.collected.contains(&a1.uid));
        assert!(r.collected.contains(&b1.uid));
        assert_eq!(r.kept.get(&a2.uid), Some(&KeepReason::UnderCap));
        assert_eq!(r.kept.get(&b2.uid), Some(&KeepReason::UnderCap));
    }

    #[test]
    fn per_namespace_off_pools_namespaces_into_one_bucket() {
        let now = Utc::now();
        let a = pod("a", "ns-a", PodPhase::Succeeded, 1000, now);
        let b = pod("b", "ns-b", PodPhase::Succeeded, 100, now);
        let pods = vec![a.clone(), b.clone()];
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        // Single global cap of 1 → oldest "a" evicted regardless of namespace
        assert_eq!(r.collected, vec![a.uid]);
        assert_eq!(r.kept.get(&b.uid), Some(&KeepReason::UnderCap));
    }

    #[test]
    fn mixed_active_and_terminated_partitioned_correctly() {
        let now = Utc::now();
        let active = pod("active", "ns", PodPhase::Running, 0, now);
        let term = pod("term", "ns", PodPhase::Succeeded, 1000, now);
        let pods = vec![active.clone(), term.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected, vec![term.uid]);
        assert_eq!(r.kept.get(&active.uid), Some(&KeepReason::Active));
    }

    #[test]
    fn ties_in_terminated_at_break_stably_by_uid() {
        let now = Utc::now();
        let mut a = pod("a", "ns", PodPhase::Succeeded, 500, now);
        let mut b = pod("b", "ns", PodPhase::Succeeded, 500, now);
        // Force fixed UIDs to make the test deterministic.
        a.uid = Uuid::from_bytes([1u8; 16]);
        b.uid = Uuid::from_bytes([2u8; 16]);
        let pods = vec![a.clone(), b.clone()];
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        // Same age — by uid, lower uid is "older", so it should be evicted first.
        assert_eq!(r.collected.len(), 1);
        assert!(r.collected.contains(&a.uid));
    }

    #[test]
    fn large_steady_state_cap_holds() {
        let now = Utc::now();
        let mut pods = Vec::new();
        for i in 0..50 {
            pods.push(pod(&format!("p{}", i), "ns", PodPhase::Succeeded, 100 + i, now));
        }
        let cfg = PodGcConfig { max_terminated: 10, min_age: Duration::seconds(50), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected.len(), 40);
        assert_eq!(r.kept.len(), 10);
    }

    #[test]
    fn never_collects_when_all_younger_than_min_age() {
        let now = Utc::now();
        let mut pods = Vec::new();
        for i in 0..20 {
            pods.push(pod(&format!("p{}", i), "ns", PodPhase::Succeeded, i, now));
        }
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(120), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.len(), 20);
        for v in r.kept.values() {
            assert_eq!(v, &KeepReason::YoungerThanMinAge);
        }
    }

    #[test]
    fn collected_uids_are_sorted_for_stable_output() {
        let now = Utc::now();
        let mut p1 = pod("p1", "ns", PodPhase::Succeeded, 1000, now);
        let mut p2 = pod("p2", "ns", PodPhase::Succeeded, 900, now);
        p1.uid = Uuid::from_bytes([5u8; 16]);
        p2.uid = Uuid::from_bytes([3u8; 16]);
        let pods = vec![p1.clone(), p2.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        let mut sorted = r.collected.clone();
        sorted.sort();
        assert_eq!(r.collected, sorted, "output must be sorted");
    }

    #[test]
    fn failed_phase_treated_same_as_succeeded() {
        let now = Utc::now();
        let f = pod("f", "ns", PodPhase::Failed, 1000, now);
        let s = pod("s", "ns", PodPhase::Succeeded, 1100, now);
        let pods = vec![f.clone(), s.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected.len(), 2);
    }

    #[test]
    fn unknown_phase_is_active_not_collected() {
        let now = Utc::now();
        let p = pod("p", "ns", PodPhase::Unknown, 5000, now);
        let pods = vec![p.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.get(&p.uid), Some(&KeepReason::Active));
    }

    #[test]
    fn pending_phase_is_active_not_collected() {
        let now = Utc::now();
        let p = pod("p", "ns", PodPhase::Pending, 5000, now);
        let pods = vec![p.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
    }

    #[test]
    fn report_count_helpers_match_lengths() {
        let now = Utc::now();
        let active = pod("a", "ns", PodPhase::Running, 0, now);
        let evict = pod("b", "ns", PodPhase::Succeeded, 1000, now);
        let pods = vec![active.clone(), evict.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected_count(), r.collected.len());
        assert_eq!(r.kept_count(), r.kept.len());
    }

    #[test]
    fn min_age_boundary_inclusive_at_threshold() {
        let now = Utc::now();
        let p = pod("p", "ns", PodPhase::Succeeded, 60, now);
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods_vec(&[p.clone()]), &cfg, now);
        // exactly at threshold → eligible (>=)
        assert_eq!(r.collected.len(), 1);
    }

    #[test]
    fn min_age_just_below_threshold_kept() {
        let now = Utc::now();
        let p = pod("p", "ns", PodPhase::Succeeded, 59, now);
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods_vec(&[p.clone()]), &cfg, now);
        assert!(r.collected.is_empty());
    }

    fn pods_vec(slice: &[ManagedPod]) -> Vec<ManagedPod> { slice.to_vec() }

    #[test]
    fn cap_equals_count_keeps_all() {
        let now = Utc::now();
        let pods: Vec<_> = (0..3).map(|i| pod(&format!("p{}", i), "ns", PodPhase::Succeeded, 100 + i, now)).collect();
        let cfg = PodGcConfig { max_terminated: 3, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.len(), 3);
    }

    #[test]
    fn fresh_terminated_with_low_cap_keeps_via_min_age() {
        let now = Utc::now();
        // 5 fresh pods + cap 1 → all kept under YoungerThanMinAge, not collected.
        let pods: Vec<_> = (0..5).map(|i| pod(&format!("p{}", i), "ns", PodPhase::Succeeded, i as i64, now)).collect();
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        for (_, v) in r.kept.iter() {
            assert_eq!(v, &KeepReason::YoungerThanMinAge);
        }
    }

    #[test]
    fn mixed_aged_and_young_only_aged_eligible_for_cap() {
        let now = Utc::now();
        let aged_a = pod("aged_a", "ns", PodPhase::Succeeded, 1000, now);
        let aged_b = pod("aged_b", "ns", PodPhase::Succeeded, 800, now);
        let young = pod("young",  "ns", PodPhase::Succeeded, 5,    now);
        let pods = vec![aged_a.clone(), aged_b.clone(), young.clone()];
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(60), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        // Only aged_a should be evicted (oldest aged, cap 1 across aged set)
        assert_eq!(r.collected, vec![aged_a.uid]);
        assert_eq!(r.kept.get(&aged_b.uid), Some(&KeepReason::UnderCap));
        assert_eq!(r.kept.get(&young.uid), Some(&KeepReason::YoungerThanMinAge));
    }

    #[test]
    fn non_terminated_pods_are_not_counted_against_cap() {
        let now = Utc::now();
        // 5 active pods, cap=0 — none collected, none aged.
        let pods: Vec<_> = (0..5).map(|i| pod(&format!("a{}", i), "ns", PodPhase::Running, 1000, now)).collect();
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.len(), 5);
    }

    #[test]
    fn duration_zero_min_age_collects_all_terminated() {
        let now = Utc::now();
        let pods: Vec<_> = (0..3).map(|i| pod(&format!("p{}", i), "ns", PodPhase::Succeeded, i as i64, now)).collect();
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::zero(), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected.len(), 3);
    }

    #[test]
    fn report_default_is_empty() {
        let r = GcReport::default();
        assert_eq!(r.collected_count(), 0);
        assert_eq!(r.kept_count(), 0);
    }

    #[test]
    fn keep_reason_equality() {
        assert_eq!(KeepReason::Active, KeepReason::Active);
        assert_ne!(KeepReason::Active, KeepReason::UnderCap);
        assert_ne!(KeepReason::UnderCap, KeepReason::YoungerThanMinAge);
    }

    #[test]
    fn config_clone_independent() {
        let c = PodGcConfig::defaults();
        let c2 = c.clone();
        assert_eq!(c.max_terminated, c2.max_terminated);
    }

    #[test]
    fn many_namespaces_per_namespace_mode() {
        let now = Utc::now();
        let mut pods = Vec::new();
        for ns in &["a","b","c","d","e"] {
            for i in 0..3 {
                pods.push(pod(&format!("{}_{}", ns, i), ns, PodPhase::Succeeded, (1000 - i) as i64, now));
            }
        }
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(60), per_namespace: true };
        let r = plan_gc(&pods, &cfg, now);
        // Each of 5 namespaces evicts 2 pods (3 - 1 cap each) → 10 collected total.
        assert_eq!(r.collected.len(), 10);
        assert_eq!(r.kept.len(), 5);
    }

    #[test]
    fn future_terminated_at_treated_as_age_zero() {
        // If terminated_at is in the future (clock skew), age becomes negative;
        // negative < min_age, so the pod is kept as YoungerThanMinAge.
        let now = Utc::now();
        let mut p = pod("clock_skew", "ns", PodPhase::Succeeded, -100, now);
        // future startup time
        p.started_at = Some(now + Duration::seconds(100));
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(1), per_namespace: false };
        let r = plan_gc(&[p.clone()], &cfg, now);
        assert!(r.collected.is_empty());
        assert_eq!(r.kept.get(&p.uid), Some(&KeepReason::YoungerThanMinAge));
    }

    #[test]
    fn isolation_across_calls_is_pure() {
        // plan_gc is pure: identical inputs yield identical outputs.
        let now = Utc::now();
        let pods: Vec<_> = (0..5).map(|i| pod(&format!("p{}", i), "ns", PodPhase::Succeeded, 100 * (i+1) as i64, now)).collect();
        let cfg = PodGcConfig { max_terminated: 2, min_age: Duration::seconds(10), per_namespace: false };
        let r1 = plan_gc(&pods, &cfg, now);
        let r2 = plan_gc(&pods, &cfg, now);
        assert_eq!(r1.collected, r2.collected);
        assert_eq!(r1.kept.len(), r2.kept.len());
    }

    #[test]
    fn defaults_min_age_is_one_minute() {
        assert_eq!(PodGcConfig::defaults().min_age, Duration::minutes(1));
    }

    #[test]
    fn defaults_max_terminated_is_one_hundred() {
        assert_eq!(PodGcConfig::defaults().max_terminated, 100);
    }

    #[test]
    fn cap_one_keeps_newest_evicts_oldest_in_pair() {
        let now = Utc::now();
        let old = pod("old", "ns", PodPhase::Failed, 2000, now);
        let new = pod("new", "ns", PodPhase::Succeeded, 1000, now);
        let pods = vec![old.clone(), new.clone()];
        let cfg = PodGcConfig { max_terminated: 1, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected, vec![old.uid]);
        assert_eq!(r.kept.get(&new.uid), Some(&KeepReason::UnderCap));
    }

    #[test]
    fn empty_namespace_string_is_valid_bucket_key() {
        let now = Utc::now();
        let p = pod("p", "", PodPhase::Succeeded, 1000, now);
        let pods = vec![p.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: true };
        let r = plan_gc(&pods, &cfg, now);
        assert_eq!(r.collected, vec![p.uid]);
    }

    #[test]
    fn high_cap_keeps_all_aged() {
        let now = Utc::now();
        let pods: Vec<_> = (0..5).map(|i| pod(&format!("p{}", i), "ns", PodPhase::Succeeded, 100 + i, now)).collect();
        let cfg = PodGcConfig { max_terminated: 1000, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        assert!(r.collected.is_empty());
    }

    #[test]
    fn evicted_pods_excluded_from_kept_map() {
        let now = Utc::now();
        let p1 = pod("p1", "ns", PodPhase::Succeeded, 1000, now);
        let p2 = pod("p2", "ns", PodPhase::Succeeded, 900, now);
        let pods = vec![p1.clone(), p2.clone()];
        let cfg = PodGcConfig { max_terminated: 0, min_age: Duration::seconds(10), per_namespace: false };
        let r = plan_gc(&pods, &cfg, now);
        for uid in &r.collected {
            assert!(!r.kept.contains_key(uid),
                "uid {} both collected AND kept", uid);
        }
    }
}
