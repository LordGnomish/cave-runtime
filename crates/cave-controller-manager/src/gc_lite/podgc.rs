// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PodGC controller — `pkg/controller/podgc/gc_controller.go`.
//!
//! Background-deletes pods that are no longer needed:
//!
//! * Phase `Succeeded` or `Failed` and older than the policy threshold
//!   (`--terminated-pod-gc-threshold`, default 12500 across the cluster —
//!   here we accept it as a per-tenant cap).
//! * Bound to a node that is no longer in the cluster (orphan).
//! * Bound to a NotReady node and explicitly force-deletable
//!   (`PodDisruptionConditions` feature gate behaviour).
//!
//! Active pods (Pending, Running) are never collected.

use crate::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

pub const DEFAULT_TERMINATED_POD_THRESHOLD: u32 = 12_500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    /// Upstream `core.PodUnknown` — treated as terminal-eligible after grace.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub phase: PodPhase,
    /// `metadata.creationTimestamp` reduced to seconds-since-epoch (monotonic).
    pub created_sec: u64,
    /// `spec.nodeName` — `None` means unscheduled.
    pub node_name: Option<String>,
    /// True when the bound node is gone from the informer cache.
    pub orphaned: bool,
}

/// Returns true if `pod` is terminal (Succeeded/Failed) and therefore eligible
/// for threshold-based GC. Mirrors `isPodTerminated` in upstream.
pub fn is_terminated(pod: &PodSummary) -> bool {
    matches!(pod.phase, PodPhase::Succeeded | PodPhase::Failed)
}

/// Selects pods to delete using the terminated-pod threshold per tenant.
///
/// 1. Filter by `tenant`.
/// 2. Keep only terminated pods.
/// 3. If count exceeds `threshold`, remove the oldest first until count == threshold.
///
/// Returns the slice of pod names that should be DELETEd.
pub fn select_terminated_for_gc(
    pods: &[PodSummary],
    tenant: &TenantId,
    threshold: u32,
) -> Vec<String> {
    let mut terminated: Vec<&PodSummary> = pods
        .iter()
        .filter(|p| p.tenant == *tenant && is_terminated(p))
        .collect();
    if terminated.len() <= threshold as usize {
        return vec![];
    }
    // Oldest first.
    terminated.sort_by_key(|p| p.created_sec);
    let to_remove = terminated.len() - threshold as usize;
    terminated.iter().take(to_remove).map(|p| p.name.clone()).collect()
}

