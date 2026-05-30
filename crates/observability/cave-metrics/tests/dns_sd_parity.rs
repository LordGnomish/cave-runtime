// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Parity tests for DNS service discovery, ported from
//! prometheus/prometheus `discovery/dns/dns_test.go` (v3.12.0, source_sha
//! a0524eeca91b19eb60d2b02f8a1c0019954e3405).
//!
//! Mirrors `TestDNS` cases: A / AAAA / SRV / MX / NS record → target group
//! label assembly, CNAME skipping, and `SDConfig.UnmarshalYAML` validation.

use cave_metrics::model::Labels;
use cave_metrics::scrape::dns_sd::{
    DnsQueryType, DnsRecord, DnsSdConfig, targets_from_records, validate,
};

fn addr(lb: &Labels) -> &str {
    lb.get("__address__").unwrap_or("")
}

// ── A / AAAA ────────────────────────────────────────────────────────────────

#[test]
fn a_record_builds_address_and_meta_labels() {
    let tg = targets_from_records("web.example.com.", 80, &[DnsRecord::A("192.0.2.2".into())]);
    assert_eq!(tg.source, "web.example.com.");
    assert_eq!(tg.targets.len(), 1);
    let t = &tg.targets[0];
    assert_eq!(addr(t), "192.0.2.2:80");
    assert_eq!(t.get("__meta_dns_name"), Some("web.example.com."));
    // For an A record all record-specific meta labels are present but empty.
    assert_eq!(t.get("__meta_dns_srv_record_target"), Some(""));
    assert_eq!(t.get("__meta_dns_srv_record_port"), Some(""));
    assert_eq!(t.get("__meta_dns_mx_record_target"), Some(""));
    assert_eq!(t.get("__meta_dns_ns_record_target"), Some(""));
}

#[test]
fn aaaa_record_brackets_ipv6_address() {
    let tg = targets_from_records("web.example.com.", 80, &[DnsRecord::Aaaa("::1".into())]);
    assert_eq!(addr(&tg.targets[0]), "[::1]:80");
}

// ── SRV ───────────────────────────────────────────────────────────────────

#[test]
fn srv_record_uses_record_port_and_trims_address_dot() {
    let tg = targets_from_records(
        "_sql._tcp.example.com.",
        0,
        &[DnsRecord::Srv {
            target: "db1.example.com.".into(),
            port: 5432,
        }],
    );
    let t = &tg.targets[0];
    // __address__ trims the trailing dot and uses the record's own port.
    assert_eq!(addr(t), "db1.example.com:5432");
    // The meta labels preserve the original (rooted) target + port.
    assert_eq!(t.get("__meta_dns_srv_record_target"), Some("db1.example.com."));
    assert_eq!(t.get("__meta_dns_srv_record_port"), Some("5432"));
    assert_eq!(t.get("__meta_dns_name"), Some("_sql._tcp.example.com."));
}

// ── MX / NS ─────────────────────────────────────────────────────────────────

#[test]
fn mx_record_uses_config_port() {
    let tg = targets_from_records(
        "example.com.",
        25,
        &[DnsRecord::Mx {
            target: "mail.example.com.".into(),
        }],
    );
    let t = &tg.targets[0];
    assert_eq!(addr(t), "mail.example.com:25");
    assert_eq!(t.get("__meta_dns_mx_record_target"), Some("mail.example.com."));
}

#[test]
fn ns_record_uses_config_port() {
    let tg = targets_from_records(
        "example.com.",
        53,
        &[DnsRecord::Ns {
            target: "ns1.example.com.".into(),
        }],
    );
    let t = &tg.targets[0];
    assert_eq!(addr(t), "ns1.example.com:53");
    assert_eq!(t.get("__meta_dns_ns_record_target"), Some("ns1.example.com."));
}

// ── CNAME skipping ───────────────────────────────────────────────────────────

#[test]
fn cname_records_are_skipped() {
    let tg = targets_from_records(
        "web.example.com.",
        80,
        &[
            DnsRecord::Cname,
            DnsRecord::A("192.0.2.9".into()),
            DnsRecord::Cname,
        ],
    );
    // Only the A record yields a target.
    assert_eq!(tg.targets.len(), 1);
    assert_eq!(addr(&tg.targets[0]), "192.0.2.9:80");
}

#[test]
fn multiple_answers_produce_multiple_targets() {
    let tg = targets_from_records(
        "web.example.com.",
        80,
        &[
            DnsRecord::A("192.0.2.2".into()),
            DnsRecord::A("192.0.2.3".into()),
        ],
    );
    assert_eq!(tg.targets.len(), 2);
}

// ── Config validation ────────────────────────────────────────────────────────

#[test]
fn validate_rejects_empty_names() {
    let cfg = DnsSdConfig {
        names: vec![],
        kind: DnsQueryType::Srv,
        ..DnsSdConfig::default()
    };
    assert!(validate(&cfg).is_err());
}

#[test]
fn validate_requires_port_for_non_srv() {
    let cfg = DnsSdConfig {
        names: vec!["web.example.com.".into()],
        kind: DnsQueryType::A,
        port: 0,
        ..DnsSdConfig::default()
    };
    assert!(validate(&cfg).is_err(), "A records require an explicit port");

    let ok = DnsSdConfig {
        names: vec!["web.example.com.".into()],
        kind: DnsQueryType::A,
        port: 80,
        ..DnsSdConfig::default()
    };
    assert!(validate(&ok).is_ok());
}

#[test]
fn validate_allows_srv_without_port() {
    let cfg = DnsSdConfig {
        names: vec!["_sql._tcp.example.com.".into()],
        kind: DnsQueryType::Srv,
        port: 0,
        ..DnsSdConfig::default()
    };
    assert!(validate(&cfg).is_ok());
}
