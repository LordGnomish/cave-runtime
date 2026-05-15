// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! iptables proxier — KUBE-SERVICES + KUBE-NODEPORTS chain emission.
//!
//! Cite: `pkg/proxy/iptables/proxier.go:55` (kubeServicesChain),
//! `:61` (kubeNodePortsChain), `:638` (syncProxyRules), `:939–:1066`
//! (per-service rule emission), `:1301–:1326` (KUBE-NODEPORTS jump
//! from KUBE-SERVICES, "must be the last rule").

use crate::endpoints::EndpointSliceMap;
use crate::error::{KubeProxyError, KubeProxyResult};
use crate::service::{ServicePortInfo, SessionAffinity};

pub const KUBE_SERVICES_CHAIN: &str = "KUBE-SERVICES";
pub const KUBE_NODEPORTS_CHAIN: &str = "KUBE-NODEPORTS";
pub const KUBE_MARK_MASQ_CHAIN: &str = "KUBE-MARK-MASQ";

/// Tenant-scoped iptables ruleset emitter. Stateless w.r.t. iptables
/// itself — render a complete ruleset, hand off to a syncer that pipes
/// it into `iptables-restore` (out of scope for this batch).
#[derive(Debug, Clone)]
pub struct IptablesProxier {
    pub tenant_id: String,
}

impl IptablesProxier {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into() }
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:939–:989` — per-service
    /// KUBE-SERVICES rules: ClusterIP `-d <vip> -p <proto> --dport <port>
    /// -j KUBE-SVC-XXXX`. cave emits the same rule shape with synthesised
    /// `KUBE-SVC-<hash>` chain names derived from the ServicePortName.
    pub fn build_kube_services_rules(
        &self,
        services: &[ServicePortInfo],
    ) -> KubeProxyResult<Vec<String>> {
        let mut out = Vec::new();
        for svc in services {
            if svc.tenant_id != self.tenant_id {
                return Err(KubeProxyError::CrossTenantDenied {
                    store: self.tenant_id.clone(),
                    req: svc.tenant_id.clone(),
                });
            }
            if svc.should_skip() {
                continue;
            }
            let svc_chain = svc_chain_name(svc);
            out.push(format!(
                "-A {chain} -d {ip} -p {proto} --dport {port} -j {svc_chain}",
                chain = KUBE_SERVICES_CHAIN,
                ip = svc.cluster_ip,
                proto = svc.protocol.as_str(),
                port = svc.port,
                svc_chain = svc_chain,
            ));
        }
        Ok(out)
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:1031–:1066` — per-service
    /// NodePort rules under KUBE-NODEPORTS: `-p <proto> --dport <nodePort>
    /// -j KUBE-EXT-XXXX` (or KUBE-SVC for cluster policy).
    pub fn build_kube_nodeports_rules(
        &self,
        services: &[ServicePortInfo],
    ) -> KubeProxyResult<Vec<String>> {
        let mut out = Vec::new();
        for svc in services {
            if svc.tenant_id != self.tenant_id {
                return Err(KubeProxyError::CrossTenantDenied {
                    store: self.tenant_id.clone(),
                    req: svc.tenant_id.clone(),
                });
            }
            let Some(np) = svc.node_port else { continue };
            let target_chain = svc_chain_name(svc);
            out.push(format!(
                "-A {chain} -p {proto} --dport {port} -j {target}",
                chain = KUBE_NODEPORTS_CHAIN,
                proto = svc.protocol.as_str(),
                port = np,
                target = target_chain,
            ));
        }
        Ok(out)
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:1301–:1326` — the trailing
    /// `KUBE-SERVICES → KUBE-NODEPORTS` jump must be appended LAST so
    /// ClusterIP rules win when both could match.
    pub fn build_kube_services_nodeports_terminator(&self) -> String {
        format!(
            "-A {svc} -m addrtype --dst-type LOCAL -j {np}",
            svc = KUBE_SERVICES_CHAIN,
            np = KUBE_NODEPORTS_CHAIN,
        )
    }

    /// Cite: `pkg/proxy/iptables/proxier.go` SVC → SEP chain hop. With
    /// session affinity ClientIP we emit a `recent` match entry that
    /// pins the client to the prior endpoint for `sticky_max_age_seconds`.
    pub fn build_svc_chain_rules(
        &self,
        svc: &ServicePortInfo,
        endpoints: &EndpointSliceMap,
    ) -> KubeProxyResult<Vec<String>> {
        if svc.tenant_id != self.tenant_id {
            return Err(KubeProxyError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: svc.tenant_id.clone(),
            });
        }
        endpoints.check_tenant(&self.tenant_id)?;
        let chain = svc_chain_name(svc);
        let mut out = Vec::new();

        if matches!(svc.session_affinity, SessionAffinity::ClientIp) {
            let ttl = svc.sticky_max_age_seconds.unwrap_or(10_800);
            // matches recent client → reuse prior SEP
            out.push(format!(
                "-A {chain} -m recent --name {chain} --rcheck --seconds {ttl} -j {chain}-AFFINITY",
                chain = chain,
                ttl = ttl,
            ));
        }

        let eps = endpoints.ready_endpoints_for(&svc.name);
        let n = eps.len();
        for (i, ep) in eps.iter().enumerate() {
            let prob = if i + 1 == n {
                1.0
            } else {
                1.0 / (n - i) as f64
            };
            for addr in &ep.addresses {
                out.push(format!(
                    "-A {chain} -m statistic --mode random --probability {prob:.10} \
                     -j DNAT --to-destination {addr}:{port}",
                    chain = chain,
                    prob = prob,
                    addr = addr,
                    port = ep.port,
                ));
            }
        }
        Ok(out)
    }

    /// Cite: `pkg/proxy/iptables/proxier.go:1066` LB source-range guard
    /// — when source ranges are present, traffic from outside any range
    /// is dropped via `KUBE-FW-XXXX -j KUBE-MARK-DROP`.
    pub fn build_loadbalancer_firewall_rules(
        &self,
        svc: &ServicePortInfo,
    ) -> KubeProxyResult<Vec<String>> {
        if svc.tenant_id != self.tenant_id {
            return Err(KubeProxyError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: svc.tenant_id.clone(),
            });
        }
        if svc.load_balancer_source_ranges.is_empty() {
            return Ok(Vec::new());
        }
        let fw = format!("KUBE-FW-{}", short_hash(&svc.name.key()));
        let mut rules = Vec::new();
        for cidr in &svc.load_balancer_source_ranges {
            rules.push(format!(
                "-A {fw} -s {cidr} -j {svc_chain}",
                fw = fw,
                cidr = cidr.to_string_canonical(),
                svc_chain = svc_chain_name(svc),
            ));
        }
        rules.push(format!("-A {fw} -j KUBE-MARK-DROP", fw = fw));
        Ok(rules)
    }
}

fn svc_chain_name(svc: &ServicePortInfo) -> String {
    format!("KUBE-SVC-{}", short_hash(&svc.name.key()))
}

/// 16-character uppercase hex of a stable hash. Mirrors openbao-style
/// salting at the cave layer (upstream uses base32(SHA256) truncated).
fn short_hash(s: &str) -> String {
    let mut h: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{:016X}", h)
}
