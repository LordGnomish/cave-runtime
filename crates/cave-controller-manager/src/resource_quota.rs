// SPDX-License-Identifier: AGPL-3.0-or-later
//! ResourceQuota controller — `pkg/controller/resourcequota/resource_quota_controller.go`.
//!
//! Maintains `ResourceQuota.status.used` by aggregating usage of
//! tracked resources across the namespace. The admission plugin gates
//! admission against `status.hard - status.used`; the controller's job
//! is to keep `status.used` accurate after every object create / delete /
//! update.

use crate::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Quantities are tracked in arbitrary integer units (cores * 1000 for cpu,
/// bytes for memory, count for object counts).
pub type Quantity = i64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuotaSpec {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub hard: BTreeMap<String, Quantity>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceQuotaStatus {
    pub used: BTreeMap<String, Quantity>,
}

/// One usage observation produced by an object event handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageDelta {
    pub resource: String,
    pub delta: Quantity,
}

/// Apply a series of usage deltas to a Status, clamping at zero
/// (used should never go negative — that would indicate a bookkeeping bug
/// or a resource the controller didn't see being created).
pub fn apply_deltas(status: &mut ResourceQuotaStatus, deltas: &[UsageDelta]) {
    for d in deltas {
        let entry = status.used.entry(d.resource.clone()).or_insert(0);
        *entry += d.delta;
        if *entry < 0 {
            *entry = 0;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionDecision {
    Admit,
    Deny(String),
}

/// Decide whether a proposed `usage` increment is admissible against the quota.
/// Mirrors `pkg/quota/v1/evaluator.go::Mask`.
pub fn admit(
    spec: &ResourceQuotaSpec,
    status: &ResourceQuotaStatus,
    proposed: &[UsageDelta],
) -> AdmissionDecision {
    for d in proposed {
        let used = status.used.get(&d.resource).copied().unwrap_or(0);
        let want = used + d.delta;
        if let Some(hard) = spec.hard.get(&d.resource) {
            if want > *hard {
                return AdmissionDecision::Deny(format!(
                    "{} request would exceed quota {}/{} ({} → {})",
                    d.resource, used, hard, used, want
                ));
            }
        }
    }
    AdmissionDecision::Admit
}

/// Compute the diff status the controller should write back. Returns
/// `Some(status)` only when the recomputed `used` map differs from the
/// observed status; otherwise `None` (no patch needed).
pub fn reconcile(
    observed_status: &ResourceQuotaStatus,
    deltas: &[UsageDelta],
) -> Option<ResourceQuotaStatus> {
    let mut next = observed_status.clone();
    apply_deltas(&mut next, deltas);
    if next.used == observed_status.used {
        None
    } else {
        Some(next)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/resourcequota/resource_quota_controller.go",
    "ResourceQuotaController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn spec(hard: &[(&str, i64)]) -> ResourceQuotaSpec {
        ResourceQuotaSpec {
            name: "q".into(),
            namespace: "default".into(),
            tenant: TenantId::new("t").expect("test fixture"),
            hard: hard.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            scopes: vec![],
        }
    }
    fn status(used: &[(&str, i64)]) -> ResourceQuotaStatus {
        ResourceQuotaStatus {
            used: used.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }
    fn d(r: &str, v: i64) -> UsageDelta {
        UsageDelta { resource: r.into(), delta: v }
    }

    #[test]
    fn apply_deltas_increments_used() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/resourcequota/resource_quota_controller.go",
            "syncResourceQuota",
            "tenant-rq-apply"
        );
        let mut s = status(&[("cpu", 1000)]);
        apply_deltas(&mut s, &[d("cpu", 500), d("memory", 1024)]);
        assert_eq!(s.used.get("cpu"), Some(&1500));
        assert_eq!(s.used.get("memory"), Some(&1024));
    }

    #[test]
    fn apply_deltas_clamps_at_zero() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/resourcequota/resource_quota_controller.go",
            "syncResourceQuota",
            "tenant-rq-clamp"
        );
        let mut s = status(&[("cpu", 100)]);
        apply_deltas(&mut s, &[d("cpu", -500)]);
        assert_eq!(s.used.get("cpu"), Some(&0));
    }

    #[test]
    fn admit_when_under_quota() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/quota/v1/evaluator.go",
            "Mask",
            "tenant-rq-admit-ok"
        );
        let sp = spec(&[("cpu", 4000)]);
        let st = status(&[("cpu", 1000)]);
        assert_eq!(admit(&sp, &st, &[d("cpu", 500)]), AdmissionDecision::Admit);
    }

    #[test]
    fn deny_when_would_exceed_quota() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/quota/v1/evaluator.go",
            "Mask",
            "tenant-rq-admit-deny"
        );
        let sp = spec(&[("cpu", 4000)]);
        let st = status(&[("cpu", 3500)]);
        match admit(&sp, &st, &[d("cpu", 1000)]) {
            AdmissionDecision::Deny(_) => {}
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn admit_when_resource_not_tracked_by_quota() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/quota/v1/evaluator.go",
            "Mask",
            "tenant-rq-admit-untracked"
        );
        let sp = spec(&[("cpu", 4000)]);
        let st = status(&[]);
        // memory not in hard → not enforced.
        assert_eq!(
            admit(&sp, &st, &[d("memory", 100_000_000)]),
            AdmissionDecision::Admit
        );
    }

    #[test]
    fn reconcile_returns_none_when_no_change() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/resourcequota/resource_quota_controller.go",
            "syncResourceQuota",
            "tenant-rq-rec-noop"
        );
        let st = status(&[("cpu", 1000)]);
        assert!(reconcile(&st, &[]).is_none());
    }

    #[test]
    fn reconcile_returns_some_when_used_changed() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/resourcequota/resource_quota_controller.go",
            "syncResourceQuota",
            "tenant-rq-rec-changed"
        );
        let st = status(&[("cpu", 1000)]);
        let next = reconcile(&st, &[d("cpu", 200)]).unwrap();
        assert_eq!(next.used.get("cpu"), Some(&1200));
    }

    #[test]
    fn admit_evaluates_every_delta_independently() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/quota/v1/evaluator.go",
            "Mask",
            "tenant-rq-admit-multi"
        );
        let sp = spec(&[("cpu", 4000), ("memory", 8 * 1024 * 1024)]);
        let st = status(&[("cpu", 1000), ("memory", 1024)]);
        let prop = vec![d("cpu", 500), d("memory", 1024)];
        assert_eq!(admit(&sp, &st, &prop), AdmissionDecision::Admit);
    }

    #[test]
    fn admit_first_failing_resource_short_circuits_deny() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/quota/v1/evaluator.go",
            "Mask",
            "tenant-rq-admit-multi-fail"
        );
        let sp = spec(&[("cpu", 1000), ("pods", 5)]);
        let st = status(&[("cpu", 800), ("pods", 5)]);
        // cpu would exceed (800+500>1000) AND pods would exceed (5+1>5).
        match admit(&sp, &st, &[d("cpu", 500), d("pods", 1)]) {
            AdmissionDecision::Deny(reason) => {
                assert!(reason.contains("cpu"));
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn admission_decision_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/resourcequota/resource_quota_controller.go",
            "AdmissionDecision",
            "tenant-rq-decision-serde"
        );
        for d in [
            AdmissionDecision::Admit,
            AdmissionDecision::Deny("x".into()),
        ] {
            let s = serde_json::to_string(&d).unwrap();
            let back: AdmissionDecision = serde_json::from_str(&s).unwrap();
            assert_eq!(d, back);
        }
    }

    #[test]
    fn used_map_round_trips_serde() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "ResourceQuotaStatus",
            "tenant-rq-used-serde"
        );
        let st = status(&[("cpu", 1000), ("memory", 2048)]);
        let s = serde_json::to_string(&st).unwrap();
        let back: ResourceQuotaStatus = serde_json::from_str(&s).unwrap();
        assert_eq!(st.used, back.used);
    }
}
