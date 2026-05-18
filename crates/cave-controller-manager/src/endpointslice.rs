// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! EndpointSlice controller — projects ready pods into one or more
//! `discovery.k8s.io/v1.EndpointSlice` objects.
//!
//! Upstream: [`pkg/controller/endpointslice`]. The full controller chunks
//! endpoints across slices, manages slice ownership labels, and reconciles
//! topology hints. This module owns the chunking arithmetic and the
//! topology-hint *entry point*; the actual hint algorithm lives in
//! [`crate::endpointslice_topology`] and is delegated to from
//! [`place_topology_hints`].

use crate::endpointslice_topology::{
    compute_hints, ReadyEndpoint, TopologyDecision, ZoneInfo, MIN_ENDPOINTS_PER_ZONE,
};
use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// Maximum endpoints per slice in upstream v1.36 (constant
/// `MaxEndpointsPerSlice` in `pkg/controller/endpointslice/util/utils.go`).
pub const MAX_ENDPOINTS_PER_SLICE: u32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointSliceSpec {
    pub service: String,
    pub namespace: String,
    /// Selector keys used to pick backing pods.
    pub selector: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointObservation {
    pub ready_pod_count: u32,
    pub current_slice_count: u32,
}

/// `Service.spec.trafficDistribution` (Kubernetes v1.31+). Mirrors the
/// upstream enum exactly: any value the controller doesn't understand
/// disengages the topology algorithm.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrafficDistribution {
    /// Default — no topology preference; route to any zone.
    #[default]
    Default,
    /// Prefer endpoints in the same zone as the caller.
    PreferClose,
}

/// Returns the number of slices required to cover `pods` endpoints. Mirrors
/// the chunking logic in `numEndpointsAndSlices` upstream.
pub fn slice_count_for(pods: u32) -> u32 {
    if pods == 0 {
        0
    } else {
        pods.div_ceil(MAX_ENDPOINTS_PER_SLICE)
    }
}

/// Mirrors `syncService` — issues create/delete/no-op for the slice set.
pub fn reconcile(
    spec: &EndpointSliceSpec,
    obs: &EndpointObservation,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if spec.selector.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "EndpointSlice",
            reason: "selector must not be empty".into(),
        });
    }
    let want = slice_count_for(obs.ready_pod_count);
    use std::cmp::Ordering;
    Ok(match want.cmp(&obs.current_slice_count) {
        Ordering::Equal => Reconcile::NoOp,
        Ordering::Greater => Reconcile::Create(want - obs.current_slice_count),
        Ordering::Less => Reconcile::Delete(obs.current_slice_count - want),
    })
}

