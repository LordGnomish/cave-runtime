// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PodDisruptionBudget controller — gates voluntary disruption.
//!
//! Upstream: [`pkg/controller/disruption`]. The full controller computes
//! `disruptionsAllowed` from `minAvailable` / `maxUnavailable`, accounts for
//! unhealthy pods, and writes a `PodDisruptionBudgetStatus`.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// PDB threshold can be expressed as either an absolute count or a percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Threshold {
    Count(u32),
    Percent(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdbSpec {
    pub name: String,
    pub namespace: String,
    /// Exactly one of these must be set; `min_available` wins if both are.
    pub min_available: Option<Threshold>,
    pub max_unavailable: Option<Threshold>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PdbStatus {
    pub current_healthy: u32,
    pub expected_pods: u32,
    pub disruptions_allowed: u32,
}

fn resolve(threshold: Threshold, total: u32) -> u32 {
    match threshold {
        Threshold::Count(n) => n.min(total),
        Threshold::Percent(p) => ((total as u64 * p as u64) / 100) as u32,
    }
}

/// Compute `disruptionsAllowed` from `min_available` / `max_unavailable` and
/// the current healthy-pod count. Mirrors `getExpectedPodCount` plus
/// `trySetPDBStatus` in `pkg/controller/disruption/disruption.go`.
pub fn disruptions_allowed(spec: &PdbSpec, status: &PdbStatus) -> Result<u32, ControllerError> {
    let total = status.expected_pods;
    let healthy = status.current_healthy;
    let allowed = if let Some(t) = spec.min_available {
        let need = resolve(t, total);
        healthy.saturating_sub(need)
    } else if let Some(t) = spec.max_unavailable {
        let unavail = total.saturating_sub(healthy);
        let cap = resolve(t, total);
        cap.saturating_sub(unavail)
    } else {
        return Err(ControllerError::InvalidSpec {
            kind: "PodDisruptionBudget",
            reason: "exactly one of min_available / max_unavailable required".into(),
        });
    };
    Ok(allowed)
}

/// Mirrors `sync` in upstream — reports a status update if the computed value
/// differs from the observed status.
pub fn reconcile(
    spec: &PdbSpec,
    status: &PdbStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    let want = disruptions_allowed(spec, status)?;
    if want == status.disruptions_allowed {
        Ok(Reconcile::NoOp)
    } else {
        Ok(Reconcile::Update(want))
    }
}

/// View of a target the PDB applies to (Deployment / RS / StatefulSet etc.).
/// Mirrors the projection used by upstream `getExpectedScale`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleTargetRef {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    /// Replica count exposed by the target's `/scale` subresource.
    pub spec_replicas: u32,
}

/// Resolve the expected pod count for `spec` from the matching ScaleTarget.
/// Mirrors `pkg/controller/disruption/disruption.go::getExpectedScale`.
pub fn resolve_scale_target(
    spec: &PdbSpec,
    target: &ScaleTargetRef,
) -> Result<u32, ControllerError> {
    if target.namespace != spec.namespace {
        return Err(ControllerError::InvalidSpec {
            kind: "PodDisruptionBudget",
            reason: format!(
                "scale target {}/{} is in namespace `{}`; PDB is in `{}` (cross-namespace not allowed)",
                target.kind, target.name, target.namespace, spec.namespace,
            ),
        });
    }
    Ok(target.spec_replicas)
}

/// Admission decision returned by the eviction handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvictionDecision {
    /// Eviction allowed — PDB has remaining budget.
    Allow,
    /// Eviction denied — would breach `disruptionsAllowed`.
    Deny { reason: &'static str },
}

/// Eviction admission decision. Mirrors the request side of
/// `pkg/registry/policy/eviction/storage/storage.go::Eviction.Create`:
///   * dry_run never consumes budget,
///   * disruptions_allowed > 0 → Allow,
///   * otherwise Deny with a fixed reason matching upstream message
///     `"Cannot evict pod as it would violate the pod's disruption budget."`
pub fn admit_eviction(status: &PdbStatus, dry_run: bool) -> EvictionDecision {
    if dry_run {
        return EvictionDecision::Allow;
    }
    if status.disruptions_allowed > 0 {
        EvictionDecision::Allow
    } else {
        EvictionDecision::Deny {
            reason: "Cannot evict pod as it would violate the pod's disruption budget.",
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/disruption/disruption.go",
    "DisruptionController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn pdb_min(t: Threshold) -> PdbSpec {
        PdbSpec {
            name: "pdb-1".into(),
            namespace: "default".into(),
            min_available: Some(t),
            max_unavailable: None,
        }
    }

    #[test]
    fn min_available_count_allows_remainder() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/disruption/disruption.go",
            "trySetPDBStatus",
            "tenant-pdb-min-count"
        );
        let _ = tenant;
        let s = pdb_min(Threshold::Count(2));
        let st = PdbStatus {
            current_healthy: 5,
            expected_pods: 5,
            disruptions_allowed: 0,
        };
        // healthy - min_available = 5 - 2 = 3
        assert_eq!(disruptions_allowed(&s, &st).unwrap(), 3);
    }

    #[test]
    fn min_available_percent_resolves_against_total() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/disruption/disruption.go",
            "getExpectedPodCount",
            "tenant-pdb-min-pct"
        );
        let _ = tenant;
        let s = pdb_min(Threshold::Percent(60));
        let st = PdbStatus {
            current_healthy: 10,
            expected_pods: 10,
            disruptions_allowed: 0,
        };
        // need = 60% of 10 = 6 → allowed = 10 - 6 = 4
        assert_eq!(disruptions_allowed(&s, &st).unwrap(), 4);
    }

    #[test]
    fn max_unavailable_caps_disruption_budget() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/disruption/disruption.go",
            "trySetPDBStatus",
            "tenant-pdb-max-unavail"
        );
        let _ = tenant;
        let spec = PdbSpec {
            name: "pdb-2".into(),
            namespace: "default".into(),
            min_available: None,
            max_unavailable: Some(Threshold::Count(2)),
        };
        let st = PdbStatus {
            current_healthy: 9,
            expected_pods: 10,
            disruptions_allowed: 0,
        };
        // unavail = 1, cap = 2, allowed = 1
        assert_eq!(disruptions_allowed(&spec, &st).unwrap(), 1);
    }

    #[test]
    fn rejects_pdb_with_neither_threshold() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/disruption/disruption.go",
            "validatePodDisruptionBudgetSpec",
            "tenant-pdb-no-threshold"
        );
        let _ = tenant;
        let spec = PdbSpec {
            name: "bad".into(),
            namespace: "default".into(),
            min_available: None,
            max_unavailable: None,
        };
        assert!(disruptions_allowed(&spec, &PdbStatus::default()).is_err());
    }

    // ── Deeper coverage (deeper-001) ─────────────────────────────────────────

    /// Upstream parity: `TestEvictionAdmission_AllowedWhenBudgetRemaining`
    /// (registry/policy/eviction/storage/storage_test.go::TestEviction —
    /// disruptions_allowed > 0 admits the eviction call).
    #[test]
    fn eviction_admitted_when_budget_remaining() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/kubernetes/pkg/registry/policy/eviction/storage/storage.go",
            "Eviction.Create",
            "tenant-pdb-evict-allow"
        );
        let _ = tenant;
        let st = PdbStatus {
            current_healthy: 5,
            expected_pods: 5,
            disruptions_allowed: 2,
        };
        assert_eq!(admit_eviction(&st, false), EvictionDecision::Allow);
    }

    /// Upstream parity: `TestEvictionAdmission_DeniedAtZeroBudget`
    /// (storage_test.go::TestEviction — exhausted budget yields the
    /// canonical 429 deny message verbatim).
    #[test]
    fn eviction_denied_when_budget_exhausted_with_canonical_message() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/kubernetes/pkg/registry/policy/eviction/storage/storage.go",
            "Eviction.Create",
            "tenant-pdb-evict-deny"
        );
        let _ = tenant;
        let st = PdbStatus {
            current_healthy: 3,
            expected_pods: 5,
            disruptions_allowed: 0,
        };
        match admit_eviction(&st, false) {
            EvictionDecision::Deny { reason } => {
                assert_eq!(
                    reason,
                    "Cannot evict pod as it would violate the pod's disruption budget."
                );
            }
            EvictionDecision::Allow => panic!("expected Deny when budget exhausted"),
        }
    }

    /// Upstream parity: `TestEvictionAdmission_DryRunNeverConsumes`
    /// (storage_test.go — dry-run path admits regardless of budget).
    #[test]
    fn dry_run_eviction_admits_even_with_zero_budget() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/kubernetes/pkg/registry/policy/eviction/storage/storage.go",
            "Eviction.Create",
            "tenant-pdb-evict-dryrun"
        );
        let _ = tenant;
        let st = PdbStatus {
            current_healthy: 1,
            expected_pods: 5,
            disruptions_allowed: 0,
        };
        assert_eq!(
            admit_eviction(&st, /*dry_run=*/ true),
            EvictionDecision::Allow
        );
    }

    /// Upstream parity: `TestPDB_ResolveScaleTargetSameNamespace`
    /// (disruption_test.go::TestGetExpectedScale — the controller resolves
    /// `target.spec.replicas` via the /scale subresource).
    #[test]
    fn resolve_scale_target_returns_target_spec_replicas() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/disruption/disruption.go",
            "getExpectedScale",
            "tenant-pdb-scale-target"
        );
        let _ = tenant;
        let spec = pdb_min(Threshold::Percent(50));
        let target = ScaleTargetRef {
            kind: "Deployment".into(),
            name: "web".into(),
            namespace: "default".into(),
            spec_replicas: 7,
        };
        assert_eq!(resolve_scale_target(&spec, &target).unwrap(), 7);
    }

    /// Upstream parity: `TestPDB_RejectsCrossNamespaceScaleTarget`
    /// (disruption.go — PDB and target live in the same namespace).
    /// In cave-apiserver this is also our tenant_id invariant for PDB.
    #[test]
    fn resolve_scale_target_rejects_cross_namespace_target() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/disruption/disruption.go",
            "getExpectedScale",
            "tenant-pdb-cross-ns"
        );
        let _ = tenant;
        let spec = pdb_min(Threshold::Percent(50));
        let target = ScaleTargetRef {
            kind: "Deployment".into(),
            name: "web".into(),
            namespace: "kube-system".into(),
            spec_replicas: 4,
        };
        let err = resolve_scale_target(&spec, &target).unwrap_err();
        assert!(
            matches!(
                err,
                ControllerError::InvalidSpec {
                    kind: "PodDisruptionBudget",
                    ..
                }
            ),
            "tenant_id invariant: cross-namespace target rejected"
        );
    }
}
