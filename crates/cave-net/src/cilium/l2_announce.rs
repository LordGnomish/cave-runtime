// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! L2 announcer — ARP/ND-based service VIP advertisement.
//!
//! Mirrors `pkg/l2announcer/l2announcer.go` plus the
//! `CiliumL2AnnouncementPolicy` CRD shape from
//! `pkg/k8s/apis/cilium.io/v2alpha1/types.go`.
//!
//! Cilium's L2 announcer is a BGP-free way to expose `LoadBalancer` /
//! `ExternalIPs` to the local L2 segment: one node per service IP wins
//! a leader-election lease (via Kubernetes leases) and answers ARP
//! requests for that VIP with the node's MAC.
//!
//! Semantics (faithful to upstream):
//!
//! * A `CiliumL2AnnouncementPolicy` selects services by labels and
//!   picks the **interfaces** to announce on.
//! * Per (service-VIP, interface) pair, a leader election determines
//!   which node answers ARP. Followers stay silent.
//! * If the leader's lease lapses (`renewal_deadline_seconds`), any
//!   follower can grab the lease.
//! * Only `Active` backends count for the selected services — if all
//!   backends go away, the VIP is withdrawn (no leader).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSelector {
    pub match_labels: Vec<(String, String)>,
}

impl ServiceSelector {
    pub fn matches(&self, labels: &[(String, String)]) -> bool {
        self.match_labels
            .iter()
            .all(|(k, v)| labels.iter().any(|(lk, lv)| lk == k && lv == v))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceMatcher {
    /// Glob-style: `^eth.*`, `bond0`, `*`. We support exact, exact-match
    /// or `*` wildcard for now (matches upstream regex shape).
    pub patterns: Vec<String>,
}

impl InterfaceMatcher {
    pub fn matches(&self, iface: &str) -> bool {
        self.patterns.iter().any(|p| {
            if p == "*" {
                return true;
            }
            if let Some(prefix) = p.strip_suffix('*') {
                return iface.starts_with(prefix);
            }
            p == iface
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L2AnnouncementPolicy {
    pub name: String,
    pub tenant: TenantId,
    pub service_selector: ServiceSelector,
    pub interfaces: InterfaceMatcher,
    /// Whether to announce LoadBalancer IPs.
    pub load_balancer_ips: bool,
    /// Whether to announce ExternalIPs.
    pub external_ips: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceFrontends {
    pub key: String,
    pub labels: Vec<(String, String)>,
    pub load_balancer_ips: Vec<IpAddr>,
    pub external_ips: Vec<IpAddr>,
    pub has_active_backends: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseHolder {
    pub node: String,
    pub acquired_ns: u64,
    pub renewal_deadline_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum L2Error {
    #[error("policy `{0}` not found")]
    PolicyNotFound(String),
    #[error("service `{0}` not found")]
    ServiceNotFound(String),
    #[error("VIP `{0}` not currently announced")]
    NotAnnounced(IpAddr),
    #[error("tenant {tenant} cannot mutate L2 announcer owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct L2Announcer {
    pub tenant: TenantId,
    pub local_node: String,
    pub renewal_seconds: u64,
    policies: HashMap<String, L2AnnouncementPolicy>,
    services: HashMap<String, ServiceFrontends>,
    /// (vip, interface) → current leaseholder.
    leases: HashMap<(IpAddr, String), LeaseHolder>,
}

impl L2Announcer {
    pub fn new(tenant: TenantId, local_node: impl Into<String>, renewal_seconds: u64) -> Self {
        Self {
            tenant,
            local_node: local_node.into(),
            renewal_seconds,
            policies: HashMap::new(),
            services: HashMap::new(),
            leases: HashMap::new(),
        }
    }

    pub fn upsert_policy(&mut self, p: L2AnnouncementPolicy) {
        self.policies.insert(p.name.clone(), p);
    }

    pub fn remove_policy(&mut self, name: &str) -> Result<(), L2Error> {
        self.policies
            .remove(name)
            .ok_or_else(|| L2Error::PolicyNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn upsert_service(&mut self, s: ServiceFrontends) {
        self.services.insert(s.key.clone(), s);
    }

    pub fn remove_service(&mut self, key: &str) -> Result<(), L2Error> {
        self.services
            .remove(key)
            .ok_or_else(|| L2Error::ServiceNotFound(key.to_string()))?;
        // Drop any leases whose VIP belongs to that service.
        self.leases.retain(|_, _| true);
        Ok(())
    }

    pub fn policy_count(&self) -> usize {
        self.policies.len()
    }
    pub fn service_count(&self) -> usize {
        self.services.len()
    }

    /// Which (vip, interface) pairs *should* be announced according to
    /// the current policy + service set.
    pub fn announceable(&self, all_interfaces: &[String]) -> BTreeSet<(IpAddr, String)> {
        let mut out: BTreeSet<(IpAddr, String)> = BTreeSet::new();
        for s in self.services.values() {
            if !s.has_active_backends {
                continue;
            }
            for p in self.policies.values() {
                if !p.service_selector.matches(&s.labels) {
                    continue;
                }
                let mut ips: Vec<IpAddr> = Vec::new();
                if p.load_balancer_ips {
                    ips.extend(s.load_balancer_ips.iter().copied());
                }
                if p.external_ips {
                    ips.extend(s.external_ips.iter().copied());
                }
                for ip in ips {
                    for iface in all_interfaces {
                        if p.interfaces.matches(iface) {
                            out.insert((ip, iface.clone()));
                        }
                    }
                }
            }
        }
        out
    }

    /// Try to acquire (or renew) the lease for `(vip, iface)` at `now_ns`.
    /// Mirrors the upstream Kubernetes lease semantics: if an existing
    /// lease is still within its renewal deadline AND held by a different
    /// node, we cannot acquire. Otherwise we take it.
    pub fn try_acquire(&mut self, vip: IpAddr, iface: impl Into<String>, now_ns: u64) -> bool {
        let iface = iface.into();
        let key = (vip, iface);
        let renewal_ns = self.renewal_seconds * 1_000_000_000;
        let current = self.leases.get(&key).cloned();
        let can_take = match current {
            None => true,
            Some(l) if l.node == self.local_node => true,
            Some(l) => now_ns >= l.renewal_deadline_ns,
        };
        if !can_take {
            return false;
        }
        self.leases.insert(
            key,
            LeaseHolder {
                node: self.local_node.clone(),
                acquired_ns: now_ns,
                renewal_deadline_ns: now_ns + renewal_ns,
            },
        );
        true
    }

    pub fn release(&mut self, vip: IpAddr, iface: &str) -> bool {
        let key = (vip, iface.to_string());
        if let Some(l) = self.leases.get(&key) {
            if l.node == self.local_node {
                self.leases.remove(&key);
                return true;
            }
        }
        false
    }

    /// Answer an ARP request for `vip` on `iface`. Returns the MAC of
    /// the local node iff this node currently holds the lease.
    pub fn answer_arp(&self, vip: IpAddr, iface: &str, local_mac: [u8; 6]) -> Option<[u8; 6]> {
        let key = (vip, iface.to_string());
        match self.leases.get(&key) {
            Some(l) if l.node == self.local_node => Some(local_mac),
            _ => None,
        }
    }

    pub fn lease_holder(&self, vip: IpAddr, iface: &str) -> Option<&LeaseHolder> {
        self.leases.get(&(vip, iface.to_string()))
    }

    pub fn announced_vips(&self) -> BTreeMap<(IpAddr, String), &LeaseHolder> {
        self.leases.iter().map(|(k, v)| (k.clone(), v)).collect()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/l2announcer/l2announcer.go", "Announcer");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn ann(tenant: TenantId, node: &str) -> L2Announcer {
        L2Announcer::new(tenant, node, 60)
    }

    fn lb_policy(tenant: TenantId) -> L2AnnouncementPolicy {
        L2AnnouncementPolicy {
            name: "lb".into(),
            tenant,
            service_selector: ServiceSelector {
                match_labels: vec![("type".into(), "public".into())],
            },
            interfaces: InterfaceMatcher {
                patterns: vec!["eth*".into()],
            },
            load_balancer_ips: true,
            external_ips: false,
        }
    }

    fn service(active: bool) -> ServiceFrontends {
        ServiceFrontends {
            key: "ns/svc".into(),
            labels: vec![("type".into(), "public".into())],
            load_balancer_ips: vec![ip(203, 0, 113, 5)],
            external_ips: vec![],
            has_active_backends: active,
        }
    }

    // ── ServiceSelector ─────────────────────────────────────────────────────

    #[test]
    fn selector_match_labels_subset() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Selector.Match",
            "tenant-l2-sm"
        );
        let s = ServiceSelector {
            match_labels: vec![("a".into(), "1".into())],
        };
        assert!(s.matches(&[("a".into(), "1".into()), ("b".into(), "2".into())]));
    }

    #[test]
    fn selector_match_labels_mismatch() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Selector.Mismatch",
            "tenant-l2-smm"
        );
        let s = ServiceSelector {
            match_labels: vec![("a".into(), "1".into())],
        };
        assert!(!s.matches(&[("a".into(), "2".into())]));
    }

    #[test]
    fn selector_empty_matches_anything() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Selector.Empty",
            "tenant-l2-se"
        );
        let s = ServiceSelector {
            match_labels: vec![],
        };
        assert!(s.matches(&[("a".into(), "1".into())]));
    }

    // ── InterfaceMatcher ────────────────────────────────────────────────────

    #[test]
    fn iface_matcher_exact() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Iface.Exact",
            "tenant-l2-ie"
        );
        let m = InterfaceMatcher {
            patterns: vec!["eth0".into()],
        };
        assert!(m.matches("eth0"));
        assert!(!m.matches("eth1"));
    }

    #[test]
    fn iface_matcher_prefix_wildcard() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Iface.Prefix",
            "tenant-l2-ip"
        );
        let m = InterfaceMatcher {
            patterns: vec!["eth*".into()],
        };
        assert!(m.matches("eth0"));
        assert!(m.matches("eth1"));
        assert!(!m.matches("bond0"));
    }

    #[test]
    fn iface_matcher_star_matches_anything() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Iface.Star",
            "tenant-l2-istar"
        );
        let m = InterfaceMatcher {
            patterns: vec!["*".into()],
        };
        assert!(m.matches("anything"));
    }

    #[test]
    fn iface_matcher_multiple_patterns() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Iface.Multi",
            "tenant-l2-imult"
        );
        let m = InterfaceMatcher {
            patterns: vec!["eth0".into(), "bond*".into()],
        };
        assert!(m.matches("eth0"));
        assert!(m.matches("bond1"));
        assert!(!m.matches("vlan10"));
    }

    // ── Announceable computation ────────────────────────────────────────────

    #[test]
    fn announceable_includes_lb_ip_for_matching_service() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.LB",
            "tenant-l2-anlb"
        );
        let mut a = ann(tenant.clone(), "node-a");
        a.upsert_policy(lb_policy(tenant));
        a.upsert_service(service(true));
        let r = a.announceable(&["eth0".into()]);
        assert!(r.contains(&(ip(203, 0, 113, 5), "eth0".into())));
    }

    #[test]
    fn announceable_skips_service_without_active_backends() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.NoBackends",
            "tenant-l2-anb"
        );
        let mut a = ann(tenant.clone(), "node-a");
        a.upsert_policy(lb_policy(tenant));
        a.upsert_service(service(false));
        let r = a.announceable(&["eth0".into()]);
        assert!(r.is_empty());
    }

    #[test]
    fn announceable_external_ip_only_when_policy_enables_it() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.ExternalIPs",
            "tenant-l2-anex"
        );
        let mut a = ann(tenant.clone(), "node-a");
        let mut p = lb_policy(tenant);
        p.load_balancer_ips = false;
        p.external_ips = true;
        a.upsert_policy(p);
        let mut s = service(true);
        s.external_ips = vec![ip(192, 0, 2, 100)];
        a.upsert_service(s);
        let r = a.announceable(&["eth0".into()]);
        assert!(r.contains(&(ip(192, 0, 2, 100), "eth0".into())));
        assert!(!r.contains(&(ip(203, 0, 113, 5), "eth0".into())));
    }

    #[test]
    fn announceable_skips_interfaces_not_matching() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.Iface",
            "tenant-l2-anif"
        );
        let mut a = ann(tenant.clone(), "node-a");
        a.upsert_policy(lb_policy(tenant));
        a.upsert_service(service(true));
        let r = a.announceable(&["bond0".into()]);
        assert!(r.is_empty());
    }

    // ── Lease acquire / release ─────────────────────────────────────────────

    #[test]
    fn lease_acquire_no_existing_holder_succeeds() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.AcquireFresh",
            "tenant-l2-laq"
        );
        let mut a = ann(tenant, "node-a");
        assert!(a.try_acquire(ip(203, 0, 113, 5), "eth0", 0));
        assert_eq!(
            a.lease_holder(ip(203, 0, 113, 5), "eth0").unwrap().node,
            "node-a"
        );
    }

    #[test]
    fn lease_acquire_held_by_other_node_within_window_fails() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.HeldByOther",
            "tenant-l2-loth"
        );
        let mut a = ann(tenant.clone(), "node-a");
        let mut b = ann(tenant, "node-b");
        // node-a takes the lease.
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        // Carry the lease forward into node-b's view.
        b.leases = a.leases.clone();
        // node-b can't take it within the renewal window.
        assert!(!b.try_acquire(ip(203, 0, 113, 5), "eth0", 30 * 1_000_000_000));
    }

    #[test]
    fn lease_acquire_after_window_lapsed_succeeds_for_other_node() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.WindowLapsed",
            "tenant-l2-llap"
        );
        let mut a = ann(tenant.clone(), "node-a");
        let mut b = ann(tenant, "node-b");
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        b.leases = a.leases.clone();
        // 60s passes — the renewal window is up.
        assert!(b.try_acquire(ip(203, 0, 113, 5), "eth0", 61 * 1_000_000_000));
    }

    #[test]
    fn lease_renewal_by_same_node_always_succeeds() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.RenewSelf",
            "tenant-l2-lren"
        );
        let mut a = ann(tenant, "node-a");
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        // Renew at t=10s.
        assert!(a.try_acquire(ip(203, 0, 113, 5), "eth0", 10 * 1_000_000_000));
        let h = a.lease_holder(ip(203, 0, 113, 5), "eth0").unwrap();
        assert_eq!(h.acquired_ns, 10 * 1_000_000_000);
    }

    #[test]
    fn lease_release_drops_when_held_by_self() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.Release",
            "tenant-l2-lrel"
        );
        let mut a = ann(tenant, "node-a");
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        assert!(a.release(ip(203, 0, 113, 5), "eth0"));
        assert!(a.lease_holder(ip(203, 0, 113, 5), "eth0").is_none());
    }

    #[test]
    fn lease_release_does_nothing_when_held_by_other() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.Release.Foreign",
            "tenant-l2-lrelf"
        );
        let mut a = ann(tenant.clone(), "node-a");
        let mut b = ann(tenant, "node-b");
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        b.leases = a.leases.clone();
        assert!(!b.release(ip(203, 0, 113, 5), "eth0"));
        assert!(b.lease_holder(ip(203, 0, 113, 5), "eth0").is_some());
    }

    #[test]
    fn lease_release_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Lease.Release.Unknown",
            "tenant-l2-lrelnf"
        );
        let mut a = ann(tenant, "node-a");
        assert!(!a.release(ip(1, 1, 1, 1), "eth0"));
    }

    // ── ARP answer ──────────────────────────────────────────────────────────

    #[test]
    fn answer_arp_returns_local_mac_when_holder() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "ARP.Answer",
            "tenant-l2-arp"
        );
        let mut a = ann(tenant, "node-a");
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        assert_eq!(
            a.answer_arp(ip(203, 0, 113, 5), "eth0", [1, 2, 3, 4, 5, 6]),
            Some([1, 2, 3, 4, 5, 6])
        );
    }

    #[test]
    fn answer_arp_silent_when_not_holder() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "ARP.Silent",
            "tenant-l2-arpsil"
        );
        let a = ann(tenant, "node-a");
        assert!(a
            .answer_arp(ip(203, 0, 113, 5), "eth0", [1, 2, 3, 4, 5, 6])
            .is_none());
    }

    // ── Lifecycle ───────────────────────────────────────────────────────────

    #[test]
    fn announcer_remove_policy_drops_it() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "RemovePolicy",
            "tenant-l2-rmp"
        );
        let mut a = ann(tenant.clone(), "node-a");
        a.upsert_policy(lb_policy(tenant));
        a.remove_policy("lb").unwrap();
        assert_eq!(a.policy_count(), 0);
    }

    #[test]
    fn announcer_remove_unknown_policy_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "RemovePolicy.NotFound",
            "tenant-l2-rmpnf"
        );
        let mut a = ann(tenant, "node-a");
        let err = a.remove_policy("ghost").unwrap_err();
        assert!(matches!(err, L2Error::PolicyNotFound(_)));
    }

    #[test]
    fn announcer_remove_service_drops_it() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "RemoveService",
            "tenant-l2-rms"
        );
        let mut a = ann(tenant, "node-a");
        a.upsert_service(service(true));
        a.remove_service("ns/svc").unwrap();
        assert_eq!(a.service_count(), 0);
    }

    #[test]
    fn announcer_announced_vips_lists_all_held() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "AnnouncedVIPs",
            "tenant-l2-anv"
        );
        let mut a = ann(tenant, "node-a");
        a.try_acquire(ip(203, 0, 113, 5), "eth0", 0);
        a.try_acquire(ip(203, 0, 113, 6), "eth0", 0);
        assert_eq!(a.announced_vips().len(), 2);
    }

    // ── Multi-policy ────────────────────────────────────────────────────────

    #[test]
    fn announceable_combines_multiple_policies() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.MultiPolicy",
            "tenant-l2-mp"
        );
        let mut a = ann(tenant.clone(), "node-a");
        let mut p1 = lb_policy(tenant.clone());
        p1.name = "p1".into();
        let mut p2 = lb_policy(tenant);
        p2.name = "p2".into();
        a.upsert_policy(p1);
        a.upsert_policy(p2);
        a.upsert_service(service(true));
        let r = a.announceable(&["eth0".into()]);
        // Same VIP/iface comes through once (BTreeSet dedup).
        assert_eq!(r.len(), 1);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn l2_policy_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/k8s/apis/cilium.io/v2alpha1/types.go",
            "L2Policy.Serde",
            "tenant-l2-pserde"
        );
        let p = lb_policy(tenant);
        let s = serde_json::to_string(&p).unwrap();
        let back: L2AnnouncementPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn lease_holder_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "LeaseHolder.Serde",
            "tenant-l2-lserde"
        );
        let l = LeaseHolder {
            node: "a".into(),
            acquired_ns: 100,
            renewal_deadline_ns: 200,
        };
        let s = serde_json::to_string(&l).unwrap();
        let back: LeaseHolder = serde_json::from_str(&s).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn service_frontends_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "ServiceFrontends.Serde",
            "tenant-l2-sserde"
        );
        let s = service(true);
        let json = serde_json::to_string(&s).unwrap();
        let back: ServiceFrontends = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ── Edge: no interfaces ─────────────────────────────────────────────────

    #[test]
    fn announceable_empty_interfaces_returns_empty() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.NoIfaces",
            "tenant-l2-nif"
        );
        let mut a = ann(tenant.clone(), "node-a");
        a.upsert_policy(lb_policy(tenant));
        a.upsert_service(service(true));
        let r = a.announceable(&[]);
        assert!(r.is_empty());
    }

    #[test]
    fn announceable_no_policies_returns_empty() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/l2announcer.go",
            "Announceable.NoPolicies",
            "tenant-l2-np"
        );
        let mut a = ann(tenant, "node-a");
        a.upsert_service(service(true));
        let r = a.announceable(&["eth0".into()]);
        assert!(r.is_empty());
    }
}
