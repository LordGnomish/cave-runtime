// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared data models — IP, MAC, CIDR, label sets, identity.
//!
//! Upstream: pkg/{addressing,labels,identity,policy/api} types.go.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::error::{CiliumError, Result};

/// 16-bit numeric Cilium identity (NUMERIC_IDENTITY_MIN..NUMERIC_IDENTITY_MAX).
/// Identity 1..127 are reserved (host, world, init, …); user identities start
/// at 256. See pkg/identity/numericidentity.go upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NumericIdentity(pub u32);

impl NumericIdentity {
    /// Reserved identity for the local host kernel/agent.
    pub const HOST: NumericIdentity = NumericIdentity(1);
    /// Reserved identity for "the world" — everything outside the cluster.
    pub const WORLD: NumericIdentity = NumericIdentity(2);
    /// Reserved identity for unmanaged pod creation events.
    pub const UNMANAGED: NumericIdentity = NumericIdentity(3);
    /// Reserved identity for the init phase of an endpoint.
    pub const INIT: NumericIdentity = NumericIdentity(5);
    /// Reserved identity for remote-node-IP-cache entries.
    pub const REMOTE_NODE: NumericIdentity = NumericIdentity(6);
    /// Reserved identity for kube-apiserver flow.
    pub const KUBE_APISERVER: NumericIdentity = NumericIdentity(7);
    /// Reserved identity for ingress flows.
    pub const INGRESS: NumericIdentity = NumericIdentity(8);
    /// Reserved identity for unknown/no-identity flows.
    pub const UNKNOWN: NumericIdentity = NumericIdentity(0);
    /// Lowest identity available to label-set-derived identities.
    pub const MIN_LOCAL: u32 = 256;
    /// Highest local-scope identity (24-bit ceiling for FQDN selectors).
    pub const MAX_LOCAL: u32 = 0xFF_FFFF;

    pub fn is_reserved(self) -> bool {
        self.0 < Self::MIN_LOCAL
    }

    pub fn is_world(self) -> bool {
        self == Self::WORLD
    }

    pub fn is_host(self) -> bool {
        self == Self::HOST
    }
}

impl fmt::Display for NumericIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Single label `source:key=value` (e.g. `k8s:io.kubernetes.pod.namespace=kube-system`).
/// Cilium identity is derived from the sorted, hashed set of these labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Label {
    pub source: String,
    pub key: String,
    pub value: String,
}

impl Label {
    pub fn new(source: impl Into<String>, key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { source: source.into(), key: key.into(), value: value.into() }
    }

    pub fn parse(s: &str) -> Result<Self> {
        let (src, rest) = match s.split_once(':') {
            Some((src, rest)) => (src.to_string(), rest),
            None => ("any".to_string(), s),
        };
        let (k, v) = match rest.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (rest.to_string(), String::new()),
        };
        Ok(Label { source: src, key: k, value: v })
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}={}", self.source, self.key, self.value)
    }
}

/// A set of labels — canonicalised by sorting. Identity derivation hashes
/// the sorted display form so two label sets are mapped to the same
/// identity iff they are set-equal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelSet {
    labels: Vec<Label>,
}

impl LabelSet {
    pub fn new() -> Self {
        Self { labels: Vec::new() }
    }

    pub fn from_iter<I: IntoIterator<Item = Label>>(iter: I) -> Self {
        let mut s = Self::new();
        for l in iter {
            s.insert(l);
        }
        s
    }

    pub fn insert(&mut self, l: Label) {
        if !self.labels.iter().any(|x| x == &l) {
            self.labels.push(l);
            self.labels.sort();
        }
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Label> {
        self.labels.iter()
    }

    /// Canonical text form: `src:k=v;src:k=v;...` (sorted).
    pub fn canonical(&self) -> String {
        self.labels.iter().map(|l| l.to_string()).collect::<Vec<_>>().join(";")
    }

    /// Identity selector match — every label in `selector` must appear here.
    pub fn matches_selector(&self, selector: &LabelSet) -> bool {
        selector.labels.iter().all(|s| self.labels.contains(s))
    }

    /// Convert a `BTreeMap<String,String>` (k8s style) to a LabelSet under
    /// the given `source` (`k8s`, `container`, `reserved`, `unspec`).
    pub fn from_map(source: &str, map: &BTreeMap<String, String>) -> Self {
        let mut s = Self::new();
        for (k, v) in map {
            s.insert(Label::new(source, k, v));
        }
        s
    }
}

/// CIDR — IPv4 or IPv6 prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cidr {
    pub addr: IpAddr,
    pub prefix_len: u8,
}

