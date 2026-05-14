// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service-side data model + change tracker.
//!
//! Cite: `pkg/proxy/serviceport.go:35` (ServicePort interface),
//! `pkg/proxy/serviceport.go:77` (BaseServicePortInfo),
//! `pkg/proxy/types.go:44` (ServicePortName),
//! `pkg/proxy/config/config.go:166` (ServiceConfig — event source).

use crate::error::{KubeProxyError, KubeProxyResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

/// Cite: `pkg/proxy/types.go:44` (ServicePortName) — namespace + name +
/// port-name uniquely identify a Service port across the cluster.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ServicePortName {
    pub namespace: String,
    pub name: String,
    pub port: String,
}

impl ServicePortName {
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, port: impl Into<String>) -> Self {
        Self { namespace: namespace.into(), name: name.into(), port: port.into() }
    }

    /// Cite: `pkg/proxy/types.go:50` (ServicePortName.String) — formatted
    /// as `namespace/name:port`.
    pub fn key(&self) -> String {
        format!("{}/{}:{}", self.namespace, self.name, self.port)
    }
}

/// Cite: `pkg/proxy/serviceport.go` Protocol field (TCP/UDP/SCTP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol { Tcp, Udp, Sctp }

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self { Self::Tcp => "tcp", Self::Udp => "udp", Self::Sctp => "sctp" }
    }
}

/// Cite: `pkg/proxy/serviceport.go:43` (SessionAffinityType) — reflects
/// `v1.ServiceAffinityClientIP` vs `v1.ServiceAffinityNone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionAffinity { None, ClientIp }

/// Cite: `pkg/proxy/serviceport.go:154` (ExternalPolicyLocal) +
/// `:159` (InternalPolicyLocal) — the per-direction traffic policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrafficPolicy { Cluster, Local }

/// A trivially-parsed CIDR for LoadBalancerSourceRanges. IPv4 only;
/// IPv6 lands in a follow-up batch.
///
/// Cite: `pkg/proxy/serviceport.go:53` (LoadBalancerSourceRanges).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cidr {
    pub addr: Ipv4Addr,
    pub prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> KubeProxyResult<Self> {
        let (addr_part, prefix_part) = s.split_once('/').ok_or_else(|| {
            KubeProxyError::InvalidCidr(s.to_string(), "missing '/' separator".into())
        })?;
        let addr = Ipv4Addr::from_str(addr_part).map_err(|e| {
            KubeProxyError::InvalidCidr(s.to_string(), e.to_string())
        })?;
        let prefix: u8 = prefix_part.parse().map_err(|_| {
            KubeProxyError::InvalidCidr(s.to_string(), "non-numeric prefix".into())
        })?;
        if prefix > 32 {
            return Err(KubeProxyError::InvalidCidr(s.to_string(), "prefix > 32".into()));
        }
        Ok(Self { addr, prefix })
    }

    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        if self.prefix == 0 {
            return true;
        }
        let net = u32::from(self.addr);
        let cand = u32::from(ip);
        let shift = 32u32 - self.prefix as u32;
        (net >> shift) == (cand >> shift)
    }

    pub fn to_string_canonical(&self) -> String {
        format!("{}/{}", self.addr, self.prefix)
    }
}

/// Cite: `pkg/proxy/serviceport.go:77` (BaseServicePortInfo) — the
/// canonical per-port snapshot consumed by the proxier. Cave adds
/// `tenant_id` for namespace isolation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePortInfo {
    pub tenant_id: String,
    pub name: ServicePortName,
    pub cluster_ip: IpAddr,
    pub port: u16,
    pub protocol: Protocol,
    pub node_port: Option<u16>,
    pub health_check_node_port: Option<u16>,
    pub session_affinity: SessionAffinity,
    /// Cite: `pkg/proxy/serviceport.go:188`
    /// (`stickyMaxAgeSeconds = SessionAffinityConfig.ClientIP.TimeoutSeconds`).
    /// Defaults to 10800s (3h) on the apiserver side; copy verbatim.
    pub sticky_max_age_seconds: Option<u32>,
    pub load_balancer_source_ranges: Vec<Cidr>,
    pub external_traffic_policy: TrafficPolicy,
    pub internal_traffic_policy: TrafficPolicy,
    pub external_ips: Vec<IpAddr>,
    pub load_balancer_vips: Vec<IpAddr>,
}

