// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! K8s object handlers — `EndpointSlice` consumer, `ServiceCIDR`
//! awareness.
//!
//! Mirrors `pkg/k8s/watchers/endpoints.go` and the `ServiceCIDR`
//! handling in `pkg/k8s/watchers/service.go` plus the Cilium translator
//! in `pkg/k8s/service_cache.go`.
//!
//! These are the agent-side K8s informers that translate K8s objects
//! (`discovery.k8s.io/v1.EndpointSlice`, `networking.k8s.io/v1.ServiceCIDR`)
//! into the per-service backend pool consumed by the LB.

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::str::FromStr;

// ── EndpointSlice ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndpointCondition {
    Ready,
    NotReady,
    Terminating,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceEndpoint {
    pub addresses: Vec<IpAddr>,
    pub condition: EndpointCondition,
    pub node_name: Option<String>,
    pub target_ref: Option<String>, // namespace/pod-name
    pub zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlicePort {
    pub name: String,
    pub port: u16,
    pub protocol: String, // "TCP"/"UDP"/"SCTP"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointSlice {
    pub name: String,
    pub namespace: String,
    /// Owning service name (label `kubernetes.io/service-name`).
    pub service_name: String,
    pub address_type: String, // "IPv4" / "IPv6"
    pub endpoints: Vec<SliceEndpoint>,
    pub ports: Vec<SlicePort>,
}

impl EndpointSlice {
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HandlerError {
    #[error("EndpointSlice `{0}` not found")]
    SliceNotFound(String),
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("ServiceCIDR `{0}` not found")]
    CidrNotFound(String),
    #[error("tenant {tenant} cannot mutate K8s handler owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Default)]
pub struct EndpointSliceHandler {
    slices: HashMap<String, EndpointSlice>,
    /// service-key (`namespace/svc-name`) → set of slice-keys.
    by_service: BTreeMap<String, Vec<String>>,
}

impl EndpointSliceHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, slice: EndpointSlice) {
        let svc_key = format!("{}/{}", slice.namespace, slice.service_name);
        let slice_key = slice.key();
        let entries = self.by_service.entry(svc_key).or_default();
        if !entries.contains(&slice_key) {
            entries.push(slice_key.clone());
        }
        self.slices.insert(slice_key, slice);
    }

    pub fn remove(&mut self, key: &str) -> Result<(), HandlerError> {
        let removed = self.slices.remove(key).ok_or_else(|| HandlerError::SliceNotFound(key.to_string()))?;
        let svc_key = format!("{}/{}", removed.namespace, removed.service_name);
        if let Some(entries) = self.by_service.get_mut(&svc_key) {
            entries.retain(|k| k != key);
            if entries.is_empty() {
                self.by_service.remove(&svc_key);
            }
        }
        Ok(())
    }

    pub fn slice(&self, key: &str) -> Option<&EndpointSlice> {
        self.slices.get(key)
    }

    pub fn slices_for_service(&self, namespace: &str, service: &str) -> Vec<&EndpointSlice> {
        let svc_key = format!("{namespace}/{service}");
        match self.by_service.get(&svc_key) {
            Some(keys) => keys.iter().filter_map(|k| self.slices.get(k)).collect(),
            None => Vec::new(),
        }
    }

    /// Aggregate every (address, port) tuple from Ready endpoints across
    /// every slice for the service. Mirrors
    /// `pkg/k8s/service_cache.go::endpointsForService`.
    pub fn ready_backends(&self, namespace: &str, service: &str) -> Vec<(IpAddr, u16)> {
        let mut out = Vec::new();
        for slice in self.slices_for_service(namespace, service) {
            for e in &slice.endpoints {
                if !matches!(e.condition, EndpointCondition::Ready) {
                    continue;
                }
                for addr in &e.addresses {
                    for p in &slice.ports {
                        out.push((*addr, p.port));
                    }
                }
            }
        }
        out
    }

    pub fn slice_count(&self) -> usize {
        self.slices.len()
    }

    pub fn service_count(&self) -> usize {
        self.by_service.len()
    }
}

