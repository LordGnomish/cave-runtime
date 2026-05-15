// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! nftables proxier — preferred datapath on Linux ≥ 7.1.
//!
//! Cite: `pkg/proxy/nftables/proxier.go:71` (servicesChain),
//! `:72` (serviceIPsMap), `:73` (serviceNodePortsMap),
//! `:138` (Proxier), `:282` (servicePortInfo), `:381–:382` (jumps from
//! NAT prerouting/output into `services`), `:410` (setupNFTables).
//!
//! cave drops the userspace mode entirely: this is a greenfield
//! deployment and the legacy `pkg/proxy/userspace` proxier is not a
//! migration target.

use crate::endpoints::EndpointSliceMap;
use crate::error::{KubeProxyError, KubeProxyResult};
use crate::service::{ServicePortInfo, SessionAffinity};

pub const TABLE_NAME: &str = "kube-proxy";
pub const SERVICES_CHAIN: &str = "services";
pub const NODEPORTS_CHAIN: &str = "nodeports";
pub const SERVICE_IPS_MAP: &str = "service-ips";
pub const SERVICE_NODEPORTS_MAP: &str = "service-nodeports";

#[derive(Debug, Clone)]
pub struct NftablesProxier {
    pub tenant_id: String,
}

impl NftablesProxier {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into() }
    }

    /// Cite: `pkg/proxy/nftables/proxier.go:410` (setupNFTables) — the
    /// scaffold transaction that primes the table, base chains, jump
    /// chains and the two service maps. cave returns the textual
    /// `nft -f -` payload (real backend will hand off to knftables).
    pub fn build_table_scaffold(&self) -> Vec<String> {
        vec![
            format!("table inet {TABLE_NAME} {{"),
            format!("    chain {SERVICES_CHAIN} {{}}"),
            format!("    chain {NODEPORTS_CHAIN} {{}}"),
            format!("    map {SERVICE_IPS_MAP} {{ type ipv4_addr . inet_proto . inet_service : verdict; }}"),
            format!("    map {SERVICE_NODEPORTS_MAP} {{ type inet_proto . inet_service : verdict; }}"),
            "}".to_string(),
        ]
    }

    /// Cite: `pkg/proxy/nftables/proxier.go:637` (services chain VMAP
    /// dispatch) — every Service contributes one row in the
    /// `service-ips` map: `(clusterIP, proto, port) → goto svc-XXXX`.
    pub fn build_service_ips_map_entries(
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
            out.push(format!(
                "{ip} . {proto} . {port} : goto {chain}",
                ip = svc.cluster_ip,
                proto = svc.protocol.as_str(),
                port = svc.port,
                chain = svc_chain_name(svc),
            ));
        }
        Ok(out)
    }

    /// Cite: `pkg/proxy/nftables/proxier.go` `service-nodeports` map
    /// (rendered alongside `service-ips`). Format mirrors knftables Map
    /// element semantics: `(proto, port) → goto svc-XXXX`.
    pub fn build_service_nodeports_map_entries(
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
            out.push(format!(
                "{proto} . {port} : goto {chain}",
                proto = svc.protocol.as_str(),
                port = np,
                chain = svc_chain_name(svc),
            ));
        }
        Ok(out)
    }

    /// Cite: `pkg/proxy/nftables/proxier.go:381–:382` — jumps installed
    /// from `nat prerouting` and `nat output` into the `services` chain.
    pub fn build_jump_rules(&self) -> Vec<String> {
        vec![
            format!("add rule inet {TABLE_NAME} prerouting jump {SERVICES_CHAIN}"),
            format!("add rule inet {TABLE_NAME} output     jump {SERVICES_CHAIN}"),
        ]
    }

    /// Cite: `pkg/proxy/nftables/proxier.go` per-service chain — mirrors
    /// the iptables proxier but expresses random LB via nftables
    /// `numgen random` and session affinity via `meta mark` + a `set`
    /// element scoped to the source IP.
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
        let eps = endpoints.ready_endpoints_for(&svc.name);
        let n = eps.len();
        let mut out = Vec::new();

        out.push(format!("add chain inet {TABLE_NAME} {chain}"));

        if matches!(svc.session_affinity, SessionAffinity::ClientIp) {
            let ttl = svc.sticky_max_age_seconds.unwrap_or(10_800);
            out.push(format!(
                "add rule inet {TABLE_NAME} {chain} ip saddr @{chain}-affinity timeout {ttl}s goto {chain}-pinned"
            ));
        }

        if n == 0 {
            // No endpoints — drop or REJECT. cave matches upstream "REJECT".
            out.push(format!("add rule inet {TABLE_NAME} {chain} reject with icmp type host-unreachable"));
            return Ok(out);
        }

        for (i, ep) in eps.iter().enumerate() {
            let addr = ep.addresses.first().expect("endpoint has at least one address");
            if i + 1 == n {
                out.push(format!(
                    "add rule inet {TABLE_NAME} {chain} dnat to {addr}:{port}",
                    addr = addr, port = ep.port,
                ));
            } else {
                out.push(format!(
                    "add rule inet {TABLE_NAME} {chain} numgen random mod {n} == {i} dnat to {addr}:{port}",
                    n = n, i = i, addr = addr, port = ep.port,
                ));
            }
        }
        Ok(out)
    }
}

fn svc_chain_name(svc: &ServicePortInfo) -> String {
    format!("svc-{}", short_hash(&svc.name.key()))
}

fn short_hash(s: &str) -> String {
    let mut h: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{:016x}", h)
}