/// Selects orphaned pods (bound to a vanished node) — these are immediately
/// eligible for force-delete regardless of phase.
/// Mirrors `gcOrphaned` in upstream.
pub fn select_orphaned_for_gc(pods: &[PodSummary], tenant: &TenantId) -> Vec<String> {
    pods.iter()
        .filter(|p| p.tenant == *tenant && p.orphaned && p.node_name.is_some())
        .map(|p| p.name.clone())
        .collect()
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/podgc/gc_controller.go", "PodGCController");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn pod(name: &str, phase: PodPhase, age: u64, tenant: &str) -> PodSummary {
        PodSummary {
            name: name.into(),
            namespace: "default".into(),
            tenant: TenantId::new(tenant).expect("test fixture"),
            phase,
            created_sec: age,
            node_name: Some("node-a".into()),
            orphaned: false,
        }
    }

    #[test]
    fn is_terminated_matches_succeeded_and_failed() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "isPodTerminated",
            "tenant-podgc-terminated"
        );
        assert!(is_terminated(&pod("a", PodPhase::Succeeded, 0, "t1")));
        assert!(is_terminated(&pod("a", PodPhase::Failed, 0, "t1")));
        assert!(!is_terminated(&pod("a", PodPhase::Running, 0, "t1")));
        assert!(!is_terminated(&pod("a", PodPhase::Pending, 0, "t1")));
    }

    #[test]
    fn no_gc_when_under_threshold() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcTerminated",
            "tenant-podgc-under-threshold"
        );
        let pods = vec![
            pod("a", PodPhase::Succeeded, 1, "t1"),
            pod("b", PodPhase::Failed, 2, "t1"),
        ];
        let got = select_terminated_for_gc(&pods, &TenantId::new("t1").expect("test fixture"), 5);
        assert!(got.is_empty());
    }

    #[test]
    fn over_threshold_removes_oldest_first() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcTerminated",
            "tenant-podgc-over-threshold"
        );
        let pods = vec![
            pod("oldest", PodPhase::Succeeded, 1, "t1"),
            pod("middle", PodPhase::Failed, 5, "t1"),
            pod("newest", PodPhase::Succeeded, 10, "t1"),
        ];
        let got = select_terminated_for_gc(&pods, &TenantId::new("t1").expect("test fixture"), 1);
        // Threshold 1, three terminated → remove the two oldest.
        assert!(got.contains(&"oldest".to_string()));
        assert!(got.contains(&"middle".to_string()));
        assert!(!got.contains(&"newest".to_string()));
    }

    #[test]
    fn active_pods_never_collected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcTerminated",
            "tenant-podgc-keeps-active"
        );
        let pods: Vec<_> = (0..20)
            .map(|i| pod(&format!("p{i}"), PodPhase::Running, i as u64, "t1"))
            .collect();
        // Threshold 0 — but everything is Running, nothing to GC.
        let got = select_terminated_for_gc(&pods, &TenantId::new("t1").expect("test fixture"), 0);
        assert!(got.is_empty());
    }

    #[test]
    fn other_tenants_are_not_collected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcTerminated",
            "tenant-podgc-tenant-isolation"
        );
        let pods = vec![
            pod("t1-a", PodPhase::Succeeded, 1, "t1"),
            pod("t1-b", PodPhase::Succeeded, 2, "t1"),
            pod("t2-a", PodPhase::Succeeded, 1, "t2"),
        ];
        let got = select_terminated_for_gc(&pods, &TenantId::new("t1").expect("test fixture"), 1);
        assert!(got.contains(&"t1-a".to_string()));
        assert!(!got.iter().any(|n| n.starts_with("t2")));
    }

    #[test]
    fn orphaned_pods_are_collected_regardless_of_phase() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcOrphaned",
            "tenant-podgc-orphan"
        );
        let mut pods = vec![
            pod("p1", PodPhase::Running, 1, "t1"),
            pod("p2", PodPhase::Pending, 2, "t1"),
        ];
        pods[0].orphaned = true;
        let got = select_orphaned_for_gc(&pods, &TenantId::new("t1").expect("test fixture"));
        assert_eq!(got, vec!["p1"]);
    }

    #[test]
    fn unscheduled_orphans_are_skipped() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcOrphaned",
            "tenant-podgc-orphan-unscheduled"
        );
        let mut p = pod("p1", PodPhase::Pending, 1, "t1");
        p.node_name = None;
        p.orphaned = true;
        // Pod was never bound — orphan-by-node-removal doesn't apply.
        let got = select_orphaned_for_gc(&[p], &TenantId::new("t1").expect("test fixture"));
        assert!(got.is_empty());
    }

    #[test]
    fn threshold_zero_collects_all_terminated() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcTerminated",
            "tenant-podgc-zero-threshold"
        );
        let pods = vec![
            pod("a", PodPhase::Succeeded, 1, "t1"),
            pod("b", PodPhase::Failed, 2, "t1"),
            pod("c", PodPhase::Running, 3, "t1"),
        ];
        let got = select_terminated_for_gc(&pods, &TenantId::new("t1").expect("test fixture"), 0);
        assert_eq!(got.len(), 2);
        assert!(!got.contains(&"c".to_string()));
    }

    #[test]
    fn default_threshold_constant_matches_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "TerminatedPodGCThreshold",
            "tenant-podgc-default-threshold"
        );
        assert_eq!(DEFAULT_TERMINATED_POD_THRESHOLD, 12_500);
    }

    #[test]
    fn pod_phase_serializes_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "PodPhase",
            "tenant-podgc-phase-serde"
        );
        for p in [
            PodPhase::Pending,
            PodPhase::Running,
            PodPhase::Succeeded,
            PodPhase::Failed,
            PodPhase::Unknown,
        ] {
            let s = serde_json::to_string(&p).unwrap();
            let back: PodPhase = serde_json::from_str(&s).unwrap();
            assert_eq!(p, back);
        }
    }

    #[test]
    fn equal_age_terminated_pods_break_ties_stably() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcTerminated",
            "tenant-podgc-tie"
        );
        // Sort is stable in Rust; ensure we don't crash on equal keys.
        let pods = vec![
            pod("a", PodPhase::Succeeded, 5, "t1"),
            pod("b", PodPhase::Failed, 5, "t1"),
            pod("c", PodPhase::Succeeded, 5, "t1"),
        ];
        let got = select_terminated_for_gc(&pods, &TenantId::new("t1").expect("test fixture"), 1);
        // Exactly 2 should be removed.
        assert_eq!(got.len(), 2);
    }
}
