// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ProxyConfig — runtime knobs consumed by the sync loop.
//!
//! Cite: `cmd/kube-proxy/app/server.go:185` (Options),
//! `pkg/proxy/apis/config/types.go:46` (KubeProxyConfiguration),
//! `pkg/proxy/apis/config/types.go:222` (DetectLocalConfiguration).
//!
//! cave drops `BindAddress` / `OOMScoreAdj` / `Conntrack` knobs that are
//! daemon-process concerns (handled at the systemd unit / runtime supervisor
//! layer in cave). The fields kept here are the ones the proxier loop needs
//! to consume per-sync.

use crate::error::{KubeProxyError, KubeProxyResult};
use crate::service::{Cidr, IpCidr};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Cite: `pkg/proxy/apis/config/types.go:55` (ProxyMode) — the family
/// the sync loop should target. The userspace mode is intentionally
/// absent: cave is greenfield, no legacy upgrade path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    Iptables,
    Nftables,
    /// IPVS support is gated behind the `cave-kube-proxy/ipvs` feature
    /// in cave-net (eBPF IPVS-compat). At this crate's layer it is a
    /// marker; the syncer does not emit IPVS rules directly.
    Ipvs,
}

impl ProxyMode {
    /// Cite: `cmd/kube-proxy/app/server.go:585` (detectAndReturnProxyMode)
    /// — kernel-version + CNI-feature detection. cave delegates the real
    /// detection to the runtime preflight check (this crate stays
    /// fileless); the helper exposes the default we'd select today.
    pub fn default_for_kernel() -> Self {
        // Linux ≥ 7.1 ships nftables-only; cave's default kernel pin is
        // newer than that, so nftables is the production default.
        Self::Nftables
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Iptables => "iptables",
            Self::Nftables => "nftables",
            Self::Ipvs => "ipvs",
        }
    }
}

/// Cite: `pkg/proxy/apis/config/types.go:222` (DetectLocalConfiguration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DetectLocal {
    /// Match endpoints whose `nodeName` equals the local node.
    NodeName,
    /// Match endpoints whose pod IP is in the cluster CIDR for this node.
    ClusterCidr,
    /// Match a configurable interface name prefix.
    InterfaceNamePrefix,
    /// No local-endpoint detection — every endpoint is "remote".
    Disabled,
}

/// Cite: `pkg/proxy/apis/config/types.go:46` (KubeProxyConfiguration) —
/// the subset the sync loop actually consumes during a tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub cluster_cidr: Option<Cidr>,
    /// Cite: `pkg/proxy/apis/config/types.go:107` (ClusterCIDR — IPv6 family).
    /// A live, parsed CIDR (was previously a dead `Option<String>` hook);
    /// consumed by [`ProxyConfig::detect_local_by_cidr`] for v6 endpoints.
    pub cluster_cidr_v6: Option<IpCidr>,
    pub detect_local: DetectLocal,
    /// Cite: `pkg/proxy/apis/config/types.go:103` (NodePortAddresses) —
    /// when set, NodePort/LoadBalancer rules only bind to interfaces
    /// matching the listed CIDRs.
    pub node_port_addresses: Vec<Cidr>,
    /// Cite: `cmd/kube-proxy/app/options/options.go` (sync_period default
    /// 30s) — the maximum interval between full proxier syncs.
    pub sync_period_secs: u32,
    /// Cite: `cmd/kube-proxy/app/options/options.go` (min_sync_period
    /// default 1s) — the minimum interval between consecutive syncs to
    /// debounce event storms.
    pub min_sync_period_secs: u32,
    /// Cite: `pkg/proxy/apis/config/types.go:147`
    /// (ClusterDNS / HealthzBindAddress) — the bind for the proxier's
    /// own healthz endpoint (default 0.0.0.0:10256).
    pub healthz_bind_port: u16,
    /// Cite: `pkg/proxy/apis/config/types.go:160` (MetricsBindAddress)
    /// — Prometheus exposition port (default 10249).
    pub metrics_bind_port: u16,
    /// Cite: `pkg/proxy/apis/config/types.go` (Conntrack.MaxPerCore) —
    /// kernel conntrack hashsize is scaled per logical core.
    pub conntrack_max_per_core: u32,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            mode: ProxyMode::default_for_kernel(),
            cluster_cidr: None,
            cluster_cidr_v6: None,
            detect_local: DetectLocal::NodeName,
            node_port_addresses: Vec::new(),
            sync_period_secs: 30,
            min_sync_period_secs: 1,
            healthz_bind_port: 10_256,
            metrics_bind_port: 10_249,
            conntrack_max_per_core: 32_768,
        }
    }
}

