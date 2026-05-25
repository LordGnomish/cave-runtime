// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service controller — clusterIP CIDR allocator + LoadBalancer reconciler
//! with finalizer ordering.
//!
//! Mirrors `pkg/registry/core/service/ipallocator/allocator.go` plus the
//! LB reconcile body in `pkg/controller/service/controller.go`.
//!
//! `IpAllocator` walks an IPv4 `/N` CIDR sequentially, skipping the
//! network and broadcast addresses (matching upstream's behaviour),
//! returning the first free `u32` IP. `release` returns the IP to the pool
//! and may be re-allocated.

use crate::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IpError {
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("CIDR `/{0}` is too narrow — at least /30 (4 addresses) required")]
    CidrTooNarrow(u8),
    #[error("CIDR /{0} is wider than /16 — refused for safety")]
    CidrTooWide(u8),
    #[error("clusterIP {0} is outside the configured CIDR")]
    OutsideCidr(u32),
    #[error("clusterIP pool exhausted")]
    Exhausted,
    #[error("ip {0} is not currently allocated")]
    NotAllocated(u32),
}

/// IPv4 sequential allocator. Returns the lowest free address in the CIDR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpAllocator {
    pub network: u32,
    pub broadcast: u32,
    pub prefix: u8,
    allocated: BTreeSet<u32>,
}

impl IpAllocator {
    pub fn new(cidr: &str) -> Result<Self, IpError> {
        let (addr, prefix) = parse_cidr(cidr)?;
        if prefix < 16 {
            return Err(IpError::CidrTooWide(prefix));
        }
        if prefix > 30 {
            return Err(IpError::CidrTooNarrow(prefix));
        }
        let mask: u32 = if prefix == 0 {
            0
        } else {
            !0u32 << (32 - prefix)
        };
        let network = addr & mask;
        let broadcast = network | !mask;
        Ok(Self {
            network,
            broadcast,
            prefix,
            allocated: BTreeSet::new(),
        })
    }

    /// Allocate the lowest free usable address (skipping network + broadcast).
    pub fn allocate(&mut self) -> Result<u32, IpError> {
        for ip in (self.network + 1)..self.broadcast {
            if !self.allocated.contains(&ip) {
                self.allocated.insert(ip);
                return Ok(ip);
            }
        }
        Err(IpError::Exhausted)
    }

    /// Allocate a specific address — must be inside the CIDR and free.
    pub fn allocate_specific(&mut self, ip: u32) -> Result<(), IpError> {
        if ip <= self.network || ip >= self.broadcast {
            return Err(IpError::OutsideCidr(ip));
        }
        if !self.allocated.insert(ip) {
            return Err(IpError::Exhausted);
        }
        Ok(())
    }

    pub fn release(&mut self, ip: u32) -> Result<(), IpError> {
        if !self.allocated.remove(&ip) {
            return Err(IpError::NotAllocated(ip));
        }
        Ok(())
    }

    pub fn count_allocated(&self) -> usize {
        self.allocated.len()
    }
    pub fn capacity(&self) -> u32 {
        self.broadcast
            .saturating_sub(self.network)
            .saturating_sub(1)
    }
}

fn parse_cidr(s: &str) -> Result<(u32, u8), IpError> {
    let (ip, pfx) = s
        .split_once('/')
        .ok_or_else(|| IpError::BadCidr(s.into()))?;
    let prefix: u8 = pfx.parse().map_err(|_| IpError::BadCidr(s.into()))?;
    if prefix > 32 {
        return Err(IpError::BadCidr(s.into()));
    }
    let octets: Vec<&str> = ip.split('.').collect();
    if octets.len() != 4 {
        return Err(IpError::BadCidr(s.into()));
    }
    let mut addr: u32 = 0;
    for o in octets {
        let n: u8 = o.parse().map_err(|_| IpError::BadCidr(s.into()))?;
        addr = (addr << 8) | n as u32;
    }
    Ok((addr, prefix))
}

