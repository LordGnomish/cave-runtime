// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DNS-based service discovery — line-by-line port of prometheus/prometheus
//! `discovery/dns/dns.go` (v3.12.0, source_sha
//! a0524eeca91b19eb60d2b02f8a1c0019954e3405).
//!
//! Upstream periodically resolves a set of DNS names (SRV/A/AAAA/MX/NS) and
//! turns each answer record into a scrape target carrying `__address__` plus
//! the `__meta_dns_*` discovery labels. The network lookup itself
//! (`lookupWithSearchPath`) is host-environment specific; like upstream's
//! injectable `lookupFn`, we take the already-resolved [`DnsRecord`]s as input
//! and port the *deterministic* part — record→label-set assembly and config
//! validation — which is the behaviour every upstream test exercises.

use crate::error::{MetricsError, Result};
use crate::model::Labels;

/// Meta-label prefix used by all Prometheus discovery mechanisms.
pub const META_PREFIX: &str = "__meta_dns_";
/// The address label every target carries.
pub const ADDRESS_LABEL: &str = "__address__";

/// DNS record type to query, mirroring upstream `SDConfig.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsQueryType {
    Srv,
    A,
    Aaaa,
    Mx,
    Ns,
}

/// One resolved DNS answer record (the subset Prometheus consumes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsRecord {
    /// SRV record: service target host + port (port is record-supplied).
    Srv { target: String, port: u16 },
    /// A record: IPv4 address string.
    A(String),
    /// AAAA record: IPv6 address string.
    Aaaa(String),
    /// MX record: mail-exchanger host (port comes from config).
    Mx { target: String },
    /// NS record: nameserver host (port comes from config).
    Ns { target: String },
    /// CNAME: explicitly ignored by upstream (can appear in A queries).
    Cname,
}

/// Configuration for a single DNS-SD job (mirrors upstream `SDConfig`).
#[derive(Debug, Clone)]
pub struct DnsSdConfig {
    pub names: Vec<String>,
    pub kind: DnsQueryType,
    /// Ignored for SRV records; required for A/AAAA/MX/NS.
    pub port: u16,
    pub refresh_interval_ms: i64,
}

impl Default for DnsSdConfig {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            kind: DnsQueryType::Srv,
            port: 0,
            refresh_interval_ms: 30_000,
        }
    }
}

/// A discovered group of targets for one source name (upstream `targetgroup.Group`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsTargetGroup {
    pub source: String,
    pub targets: Vec<Labels>,
}

/// Validate a DNS-SD config exactly like upstream `SDConfig.UnmarshalYAML`:
/// at least one name is required, and every record type except SRV needs a port.
pub fn validate(cfg: &DnsSdConfig) -> Result<()> {
    if cfg.names.is_empty() {
        return Err(MetricsError::Scrape(
            "DNS-SD config must contain at least one record name".to_string(),
        ));
    }
    if cfg.kind != DnsQueryType::Srv && cfg.port == 0 {
        return Err(MetricsError::Scrape(
            "a port is required in DNS-SD configs for all record types except SRV".to_string(),
        ));
    }
    Ok(())
}

/// `net.JoinHostPort` — brackets IPv6 literals (those containing ':').
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

fn trim_trailing_dot(s: &str) -> &str {
    s.trim_end_matches('.')
}

/// Port of `Discovery.refreshOne`: turn the answer records for one name into a
/// target group. Each target carries `__address__` and the full set of
/// `__meta_dns_*` labels (empty string where a field does not apply), exactly
/// as upstream emits them so relabeling configs behave identically.
pub fn targets_from_records(name: &str, port: u16, records: &[DnsRecord]) -> DnsTargetGroup {
    let mut targets = Vec::new();

    for record in records {
        let mut srv_target = String::new();
        let mut srv_port = String::new();
        let mut mx_target = String::new();
        let mut ns_target = String::new();

        let address = match record {
            DnsRecord::Srv {
                target: t,
                port: p,
            } => {
                srv_target = t.clone();
                srv_port = p.to_string();
                host_port(trim_trailing_dot(t), *p)
            }
            DnsRecord::Mx { target: t } => {
                mx_target = t.clone();
                host_port(trim_trailing_dot(t), port)
            }
            DnsRecord::Ns { target: t } => {
                ns_target = t.clone();
                host_port(trim_trailing_dot(t), port)
            }
            DnsRecord::A(ip) => host_port(ip, port),
            DnsRecord::Aaaa(ip) => host_port(ip, port),
            // CNAME responses can occur with "Type: A" requests — skip.
            DnsRecord::Cname => continue,
        };

        let labels = Labels::from_pairs([
            (ADDRESS_LABEL, address.as_str()),
            ("__meta_dns_name", name),
            ("__meta_dns_srv_record_target", srv_target.as_str()),
            ("__meta_dns_srv_record_port", srv_port.as_str()),
            ("__meta_dns_mx_record_target", mx_target.as_str()),
            ("__meta_dns_ns_record_target", ns_target.as_str()),
        ]);
        targets.push(labels);
    }

    DnsTargetGroup {
        source: name.to_string(),
        targets,
    }
}

/// Map a config record type to the wire query type (port of the `qtype` switch
/// in upstream `NewDiscovery`).
pub fn query_type_name(kind: DnsQueryType) -> &'static str {
    match kind {
        DnsQueryType::Srv => "SRV",
        DnsQueryType::A => "A",
        DnsQueryType::Aaaa => "AAAA",
        DnsQueryType::Mx => "MX",
        DnsQueryType::Ns => "NS",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_names() {
        let cfg = DnsSdConfig {
            names: vec![],
            kind: DnsQueryType::Srv,
            ..DnsSdConfig::default()
        };
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn host_port_brackets_ipv6() {
        assert_eq!(host_port("192.0.2.2", 80), "192.0.2.2:80");
        assert_eq!(host_port("::1", 80), "[::1]:80");
    }
}
