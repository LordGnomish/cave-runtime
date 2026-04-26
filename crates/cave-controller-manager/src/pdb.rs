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

/// Stub: scale-subresource lookup for the PDB target. Not implemented.
pub fn resolve_scale_target(_spec: &PdbSpec) -> Result<u32, ControllerError> {
    unimplemented!("PDB scale-subresource — see pkg/controller/disruption/disruption.go::getExpectedScale")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/disruption/disruption.go", "DisruptionController");

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
        let st = PdbStatus { current_healthy: 5, expected_pods: 5, disruptions_allowed: 0 };
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
        let st = PdbStatus { current_healthy: 10, expected_pods: 10, disruptions_allowed: 0 };
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
        let st = PdbStatus { current_healthy: 9, expected_pods: 10, disruptions_allowed: 0 };
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
}