pub fn ip_to_dotted(ip: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        (ip >> 24) & 0xff,
        (ip >> 16) & 0xff,
        (ip >> 8) & 0xff,
        ip & 0xff
    )
}

// ── LoadBalancer reconciler ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceObject {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub service_type: ServiceType,
    pub cluster_ip: Option<u32>,
    pub external_ip: Option<String>,
    pub finalizer_present: bool,
    pub deletion_timestamp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbStep {
    /// Allocate a clusterIP from the pool.
    AllocateClusterIp,
    /// Add the cloud finalizer (must precede LB provisioning).
    AddFinalizer,
    /// Provision the cloud LB and assign external_ip.
    ProvisionLb,
    /// Tear down the cloud LB.
    TeardownLb,
    /// Remove the finalizer (only after LB is gone).
    RemoveFinalizer,
    NoOp,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LbError {
    #[error("tenant {tenant} cannot drive service owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Decide the next reconciliation step. Mirrors `syncLoadBalancerIfNeeded`.
///
/// Order of operations matters:
/// - Add finalizer **before** ProvisionLb (so a delete during provisioning
///   can't strand the cloud resource).
/// - On delete: TeardownLb first, then RemoveFinalizer.
pub fn next_step(svc: &ServiceObject, caller: &TenantId) -> Result<LbStep, LbError> {
    if caller != &svc.tenant {
        return Err(LbError::TenantDenied {
            tenant: caller.clone(),
        });
    }
    if svc.cluster_ip.is_none() {
        return Ok(LbStep::AllocateClusterIp);
    }
    if svc.deletion_timestamp {
        if svc.external_ip.is_some() {
            return Ok(LbStep::TeardownLb);
        }
        if svc.finalizer_present {
            return Ok(LbStep::RemoveFinalizer);
        }
        return Ok(LbStep::NoOp);
    }
    if svc.service_type != ServiceType::LoadBalancer {
        return Ok(LbStep::NoOp);
    }
    if !svc.finalizer_present {
        return Ok(LbStep::AddFinalizer);
    }
    if svc.external_ip.is_none() {
        return Ok(LbStep::ProvisionLb);
    }
    Ok(LbStep::NoOp)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/registry/core/service/ipallocator/allocator.go",
    "Range.AllocateNext",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn svc(name: &str, tenant: &str, t: ServiceType) -> ServiceObject {
        ServiceObject {
            name: name.into(),
            namespace: "default".into(),
            tenant: TenantId::new(tenant).expect("test fixture"),
            service_type: t,
            cluster_ip: None,
            external_ip: None,
            finalizer_present: false,
            deletion_timestamp: false,
        }
    }

    #[test]
    fn allocator_skips_network_and_broadcast() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "Range.AllocateNext",
            "tenant-svc-skip"
        );
        let mut a = IpAllocator::new("10.96.0.0/30").unwrap();
        // /30 = 4 addresses; 1 network + 1 broadcast → 2 usable.
        let first = a.allocate().unwrap();
        let second = a.allocate().unwrap();
        assert!(first > a.network && first < a.broadcast);
        assert!(second > first);
        assert_eq!(a.allocate(), Err(IpError::Exhausted));
    }

    #[test]
    fn allocator_release_returns_ip_to_pool() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "Range.Release",
            "tenant-svc-release"
        );
        let mut a = IpAllocator::new("10.96.0.0/29").unwrap();
        let ip = a.allocate().unwrap();
        a.release(ip).unwrap();
        assert_eq!(a.count_allocated(), 0);
        let again = a.allocate().unwrap();
        assert_eq!(again, ip);
    }

    #[test]
    fn allocate_specific_refuses_outside_cidr() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "Range.AllocateService",
            "tenant-svc-outside"
        );
        let mut a = IpAllocator::new("10.96.0.0/29").unwrap();
        let outside = (192u32 << 24) | (168 << 16); // 192.168.0.0
        assert!(matches!(
            a.allocate_specific(outside),
            Err(IpError::OutsideCidr(_))
        ));
    }

    #[test]
    fn allocate_specific_refuses_already_allocated() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "Range.AllocateService",
            "tenant-svc-double"
        );
        let mut a = IpAllocator::new("10.96.0.0/29").unwrap();
        let ip = a.allocate().unwrap();
        assert_eq!(a.allocate_specific(ip), Err(IpError::Exhausted));
    }

    #[test]
    fn release_unknown_returns_not_allocated() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "Range.Release",
            "tenant-svc-release-unknown"
        );
        let mut a = IpAllocator::new("10.96.0.0/29").unwrap();
        assert!(matches!(
            a.release(0x0a600005),
            Err(IpError::NotAllocated(_))
        ));
    }

    #[test]
    fn cidr_too_narrow_or_too_wide_is_rejected() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "NewAllocatorCIDRRange",
            "tenant-svc-cidr-bounds"
        );
        assert!(matches!(
            IpAllocator::new("10.0.0.0/31"),
            Err(IpError::CidrTooNarrow(31))
        ));
        assert!(matches!(
            IpAllocator::new("10.0.0.0/8"),
            Err(IpError::CidrTooWide(8))
        ));
    }

    #[test]
    fn ip_to_dotted_round_trips_basic_addresses() {
        let (_cite, _t) = test_ctx!(
            "pkg/registry/core/service/ipallocator/allocator.go",
            "intToIPv4",
            "tenant-svc-dotted"
        );
        assert_eq!(ip_to_dotted(0x0a600001), "10.96.0.1");
        assert_eq!(ip_to_dotted(0xc0a80001), "192.168.0.1");
    }

    #[test]
    fn lb_reconciler_allocates_cluster_ip_first() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "ensureClusterIP",
            "acme"
        );
        let s = svc("web", "acme", ServiceType::LoadBalancer);
        assert_eq!(next_step(&s, &tenant).unwrap(), LbStep::AllocateClusterIp);
    }

    #[test]
    fn lb_reconciler_adds_finalizer_before_provisioning() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let mut s = svc("web", "acme", ServiceType::LoadBalancer);
        s.cluster_ip = Some(0x0a600001);
        assert_eq!(next_step(&s, &tenant).unwrap(), LbStep::AddFinalizer);
        s.finalizer_present = true;
        assert_eq!(next_step(&s, &tenant).unwrap(), LbStep::ProvisionLb);
    }

    #[test]
    fn lb_reconciler_teardown_then_finalizer_remove_on_delete() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "ensureLoadBalancerDeleted",
            "acme"
        );
        let mut s = svc("web", "acme", ServiceType::LoadBalancer);
        s.cluster_ip = Some(0x0a600001);
        s.finalizer_present = true;
        s.external_ip = Some("203.0.113.1".into());
        s.deletion_timestamp = true;
        assert_eq!(next_step(&s, &tenant).unwrap(), LbStep::TeardownLb);
        s.external_ip = None;
        assert_eq!(next_step(&s, &tenant).unwrap(), LbStep::RemoveFinalizer);
    }

    #[test]
    fn cluster_ip_only_service_does_not_provision_lb() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "wantsLoadBalancer",
            "acme"
        );
        let mut s = svc("api", "acme", ServiceType::ClusterIP);
        s.cluster_ip = Some(0x0a600002);
        assert_eq!(next_step(&s, &tenant).unwrap(), LbStep::NoOp);
    }

    #[test]
    fn cross_tenant_caller_is_refused() {
        let (_cite, attacker) = test_ctx!(
            "pkg/controller/service/controller.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let s = svc("web", "acme", ServiceType::LoadBalancer);
        let err = next_step(&s, &attacker).unwrap_err();
        assert!(matches!(err, LbError::TenantDenied { .. }));
    }
}