impl ProxyConfig {
    /// Cite: `cmd/kube-proxy/app/server.go:412` (validateProxyMode) — the
    /// runtime rejects min_sync_period > sync_period and zero healthz/metrics
    /// ports when the corresponding feature is enabled.
    pub fn validate(&self) -> KubeProxyResult<()> {
        if self.sync_period_secs == 0 {
            return Err(KubeProxyError::InvalidConfig(
                "sync_period_secs == 0".to_string(),
            ));
        }
        if self.min_sync_period_secs > self.sync_period_secs {
            return Err(KubeProxyError::InvalidConfig(format!(
                "min_sync_period_secs ({}) > sync_period_secs ({})",
                self.min_sync_period_secs, self.sync_period_secs
            )));
        }
        Ok(())
    }

    /// Cite: `pkg/proxy/util/nodeport.go:43` (CidrContainsIp) — when
    /// node_port_addresses is set, only IPs inside any listed CIDR are
    /// valid bind addresses for NodePort rules.
    pub fn node_port_bind_allowed(&self, addr: std::net::Ipv4Addr) -> bool {
        if self.node_port_addresses.is_empty() {
            return true;
        }
        self.node_port_addresses.iter().any(|c| c.contains(addr))
    }

    /// Populate the v4 + v6 ClusterCIDR fields from the upstream
    /// comma-separated dual-stack `ClusterCIDR` string (e.g.
    /// `"10.244.0.0/16,fd00:10:244::/56"`). The last CIDR of each family
    /// wins, mirroring `GetClusterIPByFamily` which keys purely on family.
    ///
    /// Cite: `pkg/proxy/apis/config/types.go:107` (ClusterCIDR),
    /// `k8s.io/utils/net.ParseCIDRs`.
    pub fn with_cluster_cidrs(mut self, spec: &str) -> KubeProxyResult<Self> {
        for cidr in IpCidr::parse_list(spec)? {
            if cidr.is_ipv6() {
                self.cluster_cidr_v6 = Some(cidr);
            } else {
                // Preserve the IPv4 `Cidr` representation other call sites
                // already consume (e.g. NodePort bind checks).
                self.cluster_cidr = Some(Cidr::parse(&cidr.to_string_canonical())?);
            }
        }
        Ok(self)
    }

    /// Return the configured ClusterCIDR for the requested IP family, or
    /// `None` if this proxier has no CIDR for that family.
    ///
    /// Cite: `pkg/proxy/util/utils.go` `GetClusterIPByFamily`.
    pub fn cluster_cidr_for_family(&self, want_v6: bool) -> Option<IpCidr> {
        if want_v6 {
            self.cluster_cidr_v6
        } else {
            self.cluster_cidr
                .map(|c| IpCidr::parse(&c.to_string_canonical()).expect("v4 Cidr is valid IpCidr"))
        }
    }

    /// Upstream `DetectLocalByCIDR`: an endpoint IP is "local" when it falls
    /// inside this node's ClusterCIDR for the endpoint's own IP family. A v4
    /// endpoint is never matched against the v6 CIDR and vice-versa.
    ///
    /// Cite: `pkg/proxy/topology.go` `DetectLocalByCIDR`.
    pub fn detect_local_by_cidr(&self, ip: IpAddr) -> bool {
        match self.cluster_cidr_for_family(ip.is_ipv6()) {
            Some(cidr) => cidr.contains(ip),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn default_mode_is_nftables() {
        assert_eq!(ProxyMode::default_for_kernel(), ProxyMode::Nftables);
    }

    #[test]
    fn validate_rejects_zero_sync_period() {
        let mut c = ProxyConfig::default();
        c.sync_period_secs = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_min_above_sync() {
        let mut c = ProxyConfig::default();
        c.min_sync_period_secs = 90;
        c.sync_period_secs = 30;
        assert!(c.validate().is_err());
    }

    #[test]
    fn default_validates() {
        assert!(ProxyConfig::default().validate().is_ok());
    }

    #[test]
    fn bind_allowed_empty_means_anywhere() {
        let c = ProxyConfig::default();
        assert!(c.node_port_bind_allowed(Ipv4Addr::new(10, 0, 0, 5)));
    }

    #[test]
    fn bind_allowed_inside_listed_cidr() {
        let mut c = ProxyConfig::default();
        c.node_port_addresses
            .push(Cidr::parse("10.0.0.0/24").unwrap());
        assert!(c.node_port_bind_allowed(Ipv4Addr::new(10, 0, 0, 5)));
        assert!(!c.node_port_bind_allowed(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn mode_as_str_roundtrip() {
        assert_eq!(ProxyMode::Iptables.as_str(), "iptables");
        assert_eq!(ProxyMode::Nftables.as_str(), "nftables");
        assert_eq!(ProxyMode::Ipvs.as_str(), "ipvs");
    }
}