impl Cidr {
    pub fn new(addr: IpAddr, prefix_len: u8) -> Result<Self> {
        let max = match addr {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix_len > max {
            return Err(CiliumError::InvalidCidr(format!("prefix {} > {}", prefix_len, max)));
        }
        Ok(Self { addr, prefix_len })
    }

    /// Canonicalise — clear host bits.
    pub fn network(&self) -> Self {
        match self.addr {
            IpAddr::V4(v4) => {
                let bits = u32::from(v4);
                let mask = if self.prefix_len == 0 {
                    0
                } else {
                    (!0u32).checked_shl(32 - self.prefix_len as u32).unwrap_or(0)
                };
                let net = bits & mask;
                Self { addr: IpAddr::V4(Ipv4Addr::from(net)), prefix_len: self.prefix_len }
            }
            IpAddr::V6(v6) => {
                let bits = u128::from(v6);
                let mask = if self.prefix_len == 0 {
                    0
                } else {
                    (!0u128).checked_shl(128 - self.prefix_len as u32).unwrap_or(0)
                };
                let net = bits & mask;
                Self { addr: IpAddr::V6(Ipv6Addr::from(net)), prefix_len: self.prefix_len }
            }
        }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(test)) => {
                let bits = u32::from(net);
                let test = u32::from(test);
                let mask = if self.prefix_len == 0 {
                    0
                } else {
                    (!0u32).checked_shl(32 - self.prefix_len as u32).unwrap_or(0)
                };
                (bits & mask) == (test & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(test)) => {
                let bits = u128::from(net);
                let test = u128::from(test);
                let mask = if self.prefix_len == 0 {
                    0
                } else {
                    (!0u128).checked_shl(128 - self.prefix_len as u32).unwrap_or(0)
                };
                (bits & mask) == (test & mask)
            }
            _ => false,
        }
    }

    pub fn capacity(&self) -> u128 {
        match self.addr {
            IpAddr::V4(_) => 1u128 << (32 - self.prefix_len as u32),
            IpAddr::V6(_) => 1u128 << (128 - self.prefix_len as u32),
        }
    }
}

impl FromStr for Cidr {
    type Err = CiliumError;

    fn from_str(s: &str) -> Result<Self> {
        let (a, p) = s.split_once('/').ok_or_else(|| CiliumError::InvalidCidr(s.into()))?;
        let addr: IpAddr = a.parse().map_err(|_| CiliumError::InvalidCidr(s.into()))?;
        let prefix: u8 = p.parse().map_err(|_| CiliumError::InvalidCidr(s.into()))?;
        Cidr::new(addr, prefix)
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix_len)
    }
}

/// L4 protocol — TCP/UDP/SCTP/ICMP/Any (Cilium uses TCP/UDP/SCTP for L4 rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum L4Protocol {
    Tcp,
    Udp,
    Sctp,
    Icmp,
    Any,
}

impl L4Protocol {
    pub fn ip_proto(self) -> u8 {
        match self {
            L4Protocol::Tcp => 6,
            L4Protocol::Udp => 17,
            L4Protocol::Sctp => 132,
            L4Protocol::Icmp => 1,
            L4Protocol::Any => 0,
        }
    }
}

impl fmt::Display for L4Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            L4Protocol::Tcp => "TCP",
            L4Protocol::Udp => "UDP",
            L4Protocol::Sctp => "SCTP",
            L4Protocol::Icmp => "ICMP",
            L4Protocol::Any => "ANY",
        };
        f.write_str(s)
    }
}

/// Traffic direction relative to an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrafficDirection {
    Ingress,
    Egress,
}

/// Policy verdict — match upstream policy.Verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Allowed — drop through to the next stage.
    Allow,
    /// Denied — drop the packet.
    Deny,
    /// Redirect to an L7 proxy (Envoy / DNS proxy).
    Redirect,
    /// Audited — log the flow but allow.
    Audit,
}

/// L4 5-tuple. Source port is reset on egress NAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tuple {
    pub src: IpAddr,
    pub dst: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub proto: L4Protocol,
}

impl Tuple {
    pub fn new(src: IpAddr, dst: IpAddr, src_port: u16, dst_port: u16, proto: L4Protocol) -> Self {
        Self { src, dst, src_port, dst_port, proto }
    }

