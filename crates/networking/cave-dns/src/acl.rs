// SPDX-License-Identifier: AGPL-3.0-or-later
//! `acl` plugin — query filtering by source network and query type.
//!
//! Line-by-line port of CoreDNS v1.14.3 `plugin/acl` (`acl.go`). The first
//! matching policy wins; Block -> REFUSED, Filter -> NOERROR with TC set,
//! Allow -> pass to the next plugin.

use crate::{DnsError, DnsResult};
use hickory_proto::rr::RecordType;
use std::net::IpAddr;

const RCODE_NOERROR: u8 = 0;
const RCODE_REFUSED: u8 = 5;

/// Action taken for a matched ACL policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclAction {
    /// Pass the query to the next plugin.
    Allow,
    /// Refuse the query (RCODE REFUSED).
    Block,
    /// Answer NOERROR with the TC bit set.
    Filter,
}

impl AclAction {
    /// Synthetic response as `(rcode, truncated)`; `Allow` yields `None`.
    #[must_use]
    pub fn response_code(&self) -> Option<(u8, bool)> {
        match self {
            AclAction::Allow => None,
            AclAction::Block => Some((RCODE_REFUSED, false)),
            AclAction::Filter => Some((RCODE_NOERROR, true)),
        }
    }
}

/// A CIDR network (IPv4 or IPv6) for source-address matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpCidr {
    base: IpAddr,
    prefix_len: u8,
}

impl IpCidr {
    /// Parse a CIDR in `addr/prefixlen` form.
    pub fn parse(s: &str) -> DnsResult<Self> {
        let (addr_str, len_str) = s
            .split_once('/')
            .ok_or_else(|| DnsError::Config(format!("invalid CIDR: {s}")))?;
        let base: IpAddr = addr_str
            .parse()
            .map_err(|_| DnsError::Config(format!("invalid CIDR address: {s}")))?;
        let prefix_len: u8 = len_str
            .parse()
            .map_err(|_| DnsError::Config(format!("invalid CIDR prefix: {s}")))?;
        let max = if base.is_ipv4() { 32 } else { 128 };
        if prefix_len > max {
            return Err(DnsError::Config(format!("CIDR prefix out of range: {s}")));
        }
        Ok(Self { base, prefix_len })
    }

    /// Whether `addr` falls inside this network. Mixed families never match.
    #[must_use]
    pub fn contains(&self, addr: IpAddr) -> bool {
        match (self.base, addr) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => masked(&net.octets(), &ip.octets(), self.prefix_len),
            (IpAddr::V6(net), IpAddr::V6(ip)) => masked(&net.octets(), &ip.octets(), self.prefix_len),
            _ => false,
        }
    }
}

fn masked(net: &[u8], ip: &[u8], prefix_len: u8) -> bool {
    let mut bits = prefix_len as usize;
    for (n, i) in net.iter().zip(ip.iter()) {
        if bits == 0 {
            break;
        }
        if bits >= 8 {
            if n != i {
                return false;
            }
            bits -= 8;
        } else {
            let mask = 0xffu8 << (8 - bits);
            if (n & mask) != (i & mask) {
                return false;
            }
            bits = 0;
        }
    }
    true
}

/// A single ACL policy.
#[derive(Debug, Clone)]
pub struct AclRule {
    /// Action when this policy matches.
    pub action: AclAction,
    /// Source networks; empty matches any source.
    pub networks: Vec<IpCidr>,
    /// Query types; empty matches any type.
    pub qtypes: Vec<RecordType>,
}

impl AclRule {
    /// Build a policy.
    #[must_use]
    pub fn new(action: AclAction, networks: Vec<IpCidr>, qtypes: Vec<RecordType>) -> Self {
        Self { action, networks, qtypes }
    }

    /// Whether this policy matches a `(source, qtype)` pair.
    #[must_use]
    pub fn matches(&self, source: IpAddr, qtype: RecordType) -> bool {
        let net_ok = self.networks.is_empty() || self.networks.iter().any(|n| n.contains(source));
        let qtype_ok = self.qtypes.is_empty() || self.qtypes.contains(&qtype);
        net_ok && qtype_ok
    }
}

/// An ordered set of ACL policies.
#[derive(Debug, Clone, Default)]
pub struct Acl {
    /// Policies; first match wins.
    pub rules: Vec<AclRule>,
}

impl Acl {
    /// Build an ACL from an ordered list of policies.
    #[must_use]
    pub fn new(rules: Vec<AclRule>) -> Self {
        Self { rules }
    }

    /// Resolve the action for a query, defaulting to `Allow`.
    #[must_use]
    pub fn evaluate(&self, source: IpAddr, qtype: RecordType) -> AclAction {
        for rule in &self.rules {
            if rule.matches(source, qtype) {
                return rule.action;
            }
        }
        AclAction::Allow
    }
}