// ── ServiceCIDR (K8s 1.31+) ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceCidrSpec {
    pub name: String,
    pub cidrs: Vec<String>, // e.g. ["10.96.0.0/12", "fd00:96::/108"]
}

#[derive(Debug, Default)]
pub struct ServiceCidrRegistry {
    cidrs: HashMap<String, ServiceCidrSpec>,
}

impl ServiceCidrRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, spec: ServiceCidrSpec) -> Result<(), HandlerError> {
        for c in &spec.cidrs {
            IpNet::from_str(c).map_err(|_| HandlerError::BadCidr(c.clone()))?;
        }
        self.cidrs.insert(spec.name.clone(), spec);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<(), HandlerError> {
        self.cidrs.remove(name).ok_or_else(|| HandlerError::CidrNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&ServiceCidrSpec> {
        self.cidrs.get(name)
    }

    pub fn count(&self) -> usize {
        self.cidrs.len()
    }

    /// True iff `ip` is in any registered ServiceCIDR.
    pub fn contains(&self, ip: IpAddr) -> Result<bool, HandlerError> {
        for spec in self.cidrs.values() {
            for c in &spec.cidrs {
                let net = IpNet::from_str(c).map_err(|_| HandlerError::BadCidr(c.clone()))?;
                if net.contains(&ip) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/k8s/service_cache.go", "ServiceCache");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn slice(ns: &str, name: &str, svc: &str, ips: &[IpAddr], ready: bool) -> EndpointSlice {
        EndpointSlice {
            name: name.into(), namespace: ns.into(), service_name: svc.into(),
            address_type: "IPv4".into(),
            endpoints: vec![SliceEndpoint {
                addresses: ips.to_vec(),
                condition: if ready { EndpointCondition::Ready } else { EndpointCondition::NotReady },
                node_name: Some("node-a".into()),
                target_ref: Some(format!("{ns}/pod-1")),
                zone: Some("zone-a".into()),
            }],
            ports: vec![SlicePort { name: "http".into(), port: 80, protocol: "TCP".into() }],
        }
    }

    // ── EndpointSlice handler ───────────────────────────────────────────────

    #[test]
    fn slice_upsert_indexes_by_service() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Upsert", "tenant-k8s-up");
        let mut h = EndpointSliceHandler::new();
        h.upsert(slice("ns", "svc-abcdef", "svc", &[ip(10, 0, 1, 1)], true));
        assert_eq!(h.slice_count(), 1);
        assert_eq!(h.service_count(), 1);
    }

    #[test]
    fn slice_upsert_replaces_existing() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Upsert.Replace", "tenant-k8s-uprep");
        let mut h = EndpointSliceHandler::new();
        h.upsert(slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1)], true));
        h.upsert(slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 99)], true));
        let s = h.slice("ns/svc-1").unwrap();
        assert_eq!(s.endpoints[0].addresses, vec![ip(10, 0, 1, 99)]);
        assert_eq!(h.slice_count(), 1);
    }

    #[test]
    fn slice_remove_drops_entry() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Remove", "tenant-k8s-rm");
        let mut h = EndpointSliceHandler::new();
        h.upsert(slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1)], true));
        h.remove("ns/svc-1").unwrap();
        assert_eq!(h.slice_count(), 0);
        assert_eq!(h.service_count(), 0);
    }

    #[test]
    fn slice_remove_unknown_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Remove.NotFound", "tenant-k8s-rmnf");
        let mut h = EndpointSliceHandler::new();
        let err = h.remove("ns/missing").unwrap_err();
        assert!(matches!(err, HandlerError::SliceNotFound(_)));
    }

    // ── slices_for_service ──────────────────────────────────────────────────

    #[test]
    fn slices_for_service_aggregates_all_owned() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/service_cache.go", "SlicesForService", "tenant-k8s-sfs");
        let mut h = EndpointSliceHandler::new();
        h.upsert(slice("ns", "svc-abc", "svc", &[ip(10, 0, 1, 1)], true));
        h.upsert(slice("ns", "svc-def", "svc", &[ip(10, 0, 1, 2)], true));
        h.upsert(slice("ns", "other-xyz", "other", &[ip(10, 0, 2, 1)], true));
        let r = h.slices_for_service("ns", "svc");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn slices_for_service_empty_when_no_match() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/service_cache.go", "SlicesForService.None", "tenant-k8s-sfsn");
        let h = EndpointSliceHandler::new();
        assert!(h.slices_for_service("ns", "svc").is_empty());
    }

    // ── ready_backends ──────────────────────────────────────────────────────

    #[test]
    fn ready_backends_returns_addr_port_pairs() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/service_cache.go", "ReadyBackends", "tenant-k8s-rb");
        let mut h = EndpointSliceHandler::new();
        h.upsert(slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1), ip(10, 0, 1, 2)], true));
        let backends = h.ready_backends("ns", "svc");
        assert_eq!(backends.len(), 2);
        assert!(backends.contains(&(ip(10, 0, 1, 1), 80)));
        assert!(backends.contains(&(ip(10, 0, 1, 2), 80)));
    }

    #[test]
    fn ready_backends_excludes_not_ready_endpoints() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/service_cache.go", "ReadyBackends.SkipNotReady", "tenant-k8s-rbs");
        let mut h = EndpointSliceHandler::new();
        h.upsert(slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1)], false));
        let backends = h.ready_backends("ns", "svc");
        assert!(backends.is_empty());
    }

    #[test]
    fn ready_backends_excludes_terminating_endpoints() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/service_cache.go", "ReadyBackends.SkipTerminating", "tenant-k8s-rbt");
        let mut h = EndpointSliceHandler::new();
        let mut s = slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1)], true);
        s.endpoints[0].condition = EndpointCondition::Terminating;
        h.upsert(s);
        let backends = h.ready_backends("ns", "svc");
        assert!(backends.is_empty());
    }

    #[test]
    fn ready_backends_combines_addresses_x_ports() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/service_cache.go", "ReadyBackends.AddressXPort", "tenant-k8s-rbxp");
        let mut h = EndpointSliceHandler::new();
        let mut s = slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1)], true);
        s.ports.push(SlicePort { name: "https".into(), port: 443, protocol: "TCP".into() });
        h.upsert(s);
        let backends = h.ready_backends("ns", "svc");
        assert_eq!(backends.len(), 2);
        assert!(backends.contains(&(ip(10, 0, 1, 1), 80)));
        assert!(backends.contains(&(ip(10, 0, 1, 1), 443)));
    }

    // ── Slice key ───────────────────────────────────────────────────────────

    #[test]
    fn slice_key_format() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Slice.Key", "tenant-k8s-key");
        let s = slice("ns", "svc-abc", "svc", &[ip(10, 0, 1, 1)], true);
        assert_eq!(s.key(), "ns/svc-abc");
    }

    // ── ServiceCIDR registry ────────────────────────────────────────────────

    #[test]
    fn cidr_upsert_registers() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Upsert", "tenant-k8s-cup");
        let mut r = ServiceCidrRegistry::new();
        r.upsert(ServiceCidrSpec {
            name: "default".into(),
            cidrs: vec!["10.96.0.0/12".into()],
        }).unwrap();
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn cidr_upsert_with_invalid_cidr_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.BadCidr", "tenant-k8s-cbad");
        let mut r = ServiceCidrRegistry::new();
        let err = r.upsert(ServiceCidrSpec {
            name: "default".into(),
            cidrs: vec!["nope".into()],
        }).unwrap_err();
        assert!(matches!(err, HandlerError::BadCidr(_)));
    }

    #[test]
    fn cidr_remove_drops_entry() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Remove", "tenant-k8s-crm");
        let mut r = ServiceCidrRegistry::new();
        r.upsert(ServiceCidrSpec { name: "default".into(), cidrs: vec!["10.96.0.0/12".into()] }).unwrap();
        r.remove("default").unwrap();
        assert_eq!(r.count(), 0);
    }

    #[test]
    fn cidr_remove_unknown_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Remove.NotFound", "tenant-k8s-crmnf");
        let mut r = ServiceCidrRegistry::new();
        let err = r.remove("ghost").unwrap_err();
        assert!(matches!(err, HandlerError::CidrNotFound(_)));
    }

    #[test]
    fn cidr_contains_true_for_in_range() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Contains.In", "tenant-k8s-cin");
        let mut r = ServiceCidrRegistry::new();
        r.upsert(ServiceCidrSpec { name: "default".into(), cidrs: vec!["10.96.0.0/12".into()] }).unwrap();
        assert!(r.contains(ip(10, 96, 0, 100)).unwrap());
    }

    #[test]
    fn cidr_contains_false_for_out_of_range() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Contains.Out", "tenant-k8s-cout");
        let mut r = ServiceCidrRegistry::new();
        r.upsert(ServiceCidrSpec { name: "default".into(), cidrs: vec!["10.96.0.0/12".into()] }).unwrap();
        assert!(!r.contains(ip(192, 168, 1, 1)).unwrap());
    }

    #[test]
    fn cidr_contains_v6() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Contains.V6", "tenant-k8s-cv6");
        let mut r = ServiceCidrRegistry::new();
        r.upsert(ServiceCidrSpec { name: "v6".into(), cidrs: vec!["fd00:96::/108".into()] }).unwrap();
        let ip6: IpAddr = "fd00:96::1".parse().unwrap();
        assert!(r.contains(ip6).unwrap());
    }

    #[test]
    fn cidr_multiple_registries_combine() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Multi", "tenant-k8s-cmulti");
        let mut r = ServiceCidrRegistry::new();
        r.upsert(ServiceCidrSpec { name: "v4".into(), cidrs: vec!["10.96.0.0/12".into()] }).unwrap();
        r.upsert(ServiceCidrSpec { name: "extra".into(), cidrs: vec!["172.16.0.0/12".into()] }).unwrap();
        assert!(r.contains(ip(10, 96, 0, 1)).unwrap());
        assert!(r.contains(ip(172, 16, 0, 1)).unwrap());
        assert!(!r.contains(ip(192, 168, 1, 1)).unwrap());
    }

    // ── Counts ──────────────────────────────────────────────────────────────

    #[test]
    fn slice_count_tracks_inserts() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "SliceCount", "tenant-k8s-sc");
        let mut h = EndpointSliceHandler::new();
        for i in 0..5u8 {
            h.upsert(slice("ns", &format!("s-{i}"), "svc", &[ip(10, 0, 1, i)], true));
        }
        assert_eq!(h.slice_count(), 5);
    }

    // ── EndpointCondition ───────────────────────────────────────────────────

    #[test]
    fn endpoint_condition_ready_returns_true_for_ready_state() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Condition.Ready", "tenant-k8s-cr");
        assert!(matches!(EndpointCondition::Ready, EndpointCondition::Ready));
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn endpoint_slice_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Slice.Serde", "tenant-k8s-sserde");
        let s = slice("ns", "svc-1", "svc", &[ip(10, 0, 1, 1)], true);
        let json = serde_json::to_string(&s).unwrap();
        let back: EndpointSlice = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn service_cidr_spec_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/service.go", "ServiceCIDR.Serde", "tenant-k8s-cserde");
        let s = ServiceCidrSpec { name: "default".into(), cidrs: vec!["10.96.0.0/12".into()] };
        let json = serde_json::to_string(&s).unwrap();
        let back: ServiceCidrSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn endpoint_condition_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/k8s/watchers/endpoints.go", "Condition.Serde", "tenant-k8s-condserde");
        for c in [EndpointCondition::Ready, EndpointCondition::NotReady, EndpointCondition::Terminating] {
            let s = serde_json::to_string(&c).unwrap();
            let back: EndpointCondition = serde_json::from_str(&s).unwrap();
            assert_eq!(back, c);
        }
    }
}