    /// The "reply" tuple — swap src/dst (used when conntrack matches a
    /// returning packet to an established flow).
    pub fn reply(self) -> Self {
        Self {
            src: self.dst,
            dst: self.src,
            src_port: self.dst_port,
            dst_port: self.src_port,
            proto: self.proto,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_parse_with_source() {
        let l = Label::parse("k8s:app=web").unwrap();
        assert_eq!(l.source, "k8s");
        assert_eq!(l.key, "app");
        assert_eq!(l.value, "web");
        assert_eq!(l.to_string(), "k8s:app=web");
    }

    #[test]
    fn label_parse_without_source_defaults_to_any() {
        let l = Label::parse("app=web").unwrap();
        assert_eq!(l.source, "any");
        assert_eq!(l.key, "app");
        assert_eq!(l.value, "web");
    }

    #[test]
    fn label_parse_no_value() {
        let l = Label::parse("k8s:role").unwrap();
        assert_eq!(l.key, "role");
        assert_eq!(l.value, "");
    }

    #[test]
    fn labelset_dedupes_and_sorts() {
        let mut s = LabelSet::new();
        s.insert(Label::new("k8s", "b", "2"));
        s.insert(Label::new("k8s", "a", "1"));
        s.insert(Label::new("k8s", "b", "2"));
        assert_eq!(s.len(), 2);
        assert_eq!(s.canonical(), "k8s:a=1;k8s:b=2");
    }

    #[test]
    fn labelset_selector_subset() {
        let pod = LabelSet::from_iter([
            Label::new("k8s", "app", "web"),
            Label::new("k8s", "tier", "front"),
        ]);
        let sel = LabelSet::from_iter([Label::new("k8s", "app", "web")]);
        assert!(pod.matches_selector(&sel));
        let bad = LabelSet::from_iter([Label::new("k8s", "app", "db")]);
        assert!(!pod.matches_selector(&bad));
    }

    #[test]
    fn labelset_from_map() {
        let mut m = BTreeMap::new();
        m.insert("app".into(), "web".into());
        m.insert("zone".into(), "eu".into());
        let s = LabelSet::from_map("k8s", &m);
        assert_eq!(s.len(), 2);
        assert!(s.iter().any(|l| l.key == "app"));
    }

    #[test]
    fn numeric_identity_reserved() {
        assert!(NumericIdentity::HOST.is_host());
        assert!(NumericIdentity::WORLD.is_world());
        assert!(NumericIdentity::HOST.is_reserved());
        assert!(!NumericIdentity(1024).is_reserved());
    }

    #[test]
    fn cidr_parse_v4_v6() {
        let c4: Cidr = "10.0.0.0/8".parse().unwrap();
        assert_eq!(c4.prefix_len, 8);
        let c6: Cidr = "fd00::/8".parse().unwrap();
        assert_eq!(c6.prefix_len, 8);
    }

    #[test]
    fn cidr_invalid_prefix() {
        let bad: Result<Cidr> = "10.0.0.0/33".parse();
        assert!(bad.is_err());
    }

    #[test]
    fn cidr_network_canonicalisation() {
        let c: Cidr = "10.1.2.3/24".parse().unwrap();
        let n = c.network();
        assert_eq!(n.to_string(), "10.1.2.0/24");
    }

    #[test]
    fn cidr_contains_v4() {
        let c: Cidr = "10.0.0.0/8".parse().unwrap();
        assert!(c.contains("10.1.2.3".parse().unwrap()));
        assert!(!c.contains("11.0.0.0".parse().unwrap()));
    }

    #[test]
    fn cidr_capacity_v4_24() {
        let c: Cidr = "10.0.0.0/24".parse().unwrap();
        assert_eq!(c.capacity(), 256);
    }

    #[test]
    fn cidr_contains_zero_prefix() {
        let c: Cidr = "0.0.0.0/0".parse().unwrap();
        assert!(c.contains("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn cidr_v6_contains() {
        let c: Cidr = "fd00::/8".parse().unwrap();
        assert!(c.contains("fd00:abcd::1".parse().unwrap()));
        assert!(!c.contains("fe80::1".parse().unwrap()));
    }

    #[test]
    fn l4_proto_ip_numbers() {
        assert_eq!(L4Protocol::Tcp.ip_proto(), 6);
        assert_eq!(L4Protocol::Udp.ip_proto(), 17);
        assert_eq!(L4Protocol::Sctp.ip_proto(), 132);
        assert_eq!(L4Protocol::Icmp.ip_proto(), 1);
        assert_eq!(L4Protocol::Any.ip_proto(), 0);
    }

    #[test]
    fn tuple_reply_swaps() {
        let t = Tuple::new(
            "10.0.0.1".parse().unwrap(),
            "10.0.0.2".parse().unwrap(),
            1000,
            80,
            L4Protocol::Tcp,
        );
        let r = t.reply();
        assert_eq!(r.src, t.dst);
        assert_eq!(r.dst, t.src);
        assert_eq!(r.src_port, t.dst_port);
        assert_eq!(r.dst_port, t.src_port);
    }
}
