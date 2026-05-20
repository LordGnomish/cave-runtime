// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EndpointSlice multi-port + slice cap — `pkg/controller/endpointslice/topologycache`
//! and `pkg/controller/endpointslice/utils.go`.
//!
//! Each EndpointSlice carries a fixed `addressType` (IPv4/IPv6/FQDN), a
//! single `ports[]` shape (every endpoint in the slice answers on the same
//! port set), and up to `MaxEndpointsPerSlice = 100` endpoints. This module
//! implements the slice-allocator that buckets endpoints across multiple
//! slices when the per-port shape collides or capacity is exceeded.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const MAX_ENDPOINTS_PER_SLICE: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressType {
    IPv4,
    IPv6,
    FQDN,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    pub port: u16,
    pub protocol: String, // "TCP"/"UDP"/"SCTP"
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSet {
    /// Sorted by (name, port) for hash/equality stability.
    pub ports: Vec<ServicePort>,
}

impl PortSet {
    pub fn new(mut ports: Vec<ServicePort>) -> Self {
        ports.sort_by(|a, b| (a.name.as_str(), a.port).cmp(&(b.name.as_str(), b.port)));
        Self { ports }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInput {
    pub address: String,
    pub address_type: AddressType,
    pub port_set: PortSet,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBucket {
    pub address_type: AddressType,
    pub ports: PortSet,
    pub endpoints: Vec<String>,
}

/// Bucket endpoints into slices. Endpoints with the same (address_type,
/// port_set) share a bucket; once `MAX_ENDPOINTS_PER_SLICE` is reached,
/// a new slice is spun up for additional endpoints in the same bucket.
pub fn bucket(endpoints: &[EndpointInput]) -> Vec<SliceBucket> {
    let mut out: Vec<SliceBucket> = Vec::new();
    for ep in endpoints {
        // Find the first existing matching slice with capacity.
        let slot = out.iter_mut().find(|b| {
            b.address_type == ep.address_type
                && b.ports == ep.port_set
                && (b.endpoints.len() as u32) < MAX_ENDPOINTS_PER_SLICE
        });
        match slot {
            Some(b) => b.endpoints.push(ep.address.clone()),
            None => out.push(SliceBucket {
                address_type: ep.address_type,
                ports: ep.port_set.clone(),
                endpoints: vec![ep.address.clone()],
            }),
        }
    }
    out
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/endpointslice/utils.go",
    "MaxEndpointsPerSlice",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn sp(name: &str, port: u16) -> ServicePort {
        ServicePort {
            name: name.into(),
            port,
            protocol: "TCP".into(),
        }
    }
    fn ep(addr: &str, ports: PortSet, addr_type: AddressType) -> EndpointInput {
        EndpointInput {
            address: addr.into(),
            address_type: addr_type,
            port_set: ports,
            ready: true,
        }
    }

    #[test]
    fn empty_input_yields_no_buckets() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "Reconciler",
            "tenant-eps-mp-empty"
        );
        assert!(bucket(&[]).is_empty());
    }

    #[test]
    fn same_port_set_groups_into_one_slice() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "Reconciler",
            "tenant-eps-mp-group"
        );
        let ps = PortSet::new(vec![sp("http", 80)]);
        let eps: Vec<_> = (0..5)
            .map(|i| ep(&format!("10.0.0.{i}"), ps.clone(), AddressType::IPv4))
            .collect();
        let buckets = bucket(&eps);
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].endpoints.len(), 5);
    }

    #[test]
    fn different_port_sets_bucket_separately() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "Reconciler",
            "tenant-eps-mp-different-ports"
        );
        let ps_http = PortSet::new(vec![sp("http", 80)]);
        let ps_metrics = PortSet::new(vec![sp("metrics", 9090)]);
        let eps = vec![
            ep("10.0.0.1", ps_http, AddressType::IPv4),
            ep("10.0.0.2", ps_metrics, AddressType::IPv4),
        ];
        let buckets = bucket(&eps);
        assert_eq!(buckets.len(), 2);
    }

    #[test]
    fn different_address_types_bucket_separately() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "Reconciler",
            "tenant-eps-mp-mixed-af"
        );
        let ps = PortSet::new(vec![sp("http", 80)]);
        let eps = vec![
            ep("10.0.0.1", ps.clone(), AddressType::IPv4),
            ep("fd00::1", ps, AddressType::IPv6),
        ];
        let buckets = bucket(&eps);
        assert_eq!(buckets.len(), 2);
    }

    #[test]
    fn slice_caps_at_max_endpoints() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "MaxEndpointsPerSlice",
            "tenant-eps-mp-cap"
        );
        let ps = PortSet::new(vec![sp("http", 80)]);
        let n = MAX_ENDPOINTS_PER_SLICE + 5;
        let eps: Vec<_> = (0..n)
            .map(|i| {
                ep(
                    &format!("10.0.{}.{}", i / 256, i % 256),
                    ps.clone(),
                    AddressType::IPv4,
                )
            })
            .collect();
        let buckets = bucket(&eps);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].endpoints.len(), MAX_ENDPOINTS_PER_SLICE as usize);
        assert_eq!(buckets[1].endpoints.len(), 5);
    }

    #[test]
    fn port_set_canonicalizes_sort_order() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "endpointPortHash",
            "tenant-eps-mp-canonical"
        );
        let a = PortSet::new(vec![sp("z", 9090), sp("a", 80)]);
        let b = PortSet::new(vec![sp("a", 80), sp("z", 9090)]);
        assert_eq!(a, b);
    }

    #[test]
    fn multi_port_service_groups_into_single_slice() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "Reconciler",
            "tenant-eps-mp-multi-port"
        );
        let ps = PortSet::new(vec![sp("http", 80), sp("https", 443)]);
        let eps = vec![
            ep("10.0.0.1", ps.clone(), AddressType::IPv4),
            ep("10.0.0.2", ps, AddressType::IPv4),
        ];
        assert_eq!(bucket(&eps).len(), 1);
    }

    #[test]
    fn max_endpoints_constant_matches_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/endpointslice/utils.go",
            "MaxEndpointsPerSlice",
            "tenant-eps-mp-const"
        );
        assert_eq!(MAX_ENDPOINTS_PER_SLICE, 100);
    }
}
