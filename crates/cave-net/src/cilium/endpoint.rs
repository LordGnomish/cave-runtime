// SPDX-License-Identifier: AGPL-3.0-or-later
//! Endpoint program model — per-pod datapath state.
//!
//! Mirrors `pkg/endpoint/endpoint.go` (the in-memory `Endpoint` struct
//! and its lifecycle) plus the tail-call program chain from
//! `bpf/bpf_lxc.c::cil_from_container`. The real datapath is BPF
//! programs loaded into the kernel; we model the Endpoint metadata,
//! state machine, and the *order* of tail-called programs.

use crate::cilium::identity::LabelSet;
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndpointState {
    Creating,
    WaitingForIdentity,
    Restoring,
    Ready,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BpfProgram {
    /// `bpf/bpf_lxc.c::cil_from_container` entry.
    FromContainer,
    /// Conntrack lookup tail call.
    Conntrack,
    /// Service LB tail call.
    Lb,
    /// Policy enforcement tail call.
    Policy,
    /// L7 redirect tail call (envoy proxy).
    L7Redirect,
    /// Encrypt tail call (WireGuard or IPsec).
    Encrypt,
    /// Final emit to lxc-egress.
    ToLxc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: u64,
    pub tenant: TenantId,
    pub pod_name: String,
    pub pod_namespace: String,
    pub pod_ip: IpAddr,
    pub identity: u32,
    pub labels: LabelSet,
    pub state: EndpointState,
    pub if_index: u32,
    /// Ordered tail-call chain (mirrors the program graph compiled into
    /// the per-endpoint BPF object).
    pub program_chain: Vec<BpfProgram>,
}

impl Endpoint {
    pub fn new_creating(
        id: u64, tenant: TenantId,
        pod_name: impl Into<String>, pod_namespace: impl Into<String>,
        pod_ip: IpAddr,
    ) -> Self {
        Self {
            id, tenant,
            pod_name: pod_name.into(),
            pod_namespace: pod_namespace.into(),
            pod_ip,
            identity: 0,
            labels: LabelSet { pairs: Vec::new() },
            state: EndpointState::Creating,
            if_index: 0,
            program_chain: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EndpointError {
    #[error("endpoint id {0} already exists")]
    DuplicateId(u64),
    #[error("endpoint id {0} not found")]
    NotFound(u64),
    #[error("invalid state transition {from:?} → {to:?}")]
    BadTransition { from: EndpointState, to: EndpointState },
    #[error("tenant {tenant} cannot mutate endpoint owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Default)]
pub struct EndpointManager {
    next_id: u64,
    by_id: HashMap<u64, Endpoint>,
    by_pod_ip: HashMap<IpAddr, u64>,
}

impl EndpointManager {
    pub fn new() -> Self {
        Self { next_id: 1, by_id: HashMap::new(), by_pod_ip: HashMap::new() }
    }

    /// Create a new endpoint, auto-assigning an ID.
    pub fn create(
        &mut self, tenant: TenantId,
        pod_name: impl Into<String>, pod_namespace: impl Into<String>,
        pod_ip: IpAddr,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let ep = Endpoint::new_creating(id, tenant, pod_name, pod_namespace, pod_ip);
        self.by_pod_ip.insert(pod_ip, id);
        self.by_id.insert(id, ep);
        id
    }

    pub fn insert(&mut self, ep: Endpoint) -> Result<(), EndpointError> {
        if self.by_id.contains_key(&ep.id) {
            return Err(EndpointError::DuplicateId(ep.id));
        }
        self.by_pod_ip.insert(ep.pod_ip, ep.id);
        if ep.id >= self.next_id {
            self.next_id = ep.id + 1;
        }
        self.by_id.insert(ep.id, ep);
        Ok(())
    }

    pub fn lookup(&self, id: u64) -> Option<&Endpoint> {
        self.by_id.get(&id)
    }

    pub fn lookup_by_pod_ip(&self, ip: IpAddr) -> Option<&Endpoint> {
        let id = self.by_pod_ip.get(&ip)?;
        self.by_id.get(id)
    }

    pub fn remove(&mut self, id: u64) -> Result<(), EndpointError> {
        let ep = self.by_id.remove(&id).ok_or(EndpointError::NotFound(id))?;
        self.by_pod_ip.remove(&ep.pod_ip);
        Ok(())
    }

    pub fn set_identity(&mut self, id: u64, identity: u32, labels: LabelSet) -> Result<(), EndpointError> {
        let ep = self.by_id.get_mut(&id).ok_or(EndpointError::NotFound(id))?;
        ep.identity = identity;
        ep.labels = labels;
        Ok(())
    }

    /// Apply a state transition. Mirrors `pkg/endpoint/endpoint.go::SetState`.
    pub fn transition(&mut self, id: u64, to: EndpointState) -> Result<(), EndpointError> {
        let ep = self.by_id.get_mut(&id).ok_or(EndpointError::NotFound(id))?;
        let from = ep.state;
        let ok = matches!(
            (from, to),
            (EndpointState::Creating, EndpointState::WaitingForIdentity)
                | (EndpointState::Creating, EndpointState::Restoring)
                | (EndpointState::WaitingForIdentity, EndpointState::Ready)
                | (EndpointState::Restoring, EndpointState::Ready)
                | (EndpointState::Ready, EndpointState::Disconnecting)
                | (EndpointState::Disconnecting, EndpointState::Disconnected)
        );
        if !ok {
            return Err(EndpointError::BadTransition { from, to });
        }
        ep.state = to;
        Ok(())
    }

    pub fn set_program_chain(&mut self, id: u64, chain: Vec<BpfProgram>) -> Result<(), EndpointError> {
        let ep = self.by_id.get_mut(&id).ok_or(EndpointError::NotFound(id))?;
        ep.program_chain = chain;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.by_id.len()
    }
}

/// The canonical egress program chain for a Cilium endpoint.
/// Mirrors the tail-call sequence in `bpf/bpf_lxc.c::cil_from_container`.
pub fn canonical_egress_chain() -> Vec<BpfProgram> {
    vec![
        BpfProgram::FromContainer,
        BpfProgram::Conntrack,
        BpfProgram::Lb,
        BpfProgram::Policy,
        BpfProgram::L7Redirect,
        BpfProgram::Encrypt,
        BpfProgram::ToLxc,
    ]
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/endpoint/endpoint.go", "Endpoint");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::identity::LabelSet;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn endpoint_create_assigns_monotonic_id() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.New", "tenant-ep-id");
        let mut mgr = EndpointManager::new();
        let a = mgr.create(tenant.clone(), "p1", "default", ip(10, 0, 1, 1));
        let b = mgr.create(tenant, "p2", "default", ip(10, 0, 1, 2));
        assert_eq!(a, 1);
        assert_eq!(b, 2);
    }

    #[test]
    fn endpoint_lookup_by_id_round_trips() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.LookupByID", "tenant-ep-lk");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p1", "default", ip(10, 0, 1, 1));
        let ep = mgr.lookup(id).unwrap();
        assert_eq!(ep.pod_name, "p1");
        assert_eq!(ep.state, EndpointState::Creating);
    }

    #[test]
    fn endpoint_lookup_by_pod_ip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.LookupByIP", "tenant-ep-lkip");
        let mut mgr = EndpointManager::new();
        mgr.create(tenant, "p1", "default", ip(10, 0, 1, 5));
        let ep = mgr.lookup_by_pod_ip(ip(10, 0, 1, 5)).unwrap();
        assert_eq!(ep.pod_name, "p1");
    }

    #[test]
    fn endpoint_lookup_unknown_returns_none() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.LookupByID.NotFound", "tenant-ep-nf");
        let mgr = EndpointManager::new();
        assert!(mgr.lookup(999).is_none());
        assert!(mgr.lookup_by_pod_ip(ip(10, 0, 1, 99)).is_none());
    }

    #[test]
    fn endpoint_remove_drops_state() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.Delete", "tenant-ep-rm");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p1", "default", ip(10, 0, 1, 1));
        mgr.remove(id).unwrap();
        assert!(mgr.lookup(id).is_none());
        assert!(mgr.lookup_by_pod_ip(ip(10, 0, 1, 1)).is_none());
    }

    #[test]
    fn endpoint_remove_unknown_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.Delete.NotFound", "tenant-ep-rmnf");
        let mut mgr = EndpointManager::new();
        let err = mgr.remove(99).unwrap_err();
        assert_eq!(err, EndpointError::NotFound(99));
    }

    #[test]
    fn endpoint_insert_with_duplicate_id_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.Insert.Duplicate", "tenant-ep-dup");
        let mut mgr = EndpointManager::new();
        let ep = Endpoint::new_creating(7, tenant.clone(), "p", "default", ip(10, 0, 1, 1));
        mgr.insert(ep.clone()).unwrap();
        let err = mgr.insert(ep).unwrap_err();
        assert_eq!(err, EndpointError::DuplicateId(7));
    }

    // ── Identity ─────────────────────────────────────────────────────────────

    #[test]
    fn endpoint_set_identity_records_labels() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.SetIdentity", "tenant-ep-idt");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p1", "default", ip(10, 0, 1, 1));
        mgr.set_identity(id, 256, ls(&[("app", "web")])).unwrap();
        let ep = mgr.lookup(id).unwrap();
        assert_eq!(ep.identity, 256);
        assert_eq!(ep.labels.pairs, vec![("app".into(), "web".into())]);
    }

    // ── State transitions ────────────────────────────────────────────────────

    #[test]
    fn endpoint_state_transition_creating_to_waiting() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.SetState", "tenant-ep-st1");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p", "default", ip(10, 0, 1, 1));
        mgr.transition(id, EndpointState::WaitingForIdentity).unwrap();
        assert_eq!(mgr.lookup(id).unwrap().state, EndpointState::WaitingForIdentity);
    }

    #[test]
    fn endpoint_state_transition_to_ready() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.SetState.Ready", "tenant-ep-st2");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p", "default", ip(10, 0, 1, 1));
        mgr.transition(id, EndpointState::WaitingForIdentity).unwrap();
        mgr.transition(id, EndpointState::Ready).unwrap();
        assert_eq!(mgr.lookup(id).unwrap().state, EndpointState::Ready);
    }

    #[test]
    fn endpoint_invalid_state_transition_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.SetState.Invalid", "tenant-ep-stbad");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p", "default", ip(10, 0, 1, 1));
        let err = mgr.transition(id, EndpointState::Disconnected).unwrap_err();
        assert_eq!(err, EndpointError::BadTransition { from: EndpointState::Creating, to: EndpointState::Disconnected });
    }

    #[test]
    fn endpoint_state_transition_to_disconnecting() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.SetState.Disconnect", "tenant-ep-dis");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p", "default", ip(10, 0, 1, 1));
        mgr.transition(id, EndpointState::WaitingForIdentity).unwrap();
        mgr.transition(id, EndpointState::Ready).unwrap();
        mgr.transition(id, EndpointState::Disconnecting).unwrap();
        assert_eq!(mgr.lookup(id).unwrap().state, EndpointState::Disconnecting);
    }

    // ── Tail-call chain ──────────────────────────────────────────────────────

    #[test]
    fn endpoint_canonical_egress_chain_order() {
        let (_c, _t) = cilium_test_ctx!("bpf/bpf_lxc.c", "cil_from_container", "tenant-ep-chain");
        let chain = canonical_egress_chain();
        assert_eq!(chain[0], BpfProgram::FromContainer);
        assert_eq!(chain[1], BpfProgram::Conntrack);
        assert_eq!(chain[2], BpfProgram::Lb);
        assert_eq!(chain[3], BpfProgram::Policy);
        assert_eq!(chain[4], BpfProgram::L7Redirect);
        assert_eq!(chain[5], BpfProgram::Encrypt);
        assert_eq!(chain[6], BpfProgram::ToLxc);
    }

    #[test]
    fn endpoint_set_program_chain_records_progs() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.RegenerateBPF", "tenant-ep-prog");
        let mut mgr = EndpointManager::new();
        let id = mgr.create(tenant, "p", "default", ip(10, 0, 1, 1));
        mgr.set_program_chain(id, canonical_egress_chain()).unwrap();
        assert_eq!(mgr.lookup(id).unwrap().program_chain.len(), 7);
    }

    // ── Count + serde ────────────────────────────────────────────────────────

    #[test]
    fn endpoint_count_tracks_creates() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.Count", "tenant-ep-cnt");
        let mut mgr = EndpointManager::new();
        for i in 0..5u32 {
            mgr.create(tenant.clone(), format!("p{i}"), "default", ip(10, 0, 1, i as u8 + 1));
        }
        assert_eq!(mgr.count(), 5);
    }

    #[test]
    fn endpoint_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("pkg/endpoint/endpoint.go", "Endpoint.Serde", "tenant-ep-serde");
        let mut ep = Endpoint::new_creating(1, tenant, "p", "default", ip(10, 0, 1, 1));
        ep.identity = 256;
        ep.labels = ls(&[("app", "web")]);
        ep.program_chain = canonical_egress_chain();
        let json = serde_json::to_string(&ep).unwrap();
        let back: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ep);
    }
}
