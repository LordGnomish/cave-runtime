// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CiliumExternalWorkload — non-K8s pod onboarding (VMs, bare-metal).
//!
//! Mirrors `pkg/clustermesh/externalworkloads/manager.go` plus the
//! `CiliumExternalWorkload` CRD shape from
//! `pkg/k8s/apis/cilium.io/v2/types.go`.
//!
//! An external workload joins the Cilium mesh as a first-class
//! endpoint: it gets an identity, an IPAM-allocated IP, and is
//! visible to NetworkPolicy + Hubble like any in-cluster pod. The
//! workload runs the cilium-agent in "external" mode and pulls config
//! from the cluster.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkloadState {
    /// CRD registered; waiting for the workload to call home.
    Pending,
    /// Workload is connected and exchanging config.
    Connected,
    /// Workload has registered an identity + IP; eligible for policy.
    Ready,
    /// Workload missed too many heartbeats.
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkloadSpec {
    pub name: String,
    pub tenant: TenantId,
    pub ipv4: Option<IpAddr>,
    pub ipv6: Option<IpAddr>,
    pub labels: BTreeMap<String, String>,
    pub trust_domain: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkload {
    pub spec: ExternalWorkloadSpec,
    pub state: WorkloadState,
    pub identity: Option<u32>,
    pub last_heartbeat_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExternalWorkloadError {
    #[error("workload `{0}` already registered")]
    Duplicate(String),
    #[error("workload `{0}` not found")]
    NotFound(String),
    #[error("invalid state transition {from:?} → {to:?}")]
    BadTransition {
        from: WorkloadState,
        to: WorkloadState,
    },
    #[error("workload `{0}` has no IPv4 nor IPv6 address")]
    NoAddress(String),
    #[error("tenant {tenant} cannot mutate workload owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ExternalWorkloadManager {
    pub tenant: TenantId,
    pub stale_threshold_ns: u64,
    workloads: HashMap<String, ExternalWorkload>,
    /// (ipv4 or ipv6) → workload name for fast IP lookups.
    by_ip: HashMap<IpAddr, String>,
}

impl ExternalWorkloadManager {
    pub fn new(tenant: TenantId, stale_threshold_seconds: u64) -> Self {
        Self {
            tenant,
            stale_threshold_ns: stale_threshold_seconds * 1_000_000_000,
            workloads: HashMap::new(),
            by_ip: HashMap::new(),
        }
    }

    pub fn register(&mut self, spec: ExternalWorkloadSpec) -> Result<(), ExternalWorkloadError> {
        if spec.tenant != self.tenant {
            return Err(ExternalWorkloadError::TenantDenied {
                tenant: spec.tenant,
            });
        }
        if spec.ipv4.is_none() && spec.ipv6.is_none() {
            return Err(ExternalWorkloadError::NoAddress(spec.name));
        }
        if self.workloads.contains_key(&spec.name) {
            return Err(ExternalWorkloadError::Duplicate(spec.name));
        }
        if let Some(ip) = spec.ipv4 {
            self.by_ip.insert(ip, spec.name.clone());
        }
        if let Some(ip) = spec.ipv6 {
            self.by_ip.insert(ip, spec.name.clone());
        }
        self.workloads.insert(
            spec.name.clone(),
            ExternalWorkload {
                spec,
                state: WorkloadState::Pending,
                identity: None,
                last_heartbeat_ns: 0,
            },
        );
        Ok(())
    }

    pub fn deregister(&mut self, name: &str) -> Result<(), ExternalWorkloadError> {
        let w = self
            .workloads
            .remove(name)
            .ok_or_else(|| ExternalWorkloadError::NotFound(name.to_string()))?;
        if let Some(ip) = w.spec.ipv4 {
            self.by_ip.remove(&ip);
        }
        if let Some(ip) = w.spec.ipv6 {
            self.by_ip.remove(&ip);
        }
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&ExternalWorkload> {
        self.workloads.get(name)
    }

    pub fn lookup_by_ip(&self, ip: IpAddr) -> Option<&ExternalWorkload> {
        let name = self.by_ip.get(&ip)?;
        self.workloads.get(name)
    }

    /// Drive the state machine. Mirrors upstream allowed transitions.
    pub fn transition(
        &mut self,
        name: &str,
        to: WorkloadState,
    ) -> Result<(), ExternalWorkloadError> {
        let w = self
            .workloads
            .get_mut(name)
            .ok_or_else(|| ExternalWorkloadError::NotFound(name.to_string()))?;
        let from = w.state;
        let ok = matches!(
            (from, to),
            (WorkloadState::Pending, WorkloadState::Connected)
                | (WorkloadState::Connected, WorkloadState::Ready)
                | (WorkloadState::Ready, WorkloadState::Stale)
                | (WorkloadState::Stale, WorkloadState::Connected)
                | (WorkloadState::Stale, WorkloadState::Ready)
                | (WorkloadState::Connected, WorkloadState::Pending)
        );
        if !ok {
            return Err(ExternalWorkloadError::BadTransition { from, to });
        }
        w.state = to;
        Ok(())
    }

    pub fn assign_identity(
        &mut self,
        name: &str,
        identity: u32,
    ) -> Result<(), ExternalWorkloadError> {
        let w = self
            .workloads
            .get_mut(name)
            .ok_or_else(|| ExternalWorkloadError::NotFound(name.to_string()))?;
        w.identity = Some(identity);
        Ok(())
    }

    pub fn heartbeat(&mut self, name: &str, now_ns: u64) -> Result<(), ExternalWorkloadError> {
        let w = self
            .workloads
            .get_mut(name)
            .ok_or_else(|| ExternalWorkloadError::NotFound(name.to_string()))?;
        w.last_heartbeat_ns = now_ns;
        if matches!(w.state, WorkloadState::Stale) {
            w.state = WorkloadState::Ready;
        }
        Ok(())
    }

    /// Sweep workloads whose heartbeat is older than the stale threshold.
    /// Returns the count transitioned to Stale.
    pub fn sweep_stale(&mut self, now_ns: u64) -> usize {
        let mut n = 0;
        for w in self.workloads.values_mut() {
            if !matches!(w.state, WorkloadState::Ready) {
                continue;
            }
            let elapsed = now_ns.saturating_sub(w.last_heartbeat_ns);
            if elapsed >= self.stale_threshold_ns {
                w.state = WorkloadState::Stale;
                n += 1;
            }
        }
        n
    }

    pub fn count(&self) -> usize {
        self.workloads.len()
    }

    pub fn ready_count(&self) -> usize {
        self.workloads
            .values()
            .filter(|w| matches!(w.state, WorkloadState::Ready))
            .count()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/clustermesh/externalworkloads/manager.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn spec(name: &str, tenant: TenantId, v4: Option<IpAddr>) -> ExternalWorkloadSpec {
        ExternalWorkloadSpec {
            name: name.into(),
            tenant,
            ipv4: v4,
            ipv6: None,
            labels: BTreeMap::from([("env".into(), "edge".into())]),
            trust_domain: "cluster.local".into(),
        }
    }

    fn mgr(tenant: TenantId) -> ExternalWorkloadManager {
        ExternalWorkloadManager::new(tenant, 30)
    }

    // ── Registration ─────────────────────────────────────────────────────────

    #[test]
    fn ew_register_succeeds_with_v4() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Register",
            "tenant-ew-reg"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn ew_register_duplicate_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Register.Duplicate",
            "tenant-ew-dup"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant.clone(), Some(ip(192, 168, 1, 10))))
            .unwrap();
        let err = m
            .register(spec("vm-1", tenant, Some(ip(192, 168, 1, 11))))
            .unwrap_err();
        assert!(matches!(err, ExternalWorkloadError::Duplicate(_)));
    }

    #[test]
    fn ew_register_no_address_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Register.NoAddr",
            "tenant-ew-noaddr"
        );
        let mut m = mgr(tenant.clone());
        let err = m.register(spec("vm-1", tenant, None)).unwrap_err();
        assert!(matches!(err, ExternalWorkloadError::NoAddress(_)));
    }

    #[test]
    fn ew_register_cross_tenant_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Register.Tenant",
            "tenant-ew-xt"
        );
        let mut m = mgr(tenant);
        let other = TenantId::new("tenant-ew-xt-other").expect("test fixture");
        let err = m
            .register(spec("vm-1", other, Some(ip(192, 168, 1, 10))))
            .unwrap_err();
        assert!(matches!(err, ExternalWorkloadError::TenantDenied { .. }));
    }

    // ── Lookup ──────────────────────────────────────────────────────────────

    #[test]
    fn ew_lookup_by_name() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Lookup.Name",
            "tenant-ew-lkn"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        let w = m.lookup("vm-1").unwrap();
        assert_eq!(w.spec.ipv4, Some(ip(192, 168, 1, 10)));
    }

    #[test]
    fn ew_lookup_by_ip() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Lookup.IP",
            "tenant-ew-lkip"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        let w = m.lookup_by_ip(ip(192, 168, 1, 10)).unwrap();
        assert_eq!(w.spec.name, "vm-1");
    }

    #[test]
    fn ew_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Lookup.NotFound",
            "tenant-ew-lknf"
        );
        let m = mgr(tenant);
        assert!(m.lookup("ghost").is_none());
        assert!(m.lookup_by_ip(ip(8, 8, 8, 8)).is_none());
    }

    // ── State machine ───────────────────────────────────────────────────────

    #[test]
    fn ew_initial_state_pending() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "State.Initial",
            "tenant-ew-pend"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        assert_eq!(m.lookup("vm-1").unwrap().state, WorkloadState::Pending);
    }

    #[test]
    fn ew_transition_pending_to_connected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "State.Connected",
            "tenant-ew-conn"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.transition("vm-1", WorkloadState::Connected).unwrap();
        assert_eq!(m.lookup("vm-1").unwrap().state, WorkloadState::Connected);
    }

    #[test]
    fn ew_transition_connected_to_ready() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "State.Ready",
            "tenant-ew-rdy"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.transition("vm-1", WorkloadState::Connected).unwrap();
        m.transition("vm-1", WorkloadState::Ready).unwrap();
        assert_eq!(m.ready_count(), 1);
    }

    #[test]
    fn ew_transition_ready_to_stale_to_ready() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "State.Recovery",
            "tenant-ew-rec"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.transition("vm-1", WorkloadState::Connected).unwrap();
        m.transition("vm-1", WorkloadState::Ready).unwrap();
        m.transition("vm-1", WorkloadState::Stale).unwrap();
        m.transition("vm-1", WorkloadState::Ready).unwrap();
        assert_eq!(m.lookup("vm-1").unwrap().state, WorkloadState::Ready);
    }

    #[test]
    fn ew_invalid_transition_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "State.BadTransition",
            "tenant-ew-bt"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        let err = m.transition("vm-1", WorkloadState::Ready).unwrap_err();
        assert!(matches!(err, ExternalWorkloadError::BadTransition { .. }));
    }

    // ── Identity ────────────────────────────────────────────────────────────

    #[test]
    fn ew_assign_identity() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "AssignIdentity",
            "tenant-ew-aid"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.assign_identity("vm-1", 1024).unwrap();
        assert_eq!(m.lookup("vm-1").unwrap().identity, Some(1024));
    }

    #[test]
    fn ew_assign_identity_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "AssignIdentity.NotFound",
            "tenant-ew-aidnf"
        );
        let mut m = mgr(tenant);
        let err = m.assign_identity("ghost", 1).unwrap_err();
        assert!(matches!(err, ExternalWorkloadError::NotFound(_)));
    }

    // ── Heartbeat ───────────────────────────────────────────────────────────

    #[test]
    fn ew_heartbeat_updates_timestamp() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Heartbeat",
            "tenant-ew-hb"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.heartbeat("vm-1", 1234).unwrap();
        assert_eq!(m.lookup("vm-1").unwrap().last_heartbeat_ns, 1234);
    }

    #[test]
    fn ew_heartbeat_recovers_stale_to_ready() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Heartbeat.Recovery",
            "tenant-ew-hbr"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.transition("vm-1", WorkloadState::Connected).unwrap();
        m.transition("vm-1", WorkloadState::Ready).unwrap();
        m.transition("vm-1", WorkloadState::Stale).unwrap();
        m.heartbeat("vm-1", 5000).unwrap();
        assert_eq!(m.lookup("vm-1").unwrap().state, WorkloadState::Ready);
    }

    // ── Sweep ────────────────────────────────────────────────────────────────

    #[test]
    fn ew_sweep_marks_stale_after_threshold() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Sweep",
            "tenant-ew-sw"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.transition("vm-1", WorkloadState::Connected).unwrap();
        m.transition("vm-1", WorkloadState::Ready).unwrap();
        m.heartbeat("vm-1", 0).unwrap();
        let n = m.sweep_stale(40_000_000_000); // 40s, threshold 30s
        assert_eq!(n, 1);
        assert_eq!(m.lookup("vm-1").unwrap().state, WorkloadState::Stale);
    }

    #[test]
    fn ew_sweep_keeps_recent_heartbeat() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Sweep.Fresh",
            "tenant-ew-swf"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.transition("vm-1", WorkloadState::Connected).unwrap();
        m.transition("vm-1", WorkloadState::Ready).unwrap();
        m.heartbeat("vm-1", 0).unwrap();
        let n = m.sweep_stale(10_000_000_000); // 10s, threshold 30s
        assert_eq!(n, 0);
    }

    #[test]
    fn ew_sweep_skips_non_ready_workloads() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Sweep.OnlyReady",
            "tenant-ew-swr"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        // Pending — sweep should skip.
        let n = m.sweep_stale(40_000_000_000);
        assert_eq!(n, 0);
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn ew_deregister_drops_workload_and_ip_index() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Deregister",
            "tenant-ew-rm"
        );
        let mut m = mgr(tenant.clone());
        m.register(spec("vm-1", tenant, Some(ip(192, 168, 1, 10))))
            .unwrap();
        m.deregister("vm-1").unwrap();
        assert_eq!(m.count(), 0);
        assert!(m.lookup_by_ip(ip(192, 168, 1, 10)).is_none());
    }

    #[test]
    fn ew_deregister_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "Deregister.NotFound",
            "tenant-ew-rmnf"
        );
        let mut m = mgr(tenant);
        let err = m.deregister("ghost").unwrap_err();
        assert!(matches!(err, ExternalWorkloadError::NotFound(_)));
    }

    // ── Counts ──────────────────────────────────────────────────────────────

    #[test]
    fn ew_ready_count_only_counts_ready_state() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "ReadyCount",
            "tenant-ew-rdc"
        );
        let mut m = mgr(tenant.clone());
        for i in 0..5u8 {
            m.register(spec(
                &format!("vm-{i}"),
                tenant.clone(),
                Some(ip(192, 168, 1, i)),
            ))
            .unwrap();
            if i < 3 {
                m.transition(&format!("vm-{i}"), WorkloadState::Connected)
                    .unwrap();
                m.transition(&format!("vm-{i}"), WorkloadState::Ready)
                    .unwrap();
            }
        }
        assert_eq!(m.ready_count(), 3);
        assert_eq!(m.count(), 5);
    }

    // ── IPv6 ────────────────────────────────────────────────────────────────

    #[test]
    fn ew_dual_stack_workload_indexed_by_both_addrs() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "DualStack",
            "tenant-ew-ds"
        );
        let mut m = mgr(tenant.clone());
        let mut s = spec("vm-1", tenant, Some(ip(192, 168, 1, 10)));
        s.ipv6 = Some("fd00::1".parse().unwrap());
        m.register(s).unwrap();
        assert!(m.lookup_by_ip(ip(192, 168, 1, 10)).is_some());
        assert!(m.lookup_by_ip("fd00::1".parse().unwrap()).is_some());
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn ew_workload_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2/types.go",
            "ExternalWorkload.Serde",
            "tenant-ew-serde"
        );
        let s = spec("vm-1", tenant, Some(ip(192, 168, 1, 10)));
        let json = serde_json::to_string(&s).unwrap();
        let back: ExternalWorkloadSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn ew_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/clustermesh/externalworkloads/manager.go",
            "State.Serde",
            "tenant-ew-stserde"
        );
        for s in [
            WorkloadState::Pending,
            WorkloadState::Connected,
            WorkloadState::Ready,
            WorkloadState::Stale,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            let back: WorkloadState = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }
}
