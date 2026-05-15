// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EndpointSlice controller — projects ready pods into one or more
//! `discovery.k8s.io/v1.EndpointSlice` objects.
//!
//! Upstream: [`pkg/controller/endpointslice`]. The full controller chunks
//! endpoints across slices, manages slice ownership labels, and reconciles
//! topology hints. This scaffold implements the chunking arithmetic.

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

/// Stub: topology-aware hint placement. Not implemented.
pub fn place_topology_hints(_spec: &EndpointSliceSpec) -> Result<Reconcile, ControllerError> {
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
}
