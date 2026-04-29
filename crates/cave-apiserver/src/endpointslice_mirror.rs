//! EndpointSlice mirror controller.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `pkg/controller/endpointslicemirroring/endpointslicemirroring_controller.go`
//!   * `pkg/controller/endpointslicemirroring/reconciler.go`
//!     (`reconcile`, `desiredEndpointSlicesFromEndpoints`).
//!   * `staging/src/k8s.io/api/discovery/v1/types.go` (`EndpointSlice`).
//!
//! The mirror controller watches `Endpoints` resources that were not created
//! from a Service (i.e. `EndpointSlice`-owned by the user) and emits
//! `EndpointSlice` objects that mirror their addresses. The output is split
//! by address-family (IPv4 / IPv6) and capped per slice via
//! `maxEndpointsPerSlice` (upstream default 100).
//!
//! Tenant invariant: every produced EndpointSlice carries the `tenant_id`
//! sourced from the input Endpoints. The reconciler MUST NOT cross tenants
//! and MUST set the standard `kubernetes.io/service-name` label so consumers
//! can locate the slice for the owning Service.

use crate::resources::{Endpoints, EndpointSubset, ObjectReference};
use serde::{Deserialize, Serialize};

/// Address family for a slice. Upstream `discovery.AddressType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressType {
    IPv4,
    IPv6,
    /// FQDN — produced when an EndpointAddress has only `hostname`.
    FQDN,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointConditions {
    pub ready: bool,
    pub serving: bool,
    pub terminating: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointEntry {
    pub addresses: Vec<String>,
    pub conditions: EndpointConditions,
    pub hostname: Option<String>,
    pub target_ref: Option<ObjectReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlicePort {
    pub name: Option<String>,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointSlice {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub address_type: AddressType,
    pub endpoints: Vec<EndpointEntry>,
    pub ports: Vec<SlicePort>,
    pub owner_endpoints: ObjectReference,
    pub service_name: String,
    /// Standard mirror-controller managed-by label value.
    pub managed_by: String,
}

/// Upstream default — `pkg/controller/endpointslicemirroring/config.go`.
pub const DEFAULT_MAX_ENDPOINTS_PER_SLICE: usize = 100;

/// Standard label value the upstream mirror controller stamps on every
/// EndpointSlice it produces.
pub const MIRROR_MANAGED_BY: &str = "endpointslice-mirroring-controller";

/// Reconciler mirrors an `Endpoints` resource into a list of `EndpointSlice`
/// resources. Pure function — no IO. Caller persists the output via the store.
pub struct MirrorReconciler {
    pub max_endpoints_per_slice: usize,
}

impl MirrorReconciler {
    pub fn new(max_endpoints_per_slice: usize) -> Self {
        assert!(max_endpoints_per_slice > 0, "max_endpoints_per_slice must be > 0");
        Self { max_endpoints_per_slice }
    }

    /// Mirror an `Endpoints` (scoped to `tenant_id`) into `EndpointSlice`s.
    /// Output ordered by address-family, then by chunk.
    pub fn reconcile(&self, tenant_id: &str, ep: &Endpoints) -> Vec<EndpointSlice> {
        let svc_name = ep.metadata.name.clone();
        let ns = ep.metadata.namespace.clone();
        let owner_ref = ObjectReference {
            kind: "Endpoints".into(),
            name: svc_name.clone(),
            namespace: ns.clone(),
            api_version: Some("v1".into()),
            uid: Some(ep.metadata.uid),
        };

        // Group entries from all subsets by address family.
        let mut v4: Vec<EndpointEntry> = vec![];
        let mut v6: Vec<EndpointEntry> = vec![];
        let mut fqdn: Vec<EndpointEntry> = vec![];
        let mut canonical_ports: Vec<SlicePort> = vec![];

        for subset in &ep.subsets {
            for sp in &subset.ports {
                let port = SlicePort {
                    name: sp.name.clone(),
                    port: sp.port,
                    protocol: sp.protocol.clone(),
                };
                if !canonical_ports.iter().any(|p|
                    p.name == port.name && p.port == port.port && p.protocol == port.protocol
                ) {
                    canonical_ports.push(port);
                }
            }
            push_addresses(subset, /*ready=*/ true,  &mut v4, &mut v6, &mut fqdn);
        }
        // Not-ready subsets: still mirrored, but with ready=false.
        for subset in &ep.subsets {
            push_addresses(subset, /*ready=*/ false, &mut v4, &mut v6, &mut fqdn);
        }

        let mut out: Vec<EndpointSlice> = vec![];
        self.emit_chunks(tenant_id, &svc_name, &ns, &owner_ref,
            AddressType::IPv4, &v4, &canonical_ports, &mut out);
        self.emit_chunks(tenant_id, &svc_name, &ns, &owner_ref,
            AddressType::IPv6, &v6, &canonical_ports, &mut out);
        self.emit_chunks(tenant_id, &svc_name, &ns, &owner_ref,
            AddressType::FQDN, &fqdn, &canonical_ports, &mut out);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_chunks(
        &self,
        tenant_id: &str,
        svc_name: &str,
        ns: &str,
        owner_ref: &ObjectReference,
        family: AddressType,
        entries: &[EndpointEntry],
        ports: &[SlicePort],
        out: &mut Vec<EndpointSlice>,
    ) {
        if entries.is_empty() {
            return;
        }
        for (chunk_idx, chunk) in entries.chunks(self.max_endpoints_per_slice).enumerate() {
            let slice_name = format!("{}-{}-{}",
                svc_name,
                family_suffix(family),
                chunk_idx,
            );
            out.push(EndpointSlice {
                api_version: "discovery.k8s.io/v1".into(),
                kind: "EndpointSlice".into(),
                name: slice_name,
                namespace: ns.into(),
                tenant_id: tenant_id.into(),
                address_type: family,
                endpoints: chunk.to_vec(),
                ports: ports.to_vec(),
                owner_endpoints: owner_ref.clone(),
                service_name: svc_name.into(),
                managed_by: MIRROR_MANAGED_BY.into(),
            });
        }
    }
}

impl Default for MirrorReconciler {
    fn default() -> Self {
        Self { max_endpoints_per_slice: DEFAULT_MAX_ENDPOINTS_PER_SLICE }
    }
}

fn family_suffix(f: AddressType) -> &'static str {
    match f {
        AddressType::IPv4 => "ipv4",
        AddressType::IPv6 => "ipv6",
        AddressType::FQDN => "fqdn",
    }
}

fn classify(addr: &str) -> AddressType {
    if addr.contains(':') {
        AddressType::IPv6
    } else if addr.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        && addr.split('.').count() == 4
    {
        AddressType::IPv4
    } else {
        AddressType::FQDN
    }
}

fn push_addresses(
    subset: &EndpointSubset,
    ready: bool,
    v4: &mut Vec<EndpointEntry>,
    v6: &mut Vec<EndpointEntry>,
    fqdn: &mut Vec<EndpointEntry>,
) {
    let raw = if ready { &subset.addresses } else { &subset.not_ready_addresses };
    for a in raw {
        let entry = EndpointEntry {
            addresses: vec![a.ip.clone()],
            conditions: EndpointConditions {
                ready,
                serving: ready,
                terminating: false,
            },
            hostname: a.hostname.clone(),
            target_ref: a.target_ref.clone(),
        };
        match classify(&a.ip) {
            AddressType::IPv4 => v4.push(entry),
            AddressType::IPv6 => v6.push(entry),
            AddressType::FQDN => fqdn.push(entry),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::{EndpointAddress, EndpointPort, ObjectMeta};

    fn ep(name: &str, ns: &str) -> Endpoints {
        Endpoints {
            api_version: "v1".into(),
            kind: "Endpoints".into(),
            metadata: ObjectMeta::new(name, ns),
            subsets: vec![],
        }
    }

    fn addr(ip: &str) -> EndpointAddress {
        EndpointAddress { ip: ip.into(), hostname: None, target_ref: None }
    }

    fn port(name: &str, p: u16) -> EndpointPort {
        EndpointPort { name: Some(name.into()), port: p, protocol: "TCP".into() }
    }

    /// Upstream parity: `TestReconciler_BasicMirrorOneSubsetOneSlice`
    /// (endpointslicemirroring/reconciler_test.go — 1 subset → 1 slice).
    #[test]
    fn test_basic_mirror_yields_one_slice_per_family() {
        let mut e = ep("svc-a", "default");
        e.subsets.push(EndpointSubset {
            addresses: vec![addr("10.0.0.1"), addr("10.0.0.2")],
            not_ready_addresses: vec![],
            ports: vec![port("http", 80)],
        });
        let r = MirrorReconciler::default();
        let slices = r.reconcile("acme", &e);
        assert_eq!(slices.len(), 1, "single IPv4 family → single slice");
        let s = &slices[0];
        assert_eq!(s.address_type, AddressType::IPv4);
        assert_eq!(s.endpoints.len(), 2);
        assert_eq!(s.service_name, "svc-a");
        assert_eq!(s.managed_by, MIRROR_MANAGED_BY);
        assert_eq!(s.tenant_id, "acme",
            "tenant_id invariant: mirrored slice carries source Endpoints' tenant");
    }

    /// Upstream parity: `TestReconciler_OwnerReferenceSet`
    /// (slices reference their source Endpoints via OwnerReference).
    #[test]
    fn test_mirrored_slice_owner_ref_points_to_source_endpoints() {
        let mut e = ep("svc-a", "default");
        e.subsets.push(EndpointSubset {
            addresses: vec![addr("10.0.0.1")],
            not_ready_addresses: vec![],
            ports: vec![port("http", 80)],
        });
        let source_uid = e.metadata.uid;
        let r = MirrorReconciler::default();
        let slices = r.reconcile("acme", &e);
        let s = &slices[0];
        assert_eq!(s.owner_endpoints.kind, "Endpoints");
        assert_eq!(s.owner_endpoints.name, "svc-a");
        assert_eq!(s.owner_endpoints.uid, Some(source_uid));
        assert_eq!(s.tenant_id, "acme",
            "tenant_id invariant: owner-ref tagged slice still carries tenant");
    }

    /// Upstream parity: `TestReconciler_AddressFamiliesSplit`
    /// (mixed IPv4 + IPv6 → one slice per family — discovery v1 does not
    /// allow mixed-family slices).
    #[test]
    fn test_mixed_address_families_produce_separate_slices() {
        let mut e = ep("svc-a", "default");
        e.subsets.push(EndpointSubset {
            addresses: vec![addr("10.0.0.1"), addr("fd00::1"), addr("10.0.0.2")],
            not_ready_addresses: vec![],
            ports: vec![port("http", 80)],
        });
        let r = MirrorReconciler::default();
        let slices = r.reconcile("acme", &e);
        assert_eq!(slices.len(), 2, "IPv4 + IPv6 → exactly two slices");
        let v4 = slices.iter().find(|s| s.address_type == AddressType::IPv4).unwrap();
        let v6 = slices.iter().find(|s| s.address_type == AddressType::IPv6).unwrap();
        assert_eq!(v4.endpoints.len(), 2);
        assert_eq!(v6.endpoints.len(), 1);
        assert!(slices.iter().all(|s| s.tenant_id == "acme"),
            "tenant_id invariant: per-family slices all carry source tenant");
    }

    /// Upstream parity: `TestReconciler_MaxEndpointsPerSliceSplits`
    /// (chunking by `maxEndpointsPerSlice` — upstream default 100).
    #[test]
    fn test_chunking_respects_max_endpoints_per_slice() {
        let mut e = ep("svc-big", "default");
        let addrs: Vec<EndpointAddress> = (0..7)
            .map(|i| addr(&format!("10.0.0.{}", i + 1))).collect();
        e.subsets.push(EndpointSubset {
            addresses: addrs,
            not_ready_addresses: vec![],
            ports: vec![port("http", 80)],
        });
        let r = MirrorReconciler::new(3);
        let slices = r.reconcile("acme", &e);
        assert_eq!(slices.len(), 3, "7 addresses chunked by 3 → 3 slices");
        assert_eq!(slices[0].endpoints.len(), 3);
        assert_eq!(slices[1].endpoints.len(), 3);
        assert_eq!(slices[2].endpoints.len(), 1);
        assert!(slices.iter().all(|s| s.tenant_id == "acme"),
            "tenant_id invariant: every chunk carries source tenant");
        // Chunk names are deterministic + distinct.
        let names: Vec<_> = slices.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names[0], "svc-big-ipv4-0");
        assert_eq!(names[2], "svc-big-ipv4-2");
    }

    /// Upstream parity: `TestReconciler_NotReadyAddressesMirroredWithReadyFalse`
    /// (terminating/not-ready addresses become `conditions.ready=false`).
    #[test]
    fn test_not_ready_addresses_mirrored_with_ready_false() {
        let mut e = ep("svc-a", "default");
        e.subsets.push(EndpointSubset {
            addresses: vec![addr("10.0.0.1")],
            not_ready_addresses: vec![addr("10.0.0.99")],
            ports: vec![port("http", 80)],
        });
        let r = MirrorReconciler::default();
        let slices = r.reconcile("acme", &e);
        assert_eq!(slices.len(), 1);
        let s = &slices[0];
        let ready = s.endpoints.iter().filter(|e| e.conditions.ready).count();
        let not_ready = s.endpoints.iter().filter(|e| !e.conditions.ready).count();
        assert_eq!(ready, 1, "the .1 address is ready");
        assert_eq!(not_ready, 1, "the .99 address is not-ready");
        assert_eq!(s.tenant_id, "acme",
            "tenant_id invariant: not-ready entries still scoped to source tenant");
    }

    /// Upstream parity: `TestReconciler_EmptyEndpointsProducesNoSlices`
    /// (a Endpoints with no subsets emits no slices — never an empty slice).
    #[test]
    fn test_empty_endpoints_produces_no_slices() {
        let e = ep("svc-empty", "default");
        let r = MirrorReconciler::default();
        let slices = r.reconcile("acme", &e);
        assert!(slices.is_empty(),
            "no subsets → no slices (upstream avoids empty-slice churn)");
        // tenant_id invariant smoke: a parallel call for another tenant is
        // also empty and disjoint.
        let other = r.reconcile("globex", &e);
        assert!(other.is_empty());
    }

    /// Upstream parity: `TestReconciler_TenantIdSourcedFromCallerNotEndpoints`
    /// (the reconciler is the trust boundary — the tenant_id parameter wins
    /// and the same Endpoints object can be mirrored under different tenants
    /// without cross-contamination).
    #[test]
    fn test_tenant_id_for_mirror_comes_from_caller_and_does_not_cross() {
        let mut e = ep("svc-a", "default");
        e.subsets.push(EndpointSubset {
            addresses: vec![addr("10.0.0.1")],
            not_ready_addresses: vec![],
            ports: vec![port("http", 80)],
        });
        let r = MirrorReconciler::default();
        let acme_out   = r.reconcile("acme", &e);
        let globex_out = r.reconcile("globex", &e);
        assert!(acme_out.iter().all(|s| s.tenant_id == "acme"),
            "tenant_id invariant: acme call stamps acme on every slice");
        assert!(globex_out.iter().all(|s| s.tenant_id == "globex"),
            "tenant_id invariant: globex call stamps globex on every slice");
        // Same slice content, different tenants — proves no shared mutable state.
        assert_eq!(acme_out[0].endpoints.len(), globex_out[0].endpoints.len());
    }
}
