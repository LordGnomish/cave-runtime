// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CNI plugin chain — Cilium as a chained plugin alongside portmap,
//! bandwidth, sbr, etc.
//!
//! Mirrors `plugins/cilium-cni/chaining/chaining.go` and the CNI spec
//! `containernetworking/cni::types.NetConf::PrevResult`. When Cilium
//! runs as a chained plugin, it consumes the previous plugin's result,
//! attaches its BPF programs, and emits the augmented result.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniNetConf {
    pub cni_version: String,
    pub name: String,
    pub plugin: String,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniInterface {
    pub name: String,
    pub mac: String,
    pub sandbox: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniIpConfig {
    pub address: String, // CIDR form e.g. "10.244.1.5/24"
    pub gateway: Option<IpAddr>,
    pub interface: Option<u32>, // index into prev_result.interfaces
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniRoute {
    pub dst: String,
    pub gw: Option<IpAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniResult {
    pub cni_version: String,
    pub interfaces: Vec<CniInterface>,
    pub ips: Vec<CniIpConfig>,
    pub routes: Vec<CniRoute>,
    pub dns_nameservers: Vec<IpAddr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChainError {
    #[error("chain has no Cilium config")]
    MissingCilium,
    #[error("previous result missing interface index {0}")]
    BadInterfaceRef(u32),
    #[error("previous result has no IP for the container")]
    NoContainerIp,
    #[error("plugin `{0}` already in chain")]
    DuplicatePlugin(String),
    #[error("tenant {tenant} cannot mutate chain owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct CniChain {
    pub tenant: TenantId,
    pub plugins: Vec<CniNetConf>,
}

impl CniChain {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            plugins: Vec::new(),
        }
    }

    pub fn append(&mut self, plugin: CniNetConf) -> Result<(), ChainError> {
        if self.plugins.iter().any(|p| p.plugin == plugin.plugin) {
            return Err(ChainError::DuplicatePlugin(plugin.plugin));
        }
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn position_of(&self, plugin: &str) -> Option<usize> {
        self.plugins.iter().position(|p| p.plugin == plugin)
    }

    pub fn has_cilium(&self) -> bool {
        self.plugins.iter().any(|p| p.plugin == "cilium-cni")
    }

    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Augment the previous plugin's result. Mirrors
    /// `plugins/cilium-cni/chaining/chaining.go::Add`.
    pub fn run_cilium_chain(&self, prev: CniResult) -> Result<CniResult, ChainError> {
        if !self.has_cilium() {
            return Err(ChainError::MissingCilium);
        }
        if prev.ips.is_empty() {
            return Err(ChainError::NoContainerIp);
        }
        for ip in &prev.ips {
            if let Some(idx) = ip.interface {
                if (idx as usize) >= prev.interfaces.len() {
                    return Err(ChainError::BadInterfaceRef(idx));
                }
            }
        }
        // The Cilium chaining plugin doesn't add new interfaces or
        // addresses; it attaches BPF programs in-place. The result is
        // therefore the same as `prev` plus a pseudo-route the agent
        // adds to direct egress to cilium_host (we omit modelling the
        // actual route but record the unchanged shape).
        Ok(prev)
    }

    /// Find the per-pod bandwidth limit from a chained `bandwidth` plugin.
    pub fn bandwidth_limit(&self) -> Option<&str> {
        let p = self.plugins.iter().find(|p| p.plugin == "bandwidth")?;
        p.config.get("egressRate").map(|s| s.as_str())
    }

    /// Find any chained `portmap` plugin's mappings (just the count for
    /// our purposes).
    pub fn portmap_count(&self) -> usize {
        self.plugins
            .iter()
            .filter(|p| p.plugin == "portmap")
            .map(|p| {
                p.config
                    .get("ports")
                    .map(|s| s.split(',').count())
                    .unwrap_or(0)
            })
            .sum()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("plugins/cilium-cni/chaining/chaining.go", "Add");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn cilium_conf() -> CniNetConf {
        CniNetConf {
            cni_version: "1.0.0".into(),
            name: "cluster".into(),
            plugin: "cilium-cni".into(),
            config: HashMap::new(),
        }
    }

    fn portmap_conf(ports: &str) -> CniNetConf {
        let mut c = HashMap::new();
        c.insert("ports".into(), ports.into());
        CniNetConf {
            cni_version: "1.0.0".into(),
            name: "cluster".into(),
            plugin: "portmap".into(),
            config: c,
        }
    }

    fn bandwidth_conf(rate: &str) -> CniNetConf {
        let mut c = HashMap::new();
        c.insert("egressRate".into(), rate.into());
        CniNetConf {
            cni_version: "1.0.0".into(),
            name: "cluster".into(),
            plugin: "bandwidth".into(),
            config: c,
        }
    }

    fn prev_result(addr: &str) -> CniResult {
        CniResult {
            cni_version: "1.0.0".into(),
            interfaces: vec![CniInterface {
                name: "eth0".into(),
                mac: "0a:00:00:00:00:01".into(),
                sandbox: Some("/var/run/netns/abc".into()),
            }],
            ips: vec![CniIpConfig {
                address: addr.into(),
                gateway: Some(ip(10, 244, 1, 1)),
                interface: Some(0),
            }],
            routes: vec![CniRoute {
                dst: "0.0.0.0/0".into(),
                gw: Some(ip(10, 244, 1, 1)),
            }],
            dns_nameservers: vec![ip(10, 96, 0, 10)],
        }
    }

    fn chain(tenant: TenantId) -> CniChain {
        CniChain::new(tenant)
    }

    // ── Append / introspection ─────────────────────────────────────────────

    #[test]
    fn chain_append_records_plugin() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Append",
            "tenant-cn-a"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        assert!(c.has_cilium());
    }

    #[test]
    fn chain_append_duplicate_plugin_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Append.Duplicate",
            "tenant-cn-ad"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        let err = c.append(cilium_conf()).unwrap_err();
        assert!(matches!(err, ChainError::DuplicatePlugin(_)));
    }

    #[test]
    fn chain_position_of_locates_plugin() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Position",
            "tenant-cn-p"
        );
        let mut c = chain(tenant);
        c.append(portmap_conf("80,443")).unwrap();
        c.append(cilium_conf()).unwrap();
        assert_eq!(c.position_of("cilium-cni"), Some(1));
    }

    #[test]
    fn chain_position_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Position.NotFound",
            "tenant-cn-pn"
        );
        let c = chain(tenant);
        assert!(c.position_of("ghost").is_none());
    }

    #[test]
    fn chain_len_tracks_appends() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Len",
            "tenant-cn-l"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        c.append(portmap_conf("80,443")).unwrap();
        c.append(bandwidth_conf("10M")).unwrap();
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn chain_is_empty_initially() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "IsEmpty",
            "tenant-cn-emp"
        );
        let c = chain(tenant);
        assert!(c.is_empty());
    }

    // ── run_cilium_chain ───────────────────────────────────────────────────

    #[test]
    fn run_chain_returns_prev_result_unchanged() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Run",
            "tenant-cn-r"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        let prev = prev_result("10.244.1.5/24");
        let out = c.run_cilium_chain(prev.clone()).unwrap();
        assert_eq!(out, prev);
    }

    #[test]
    fn run_chain_without_cilium_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Run.MissingCilium",
            "tenant-cn-rm"
        );
        let mut c = chain(tenant);
        c.append(portmap_conf("80")).unwrap();
        let err = c
            .run_cilium_chain(prev_result("10.244.1.5/24"))
            .unwrap_err();
        assert!(matches!(err, ChainError::MissingCilium));
    }

    #[test]
    fn run_chain_without_container_ip_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Run.NoIP",
            "tenant-cn-rn"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        let mut prev = prev_result("10.244.1.5/24");
        prev.ips.clear();
        let err = c.run_cilium_chain(prev).unwrap_err();
        assert!(matches!(err, ChainError::NoContainerIp));
    }

    #[test]
    fn run_chain_with_bad_interface_ref_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Run.BadInterfaceRef",
            "tenant-cn-rb"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        let mut prev = prev_result("10.244.1.5/24");
        prev.ips[0].interface = Some(99);
        let err = c.run_cilium_chain(prev).unwrap_err();
        assert!(matches!(err, ChainError::BadInterfaceRef(99)));
    }

    // ── bandwidth_limit / portmap_count ────────────────────────────────────

    #[test]
    fn bandwidth_limit_extracted_from_chained_plugin() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "BandwidthLimit",
            "tenant-cn-bl"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        c.append(bandwidth_conf("100M")).unwrap();
        assert_eq!(c.bandwidth_limit(), Some("100M"));
    }

    #[test]
    fn bandwidth_limit_none_when_no_bandwidth_plugin() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "BandwidthLimit.None",
            "tenant-cn-bln"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        assert!(c.bandwidth_limit().is_none());
    }

    #[test]
    fn portmap_count_sums_chained_portmaps() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "PortmapCount",
            "tenant-cn-pc"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        c.append(portmap_conf("80,443,8080")).unwrap();
        assert_eq!(c.portmap_count(), 3);
    }

    #[test]
    fn portmap_count_zero_when_no_portmap() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "PortmapCount.Zero",
            "tenant-cn-pcz"
        );
        let mut c = chain(tenant);
        c.append(cilium_conf()).unwrap();
        assert_eq!(c.portmap_count(), 0);
    }

    // ── Plugin enumeration ─────────────────────────────────────────────────

    #[test]
    fn ordering_preserved_in_chain() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Order",
            "tenant-cn-o"
        );
        let mut c = chain(tenant);
        c.append(portmap_conf("80")).unwrap();
        c.append(cilium_conf()).unwrap();
        c.append(bandwidth_conf("10M")).unwrap();
        assert_eq!(c.plugins[0].plugin, "portmap");
        assert_eq!(c.plugins[1].plugin, "cilium-cni");
        assert_eq!(c.plugins[2].plugin, "bandwidth");
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn cni_netconf_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "NetConf.Serde",
            "tenant-cn-nserde"
        );
        let p = portmap_conf("80,443");
        let s = serde_json::to_string(&p).unwrap();
        let back: CniNetConf = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn cni_result_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Result.Serde",
            "tenant-cn-rserde"
        );
        let r = prev_result("10.244.1.5/24");
        let s = serde_json::to_string(&r).unwrap();
        let back: CniResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn cni_route_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "Route.Serde",
            "tenant-cn-roserde"
        );
        let r = CniRoute {
            dst: "0.0.0.0/0".into(),
            gw: Some(ip(10, 0, 0, 1)),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: CniRoute = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    // ── Multi-plugin scenario ──────────────────────────────────────────────

    #[test]
    fn full_chain_runs_with_three_plugins() {
        let (_c, tenant) = cilium_test_ctx!(
            "plugins/cilium-cni/chaining/chaining.go",
            "FullChain",
            "tenant-cn-fc"
        );
        let mut c = chain(tenant);
        c.append(portmap_conf("80,443")).unwrap();
        c.append(cilium_conf()).unwrap();
        c.append(bandwidth_conf("100M")).unwrap();
        let prev = prev_result("10.244.1.5/24");
        c.run_cilium_chain(prev).unwrap();
        assert_eq!(c.bandwidth_limit(), Some("100M"));
        assert_eq!(c.portmap_count(), 2);
    }
}
