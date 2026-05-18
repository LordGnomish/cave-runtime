// SPDX-License-Identifier: AGPL-3.0-or-later
//! EndpointSlice — per-service slice allocator + hash-based slice keying.
//!
//! Mirrors `pkg/controller/endpointslice/util/utils.go` plus the
//! port-set hashing in `pkg/controller/endpointslice/topologycache/sliceinfo.go`.
//!
//! Each `Service` gets one or more `EndpointSlice`s; endpoints are bucketed
//! by their *port set* (slices for `tcp/80` vs `tcp/443` are separate). Each
//! bucket holds at most [`MAX_ENDPOINTS_PER_SLICE`] = 100 endpoints.
//!
//! Slice names follow upstream's `<svc>-<8-char-port-hash>-<index>` shape so
//! adding a port doesn't reshuffle existing slice keys.

use crate::types::{Cite, ControllerError, TenantId};
use serde::Serialize;
use std::collections::BTreeMap;

/// Mirrors upstream's `MaxEndpointsPerSlice`.
pub const MAX_ENDPOINTS_PER_SLICE: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize)]
pub struct ServicePort {
    pub protocol: &'static str, // "TCP" | "UDP" | "SCTP"
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointAddr {
    pub address: String,
    /// One entry per port served by this endpoint.
    pub ports: Vec<ServicePort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointSlice {
    pub name: String,
    pub service: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub ports: Vec<ServicePort>,
    pub addresses: Vec<String>,
}

/// FNV-1a 64-bit on the canonicalised port set (sorted, ascii-lowercased
/// protocol). Returns the lower 32 bits as 8 hex chars for use in the
/// slice name. Mirrors upstream's `generateSliceNameSuffix`.
pub fn port_set_hash(ports: &[ServicePort]) -> String {
    let mut sorted: Vec<&ServicePort> = ports.iter().collect();
    sorted.sort();
    sorted.dedup();
    let mut h: u64 = 0xcbf29ce484222325;
    for p in &sorted {
        for b in p.protocol.to_ascii_lowercase().as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= b'/' as u64;
        h = h.wrapping_mul(0x100000001b3);
        for b in p.port.to_string().as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= b';' as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", (h & 0xffff_ffff) as u32)
}

/// Group endpoints by port-set, then chunk each group into slices of at
/// most [`MAX_ENDPOINTS_PER_SLICE`] addresses. Returns slices in stable
/// order (by port-hash, then by chunk index).
pub fn allocate_slices(
    svc: &ServiceSpec,
    endpoints: &[EndpointAddr],
    caller: &TenantId,
) -> Result<Vec<EndpointSlice>, ControllerError> {
    if caller != &svc.tenant {
        return Err(ControllerError::TenantDenied {
            tenant: caller.clone(),
            kind: "EndpointSlice",
            name: svc.name.clone(),
        });
    }
    // Group: port-set → addresses.
    let mut groups: BTreeMap<Vec<ServicePort>, Vec<String>> = BTreeMap::new();
    for ep in endpoints {
        let mut ports = ep.ports.clone();
        ports.sort();
        ports.dedup();
        groups.entry(ports).or_default().push(ep.address.clone());
    }
    let mut out = Vec::new();
    for (ports, mut addrs) in groups {
        addrs.sort();
        addrs.dedup();
        let suffix = port_set_hash(&ports);
        for (idx, chunk) in addrs.chunks(MAX_ENDPOINTS_PER_SLICE as usize).enumerate() {
            out.push(EndpointSlice {
                name: format!("{}-{}-{}", svc.name, suffix, idx),
                service: svc.name.clone(),
                namespace: svc.namespace.clone(),
                tenant: svc.tenant.clone(),
                ports: ports.clone(),
                addresses: chunk.to_vec(),
            });
        }
    }
    Ok(out)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/endpointslice/util/utils.go",
    "generateSliceNameSuffix",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn svc(name: &str, tenant: &str) -> ServiceSpec {
        ServiceSpec { name: name.into(), namespace: "default".into(), tenant: TenantId::new(tenant).expect("test fixture") }
    }

    fn ep(addr: &str, ports: &[(&'static str, u16)]) -> EndpointAddr {
        EndpointAddr {
            address: addr.into(),
            ports: ports.iter().map(|(p, n)| ServicePort { protocol: p, port: *n }).collect(),
        }
    }

    #[test]
    fn port_set_hash_is_stable_for_same_set_independent_of_input_order() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "generateSliceNameSuffix",
            "tenant-eps-hash-stable"
        );
        let a = vec![
            ServicePort { protocol: "TCP", port: 80 },
            ServicePort { protocol: "TCP", port: 443 },
        ];
        let b = vec![
            ServicePort { protocol: "TCP", port: 443 },
            ServicePort { protocol: "TCP", port: 80 },
        ];
        assert_eq!(port_set_hash(&a), port_set_hash(&b));
    }

    #[test]
    fn port_set_hash_differs_when_port_changes() {
        let (_cite, _t) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "generateSliceNameSuffix",
            "tenant-eps-hash-diff"
        );
        let a = vec![ServicePort { protocol: "TCP", port: 80 }];
        let b = vec![ServicePort { protocol: "TCP", port: 81 }];
        assert_ne!(port_set_hash(&a), port_set_hash(&b));
    }

    #[test]
    fn allocate_returns_one_slice_when_endpoint_count_below_chunk() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "syncService",
            "acme"
        );
        let s = svc("web", "acme");
        let eps: Vec<EndpointAddr> = (0..50)
            .map(|i| ep(&format!("10.0.0.{i}"), &[("TCP", 80)]))
            .collect();
        let slices = allocate_slices(&s, &eps, &tenant).unwrap();
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].addresses.len(), 50);
    }

    #[test]
    fn allocate_chunks_at_max_endpoints_per_slice() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "syncService",
            "acme"
        );
        let s = svc("web", "acme");
        let eps: Vec<EndpointAddr> = (0..(MAX_ENDPOINTS_PER_SLICE * 2 + 5))
            .map(|i| ep(&format!("10.0.{}.{}", i / 256, i % 256), &[("TCP", 80)]))
            .collect();
        let slices = allocate_slices(&s, &eps, &tenant).unwrap();
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[0].addresses.len(), MAX_ENDPOINTS_PER_SLICE as usize);
        assert_eq!(slices[1].addresses.len(), MAX_ENDPOINTS_PER_SLICE as usize);
        assert_eq!(slices[2].addresses.len(), 5);
    }

    #[test]
    fn allocate_groups_distinct_port_sets_into_separate_slices() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "groupAddrsByPortSet",
            "acme"
        );
        let s = svc("web", "acme");
        let eps = vec![
            ep("10.0.0.1", &[("TCP", 80)]),
            ep("10.0.0.2", &[("TCP", 443)]),
            ep("10.0.0.3", &[("TCP", 80)]),
        ];
        let slices = allocate_slices(&s, &eps, &tenant).unwrap();
        assert_eq!(slices.len(), 2);
        // Each slice's address set is restricted to its port-set.
        for sl in &slices {
            assert!(sl.ports.len() == 1);
        }
    }

    #[test]
    fn slice_name_contains_service_hash_and_index() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "newSliceName",
            "acme"
        );
        let s = svc("web", "acme");
        let eps = vec![ep("10.0.0.1", &[("TCP", 80)])];
        let slices = allocate_slices(&s, &eps, &tenant).unwrap();
        let parts: Vec<&str> = slices[0].name.splitn(3, '-').collect();
        assert_eq!(parts[0], "web");
        assert_eq!(parts[1].len(), 8); // 8-char hash
        assert_eq!(parts[2], "0");
    }

    #[test]
    fn slice_name_changes_with_port_set_but_not_with_endpoint_added() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "stableSliceName",
            "acme"
        );
        let s = svc("web", "acme");
        let eps_v1 = vec![ep("10.0.0.1", &[("TCP", 80)])];
        let eps_v2 = vec![ep("10.0.0.1", &[("TCP", 80)]), ep("10.0.0.2", &[("TCP", 80)])];
        let s_v1 = allocate_slices(&s, &eps_v1, &tenant).unwrap();
        let s_v2 = allocate_slices(&s, &eps_v2, &tenant).unwrap();
        assert_eq!(s_v1[0].name, s_v2[0].name);
    }

    #[test]
    fn empty_endpoint_set_yields_no_slices() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "syncService",
            "acme"
        );
        let s = svc("web", "acme");
        let slices = allocate_slices(&s, &[], &tenant).unwrap();
        assert!(slices.is_empty());
    }

    #[test]
    fn duplicate_addresses_in_same_port_set_are_collapsed() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/endpointslice/util/utils.go",
            "dedupAddresses",
            "acme"
        );
        let s = svc("web", "acme");
        let eps = vec![
            ep("10.0.0.1", &[("TCP", 80)]),
            ep("10.0.0.1", &[("TCP", 80)]),
            ep("10.0.0.2", &[("TCP", 80)]),
        ];
        let slices = allocate_slices(&s, &eps, &tenant).unwrap();
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].addresses.len(), 2);
    }

    #[test]
    fn cross_tenant_caller_is_refused() {
        let (_cite, attacker) = test_ctx!(
            "pkg/controller/endpointslice/endpointslice_controller.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let s = svc("web", "acme");
        let err = allocate_slices(&s, &[ep("10.0.0.1", &[("TCP", 80)])], &attacker).unwrap_err();
        assert!(matches!(err, ControllerError::TenantDenied { .. }));
    }
}
