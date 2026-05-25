// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CIS K8s Benchmark — etcd controls (2.x).
//!
//! Upstream: kube-bench `cfg/cis-1.10/etcd.yaml`. Apache-2.0 line-port.

use crate::cis_engine::{BinOp, CisRule, TestItem, ValueSource};
use crate::models::{Check, CisLevel, Framework, NodeType, Severity};

pub fn etcd_checks() -> Vec<(Check, CisRule)> {
    let mut out = Vec::new();

    out.push(flag_neq("cis-2.1", "Ensure --cert-file and --key-file are set on etcd",
        "etcd", "--cert-file", "", Severity::Critical));
    out.push(flag_eq("cis-2.2", "Ensure --client-cert-auth is set to true",
        "etcd", "--client-cert-auth", "true", Severity::Critical));
    out.push(flag_eq("cis-2.3", "Ensure --auto-tls is not set to true",
        "etcd", "--auto-tls", "false", Severity::High));
    out.push(flag_neq("cis-2.4", "Ensure --peer-cert-file and --peer-key-file are set",
        "etcd", "--peer-cert-file", "", Severity::Critical));
    out.push(flag_eq("cis-2.5", "Ensure --peer-client-cert-auth is true",
        "etcd", "--peer-client-cert-auth", "true", Severity::High));
    out.push(flag_eq("cis-2.6", "Ensure --peer-auto-tls is not true",
        "etcd", "--peer-auto-tls", "false", Severity::High));
    out.push(flag_neq("cis-2.7", "Ensure --trusted-ca-file is set (unique CA)",
        "etcd", "--trusted-ca-file", "", Severity::High));
    out.push(file_mode("cis-2.8", "Ensure etcd data-dir permissions are 700",
        "/var/lib/etcd", "700", Severity::Critical));
    out.push(file_owner("cis-2.9", "Ensure etcd data-dir ownership is etcd:etcd",
        "/var/lib/etcd", "etcd:etcd", Severity::High));

    out
}

fn make_meta(id: &str, title: &str, severity: Severity) -> Check {
    let mut c = Check::new(id, Framework::CisK8s, NodeType::Etcd, title);
    c.severity = severity;
    c.level = CisLevel::L1;
    c.tags = vec!["cis-etcd".into()];
    c
}

fn flag_eq(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::Eq,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Set {flag}={val} on {bin}.");
    (meta, rule)
}

fn flag_neq(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::NotEq,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Ensure {flag} on {bin} ≠ '{val}'.");
    (meta, rule)
}

fn file_mode(id: &str, title: &str, path: &str, expected: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::FileMode(path.into()),
        op: BinOp::Lte,
        value: expected.into(),
        set: Some(true),
    });
    rule.remediation = format!("chmod {expected} {path}.");
    (meta, rule)
}

fn file_owner(id: &str, title: &str, path: &str, expected: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::FileOwner(path.into()),
        op: BinOp::Eq,
        value: expected.into(),
        set: Some(true),
    });
    rule.remediation = format!("chown {expected} {path}.");
    (meta, rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cis_engine::{CisContext, evaluate_rule};

    #[test]
    fn test_etcd_count_meets_floor() {
        assert!(etcd_checks().len() >= 8);
    }

    #[test]
    fn test_etcd_all_node_type_etcd() {
        for (c, _) in etcd_checks() {
            assert_eq!(c.node_type, NodeType::Etcd);
        }
    }

    #[test]
    fn test_etcd_client_cert_auth() {
        let mut ctx = CisContext::default();
        ctx.set_flag("etcd", "--client-cert-auth", "true");
        let (meta, rule) = etcd_checks().into_iter().find(|(c, _)| c.id == "cis-2.2").unwrap();
        let f = evaluate_rule(&rule, &meta, &ctx, "etcd", "etcd-1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }

    #[test]
    fn test_etcd_data_dir_perms_pass() {
        let mut ctx = CisContext::default();
        ctx.set_file("/var/lib/etcd", "700", "etcd:etcd");
        let (meta, rule) = etcd_checks().into_iter().find(|(c, _)| c.id == "cis-2.8").unwrap();
        let f = evaluate_rule(&rule, &meta, &ctx, "etcd", "etcd-1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }
}