impl ServicePortInfo {
    pub fn cluster_ip_only(
        tenant_id: impl Into<String>,
        name: ServicePortName,
        cluster_ip: IpAddr,
        port: u16,
        protocol: Protocol,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            name,
            cluster_ip,
            port,
            protocol,
            node_port: None,
            health_check_node_port: None,
            session_affinity: SessionAffinity::None,
            sticky_max_age_seconds: None,
            load_balancer_source_ranges: Vec::new(),
            external_traffic_policy: TrafficPolicy::Cluster,
            internal_traffic_policy: TrafficPolicy::Cluster,
            external_ips: Vec::new(),
            load_balancer_vips: Vec::new(),
        }
    }

    /// Cite: `pkg/proxy/util/utils.go:55` (ShouldSkipService) — Services
    /// whose ClusterIP is empty / "None" must NOT be programmed into the
    /// proxier. cave maps this onto `cluster_ip == 0.0.0.0` as the
    /// "skip" sentinel since headless Services arrive without a VIP.
    pub fn should_skip(&self) -> bool {
        matches!(self.cluster_ip, IpAddr::V4(v4) if v4.is_unspecified())
    }

    /// Cite: `pkg/proxy/serviceport.go:124` (LoadBalancerSourceRanges)
    /// — empty list ⇒ everyone allowed; otherwise at least one CIDR
    /// must contain the source IP.
    pub fn allowed_by_source_ranges(&self, src: Ipv4Addr) -> bool {
        if self.load_balancer_source_ranges.is_empty() {
            return true;
        }
        self.load_balancer_source_ranges.iter().any(|c| c.contains(src))
    }
}

/// Cite: `pkg/proxy/config/config.go:166` (ServiceConfig) — the
/// upstream change tracker accumulates Add/Update/Delete events and
/// yields a snapshot of pending changes during `syncProxyRules`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceChange {
    pub previous: Option<ServicePortInfo>,
    pub current: Option<ServicePortInfo>,
}

impl ServiceChange {
    pub fn is_add(&self) -> bool { self.previous.is_none() && self.current.is_some() }
    pub fn is_delete(&self) -> bool { self.previous.is_some() && self.current.is_none() }
    pub fn is_update(&self) -> bool { self.previous.is_some() && self.current.is_some() }
}

#[derive(Debug, Clone)]
pub struct ServiceChangeTracker {
    pub tenant_id: String,
    pending: BTreeMap<ServicePortName, ServiceChange>,
}

impl ServiceChangeTracker {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), pending: BTreeMap::new() }
    }

    /// Cite: `pkg/proxy/config/config.go:212` (handleAddService) /
    /// `:224` (handleUpdateService) / `:241` (handleDeleteService).
    /// Cross-tenant updates are rejected: the tracker mirrors the
    /// `tenant_id` of its owning proxier.
    pub fn update(
        &mut self,
        previous: Option<ServicePortInfo>,
        current: Option<ServicePortInfo>,
    ) -> KubeProxyResult<()> {
        let svc = previous.as_ref().or(current.as_ref())
            .map(|s| (s.name.clone(), s.tenant_id.clone()));
        let (name, t) = match svc {
            Some(x) => x,
            None => return Ok(()),
        };
        if t != self.tenant_id {
            return Err(KubeProxyError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: t,
            });
        }
        // Coalesce: if a previous pending change exists, keep its
        // `previous` and overwrite `current`. If the net effect is a
        // no-op (previous == current after coalescing), drop the entry.
        let coalesced = match self.pending.remove(&name) {
            Some(prev_change) => ServiceChange {
                previous: prev_change.previous,
                current,
            },
            None => ServiceChange { previous, current },
        };
        if coalesced.previous.is_none() && coalesced.current.is_none() {
            return Ok(());
        }
        self.pending.insert(name, coalesced);
        Ok(())
    }

    pub fn pending(&self) -> Vec<&ServiceChange> {
        self.pending.values().collect()
    }

    pub fn pending_for(&self, name: &ServicePortName) -> Option<&ServiceChange> {
        self.pending.get(name)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn drain(&mut self) -> HashMap<ServicePortName, ServiceChange> {
        std::mem::take(&mut self.pending).into_iter().collect()
    }
}