/// Top-level entry point: drive topology-aware hint placement for a given
/// EndpointSlice + its observed ready endpoints + the cluster's zone-CPU
/// profile.
///
/// Behaviour matches upstream `pkg/controller/endpointslice/topologycache`:
///
/// * `TrafficDistribution::Default` ⇒ algorithm disabled, hints stripped
///   (`Disabled("trafficDistribution=Default")`).
/// * `TrafficDistribution::PreferClose` ⇒ delegate to [`compute_hints`] with
///   the upstream `MinEndpointsPerZone = 7` threshold.
///
/// The selector must not be empty — same invariant the chunking
/// [`reconcile`] enforces — because hint placement without a selector means
/// the controller has no pods to project.
pub fn place_topology_hints(
    _spec: &EndpointSliceSpec,
    _endpoints: &[ReadyEndpoint],
    _zones: &[ZoneInfo],
    _distribution: TrafficDistribution,
) -> Result<TopologyDecision, ControllerError> {
    unimplemented!("Topology-aware hints — see pkg/controller/endpointslice/topologycache")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/endpointslice/endpointslice_controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn slice(selector: Vec<(&str, &str)>) -> EndpointSliceSpec {
        EndpointSliceSpec {
            service: "web".into(),
            namespace: "default".into(),
            selector: selector
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn ep(addr: &str, zone: &str) -> ReadyEndpoint {
        ReadyEndpoint { address: addr.into(), zone: zone.into(), ready: true }
    }

    fn zinfo(name: &str, cpu: u64) -> ZoneInfo {
        ZoneInfo { name: name.into(), cpu_milli: cpu }
    }

    #[test]
    fn slice_count_rounds_up() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "numEndpointsAndSlices",
            "tenant-eps-chunking"
        );
        let _ = tenant;
        assert_eq!(slice_count_for(0), 0);
        assert_eq!(slice_count_for(1), 1);
        assert_eq!(slice_count_for(MAX_ENDPOINTS_PER_SLICE), 1);
        assert_eq!(slice_count_for(MAX_ENDPOINTS_PER_SLICE + 1), 2);
        assert_eq!(slice_count_for(MAX_ENDPOINTS_PER_SLICE * 3), 3);
    }

    #[test]
    fn empty_selector_is_rejected() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "syncService",
            "tenant-eps-bad-selector"
        );
        let s = slice(vec![]);
        let obs = EndpointObservation::default();
        assert!(reconcile(&s, &obs, &tenant).is_err());
    }

    #[test]
    fn creates_additional_slice_when_pod_count_exceeds_chunk() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "syncService",
            "tenant-eps-grow"
        );
        let s = slice(vec![("app", "web")]);
        let obs = EndpointObservation { ready_pod_count: 250, current_slice_count: 1 };
        // need ceil(250 / 100) = 3, have 1 → create 2
        assert_eq!(reconcile(&s, &obs, &tenant).unwrap(), Reconcile::Create(2));
    }

    #[test]
    fn deletes_excess_slices_when_pods_drain() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/reconciler.go",
            "deleteEndpointSlices",
            "tenant-eps-drain"
        );
        let s = slice(vec![("app", "web")]);
        let obs = EndpointObservation { ready_pod_count: 0, current_slice_count: 3 };
        assert_eq!(reconcile(&s, &obs, &tenant).unwrap(), Reconcile::Delete(3));
    }

    // ── place_topology_hints ────────────────────────────────────────────────

    #[test]
    fn topology_hints_disabled_when_distribution_is_default() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-eps-topo-default"
        );
        let s = slice(vec![("app", "web")]);
        let dec = place_topology_hints(&s, &[], &[], TrafficDistribution::Default).unwrap();
        match dec {
            TopologyDecision::Disabled(reason) => {
                assert!(reason.contains("Default"));
            }
            other => panic!("expected Disabled, got {other:?}"),
        }
    }

    #[test]
    fn topology_hints_engaged_with_prefer_close_and_quorum() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/topologycache/topologycache.go",
            "redistributeHintsByZone",
            "tenant-eps-topo-prefer-close"
        );
        let s = slice(vec![("app", "web")]);
        let mut eps = Vec::new();
        for i in 0..MIN_ENDPOINTS_PER_ZONE {
            eps.push(ep(&format!("a{i}"), "z1"));
            eps.push(ep(&format!("b{i}"), "z2"));
        }
        let zones = vec![zinfo("z1", 1000), zinfo("z2", 1000)];
        let dec =
            place_topology_hints(&s, &eps, &zones, TrafficDistribution::PreferClose).unwrap();
        match dec {
            TopologyDecision::Engaged(hints) => {
                assert_eq!(hints.len(), (MIN_ENDPOINTS_PER_ZONE * 2) as usize);
            }
            other => panic!("expected Engaged, got {other:?}"),
        }
    }

    #[test]
    fn topology_hints_disabled_when_endpoints_below_threshold() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/topologycache/topologycache.go",
            "AddHints",
            "tenant-eps-topo-too-few"
        );
        let s = slice(vec![("app", "web")]);
        let eps = vec![ep("a", "z1"), ep("b", "z2")];
        let zones = vec![zinfo("z1", 1000), zinfo("z2", 1000)];
        let dec =
            place_topology_hints(&s, &eps, &zones, TrafficDistribution::PreferClose).unwrap();
        assert!(matches!(dec, TopologyDecision::Disabled(_)));
    }

    #[test]
    fn topology_hints_rejects_empty_selector() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "syncService",
            "tenant-eps-topo-bad-selector"
        );
        let s = slice(vec![]);
        let err = place_topology_hints(&s, &[], &[], TrafficDistribution::PreferClose)
            .expect_err("empty selector must be rejected");
        match err {
            ControllerError::InvalidSpec { kind, .. } => assert_eq!(kind, "EndpointSlice"),
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    #[test]
    fn traffic_distribution_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/topologycache/topologycache.go",
            "TrafficDistribution",
            "tenant-eps-topo-serde"
        );
        for d in [TrafficDistribution::Default, TrafficDistribution::PreferClose] {
            let s = serde_json::to_string(&d).unwrap();
            let back: TrafficDistribution = serde_json::from_str(&s).unwrap();
            assert_eq!(d, back);
        }
    }
}
