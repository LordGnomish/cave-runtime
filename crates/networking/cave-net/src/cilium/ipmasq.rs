// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! IP masquerade agent (config + map shape).
//!
//! Mirrors `pkg/ipmasq/ipmasq.go`. Cilium ships an ip-masq-agent-compatible
//! controller that reads a YAML/JSON config (`nonMasqueradeCIDRs` plus link-
//! local toggles) and pins a BPF map of "do-not-masquerade" prefixes.
//!
//! We port:
//!   * the default non-masquerade CIDR table (RFC 1918 + RFC 6598 + IANA
//!     reserved ranges) — same exact list as upstream
//!   * the [`Config`] JSON shape with the exact upstream JSON field names
//!   * an in-memory map model with Update/Delete/Dump

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::str::FromStr;

/// Link-local IPv4 CIDR (RFC 3927).
pub const LINK_LOCAL_CIDR_IPV4: &str = "169.254.0.0/16";
/// Link-local IPv6 CIDR (RFC 4291).
pub const LINK_LOCAL_CIDR_IPV6: &str = "fe80::/10";

/// Default non-masquerade CIDRs — matches the slice baked in
/// `pkg/ipmasq/ipmasq.go` (RFC 1918, RFC 6598, RFC 5735/5737/3068).
pub fn default_non_masq_cidrs() -> Vec<&'static str> {
    vec![
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "100.64.0.0/10",
        "192.0.0.0/24",
        "192.0.2.0/24",
        "192.88.99.0/24",
        "198.18.0.0/15",
        "198.51.100.0/24",
        "203.0.113.0/24",
        "240.0.0.0/4",
    ]
}

