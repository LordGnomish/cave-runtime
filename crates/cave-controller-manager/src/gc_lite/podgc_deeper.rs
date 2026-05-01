//! PodGC deeper — `pkg/controller/podgc/gc_controller.go`.
//!
//! Beyond the M3 baseline (terminated-threshold + orphaned), this module
//! implements:
//!
//! * `markPodsTerminating` — pods whose binding node entered NoExecute
//!   are added a `DisruptionTarget` condition.
//! * Stuck-Terminating cleanup — pods with `deletionTimestamp` older than
//!   `gracePeriod * 2` are force-removed (KEP-3329).
//! * Out-of-service taint reaction — pods on `out-of-service` nodes are
//!   force-deleted with `gracePeriod = 0`.

use super::podgc::{PodPhase, PodSummary};
use crate::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisruptionReason {
    /// Pod was on a NotReady node that's now being evicted.
    NodeNotReady,
    /// Pod was on a node tainted out-of-service.
    OutOfService,
    /// Pod is past gracePeriod but still alive (kubelet stuck).
    Stuck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisruptedPod {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub reason: DisruptionReason,
    pub force_delete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodNodeView {
    pub pod: PodSummary,
    /// Set when the pod has `metadata.deletionTimestamp`.
    pub deletion_timestamp_sec: Option<u64>,
    /// `metadata.deletionGracePeriodSeconds` — defaults to 30s upstream.
    pub grace_period_sec: u32,
    /// True when the bound node has the `node.kubernetes.io/out-of-service`
    /// NoExecute taint.
    pub on_out_of_service: bool,
    /// True when the bound node is NotReady AND past the eviction timeout.
    pub on_not_ready_evicting: bool,
}

/// Detect pods that need a `DisruptionTarget` condition + force-delete.
/// Mirrors `markPodsTerminating` and `gcOrphaned` extension.
pub fn select_disrupted(views: &[PodNodeView], now_sec: u64) -> Vec<DisruptedPod> {
    let mut out = Vec::new();
    for v in views {
        if v.on_out_of_service {
            out.push(DisruptedPod {
                name: v.pod.name.clone(),
                namespace: v.pod.namespace.clone(),
                tenant: v.pod.tenant.clone(),
                reason: DisruptionReason::OutOfService,
                force_delete: true,
            });
            continue;
        }
        if v.on_not_ready_evicting {
            out.push(DisruptedPod {
                name: v.pod.name.clone(),
                namespace: v.pod.namespace.clone(),
                tenant: v.pod.tenant.clone(),
                reason: DisruptionReason::NodeNotReady,
                force_delete: false,
            });
            continue;
        }
        if let Some(dt) = v.deletion_timestamp_sec {
            // Stuck once age > 2 * gracePeriod.
            if now_sec >= dt + 2 * v.grace_period_sec as u64 {
                out.push(DisruptedPod {
                    name: v.pod.name.clone(),
                    namespace: v.pod.namespace.clone(),
                    tenant: v.pod.tenant.clone(),
                    reason: DisruptionReason::Stuck,
                    force_delete: true,
                });
            }
        }
    }
    out
}

/// True when the pod is in a terminal-eligible phase OR has been marked
/// for deletion (so PodGC may still see it after kubelet acknowledged
/// the delete).
pub fn is_collectible(view: &PodNodeView) -> bool {
    matches!(view.pod.phase, PodPhase::Succeeded | PodPhase::Failed | PodPhase::Unknown)
        || view.deletion_timestamp_sec.is_some()
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/podgc/gc_controller.go",
    "PodGCController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn p(name: &str, phase: PodPhase) -> PodSummary {
        PodSummary {
            name: name.into(),
            namespace: "default".into(),
            tenant: TenantId::new("t1").expect("test fixture"),
            phase,
            created_sec: 0,
            node_name: Some("n1".into()),
            orphaned: false,
        }
    }
    fn v(
        pod: PodSummary,
        dt: Option<u64>,
        grace: u32,
        oos: bool,
        not_ready: bool,
    ) -> PodNodeView {
        PodNodeView {
            pod,
            deletion_timestamp_sec: dt,
            grace_period_sec: grace,
            on_out_of_service: oos,
            on_not_ready_evicting: not_ready,
        }
    }

    #[test]
    fn out_of_service_node_force_deletes_pods() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "outOfServicePods",
            "tenant-podgc2-oos"
        );
        let view = v(p("a", PodPhase::Running), None, 30, true, false);
        let got = select_disrupted(&[view], 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].reason, DisruptionReason::OutOfService);
        assert!(got[0].force_delete);
    }

    #[test]
    fn not_ready_evicting_node_marks_disrupted_no_force() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "markPodsTerminating",
            "tenant-podgc2-not-ready"
        );
        let view = v(p("a", PodPhase::Running), None, 30, false, true);
        let got = select_disrupted(&[view], 0);
        assert_eq!(got[0].reason, DisruptionReason::NodeNotReady);
        assert!(!got[0].force_delete);
    }

    #[test]
    fn stuck_terminating_after_double_grace_force_deletes() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcStuckPods",
            "tenant-podgc2-stuck"
        );
        // dt=100, grace=30 → stuck once now >= 100 + 60 = 160.
        let view = v(p("a", PodPhase::Running), Some(100), 30, false, false);
        let got = select_disrupted(&[view], 160);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].reason, DisruptionReason::Stuck);
        assert!(got[0].force_delete);
    }

    #[test]
    fn within_double_grace_is_not_stuck_yet() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcStuckPods",
            "tenant-podgc2-not-stuck"
        );
        let view = v(p("a", PodPhase::Running), Some(100), 30, false, false);
        let got = select_disrupted(&[view], 159);
        assert!(got.is_empty());
    }

    #[test]
    fn no_deletion_timestamp_is_not_stuck() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "gcStuckPods",
            "tenant-podgc2-no-dt"
        );
        let view = v(p("a", PodPhase::Running), None, 30, false, false);
        let got = select_disrupted(&[view], 99999);
        assert!(got.is_empty());
    }

    #[test]
    fn out_of_service_takes_precedence_over_stuck() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "markPodsTerminating",
            "tenant-podgc2-oos-precedence"
        );
        let view = v(p("a", PodPhase::Running), Some(100), 30, true, false);
        let got = select_disrupted(&[view], 9999);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].reason, DisruptionReason::OutOfService);
    }

    #[test]
    fn out_of_service_takes_precedence_over_not_ready() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "markPodsTerminating",
            "tenant-podgc2-oos-vs-not-ready"
        );
        let view = v(p("a", PodPhase::Running), None, 30, true, true);
        let got = select_disrupted(&[view], 0);
        assert_eq!(got[0].reason, DisruptionReason::OutOfService);
    }

    #[test]
    fn collectible_includes_deletion_timestamp_pods() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "isPodTerminated",
            "tenant-podgc2-collect-dt"
        );
        let view = v(p("a", PodPhase::Running), Some(50), 30, false, false);
        assert!(is_collectible(&view));
    }

    #[test]
    fn collectible_includes_unknown_phase() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "isPodTerminated",
            "tenant-podgc2-collect-unknown"
        );
        let view = v(p("a", PodPhase::Unknown), None, 30, false, false);
        assert!(is_collectible(&view));
    }

    #[test]
    fn disrupted_pod_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/podgc/gc_controller.go",
            "DisruptedPod",
            "tenant-podgc2-serde"
        );
        let d = DisruptedPod {
            name: "p".into(),
            namespace: "default".into(),
            tenant: TenantId::new("t1").expect("test fixture"),
            reason: DisruptionReason::Stuck,
            force_delete: true,
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: DisruptedPod = serde_json::from_str(&s).unwrap();
        assert_eq!(d.name, back.name);
        assert_eq!(d.reason, back.reason);
    }
}