/// Config file shape. The JSON field names are exactly the upstream tag
/// values (`nonMasqueradeCIDRs`, `masqLinkLocal`, `masqLinkLocalIPv6`)
/// so an upstream-formatted config file deserialises directly.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "nonMasqueradeCIDRs", default)]
    pub non_masq_cidrs: Vec<String>,
    #[serde(rename = "masqLinkLocal", default)]
    pub masq_link_local_ipv4: bool,
    #[serde(rename = "masqLinkLocalIPv6", default)]
    pub masq_link_local_ipv6: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IpMasqError {
    #[error("invalid CIDR {0}")]
    InvalidCidr(String),
    #[error("tenant {tenant} cannot mutate ipmasq map owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// In-memory model of the BPF ipmasq map (the kernel-side table the
/// agent pins to BPFFS). We expose Update/Delete/Dump as the upstream
/// `IPMasqMap` interface does.
#[derive(Debug, Default)]
pub struct IpMasqMap {
    entries: BTreeSet<String>, // canonical "ip/prefix" strings
}

impl IpMasqMap {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn update(&mut self, cidr: &str) -> Result<(), IpMasqError> {
        let p = IpNet::from_str(cidr).map_err(|_| IpMasqError::InvalidCidr(cidr.to_string()))?;
        self.entries.insert(p.to_string());
        Ok(())
    }
    pub fn delete(&mut self, cidr: &str) -> Result<bool, IpMasqError> {
        let p = IpNet::from_str(cidr).map_err(|_| IpMasqError::InvalidCidr(cidr.to_string()))?;
        Ok(self.entries.remove(&p.to_string()))
    }
    pub fn dump(&self) -> Vec<String> {
        self.entries.iter().cloned().collect()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ipmasq/ipmasq.go", "IPMasqAgent");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn default_cidrs_count_matches_upstream() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ipmasq/ipmasq.go",
            "DefaultCIDRs.Count",
            "tenant-imm-cnt"
        );
        assert_eq!(default_non_masq_cidrs().len(), 11);
    }

    #[test]
    fn default_cidrs_include_rfc1918() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ipmasq/ipmasq.go",
            "DefaultCIDRs.RFC1918",
            "tenant-imm-rfc"
        );
        let list = default_non_masq_cidrs();
        for c in ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"] {
            assert!(list.contains(&c), "missing {}", c);
        }
    }

    #[test]
    fn default_cidrs_include_carrier_grade_nat() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/ipmasq/ipmasq.go",
            "DefaultCIDRs.CGNAT",
            "tenant-imm-cg"
        );
        let list = default_non_masq_cidrs();
        // RFC 6598
        assert!(list.contains(&"100.64.0.0/10"));
    }

    #[test]
    fn link_local_ipv4_is_rfc3927() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "LinkLocal.IPv4", "tenant-imm-ll4");
        assert_eq!(LINK_LOCAL_CIDR_IPV4, "169.254.0.0/16");
    }

    #[test]
    fn link_local_ipv6_is_fe80() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "LinkLocal.IPv6", "tenant-imm-ll6");
        assert_eq!(LINK_LOCAL_CIDR_IPV6, "fe80::/10");
    }

    #[test]
    fn config_json_uses_upstream_field_names() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Config.JSON", "tenant-imm-cfg");
        let raw = r#"{"nonMasqueradeCIDRs":["10.0.0.0/8"],"masqLinkLocal":true,"masqLinkLocalIPv6":false}"#;
        let cfg: Config = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.non_masq_cidrs, vec!["10.0.0.0/8".to_string()]);
        assert!(cfg.masq_link_local_ipv4);
        assert!(!cfg.masq_link_local_ipv6);
    }

    #[test]
    fn config_serialises_with_upstream_field_names() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Config.Round", "tenant-imm-cfgr");
        let cfg = Config {
            non_masq_cidrs: vec!["10.0.0.0/8".into()],
            masq_link_local_ipv4: true,
            masq_link_local_ipv6: false,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("\"nonMasqueradeCIDRs\""));
        assert!(s.contains("\"masqLinkLocal\""));
        assert!(s.contains("\"masqLinkLocalIPv6\""));
    }

    #[test]
    fn map_update_normalises_cidr() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Map.Update", "tenant-imm-mu");
        let mut m = IpMasqMap::new();
        m.update("10.0.0.0/8").unwrap();
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn map_update_invalid_cidr_errors() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Map.UpdateBad", "tenant-imm-mub");
        let mut m = IpMasqMap::new();
        let e = m.update("not-a-cidr").unwrap_err();
        assert_eq!(e, IpMasqError::InvalidCidr("not-a-cidr".to_string()));
    }

    #[test]
    fn map_delete_returns_whether_present() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Map.Delete", "tenant-imm-md");
        let mut m = IpMasqMap::new();
        m.update("10.0.0.0/8").unwrap();
        assert!(m.delete("10.0.0.0/8").unwrap());
        assert!(!m.delete("10.0.0.0/8").unwrap());
    }

    #[test]
    fn map_dump_returns_sorted_cidrs() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Map.Dump", "tenant-imm-dmp");
        let mut m = IpMasqMap::new();
        m.update("192.168.0.0/16").unwrap();
        m.update("10.0.0.0/8").unwrap();
        let d = m.dump();
        // BTreeSet → sorted lexicographically
        assert_eq!(
            d,
            vec!["10.0.0.0/8".to_string(), "192.168.0.0/16".to_string()]
        );
    }

    #[test]
    fn config_empty_defaults_to_no_cidrs_no_linklocal() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Config.Default", "tenant-imm-cd");
        let cfg = Config::default();
        assert!(cfg.non_masq_cidrs.is_empty());
        assert!(!cfg.masq_link_local_ipv4);
        assert!(!cfg.masq_link_local_ipv6);
    }

    #[test]
    fn ipmasq_error_renders_with_inputs() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipmasq/ipmasq.go", "Error.Display", "tenant-imm-err");
        let e = IpMasqError::TenantDenied {
            tenant: TenantId::new("t").expect("test fixture"),
        };
        assert!(format!("{}", e).contains("ipmasq"));
        let e = IpMasqError::InvalidCidr("x".into());
        assert!(format!("{}", e).contains("invalid CIDR"));
    }
}
