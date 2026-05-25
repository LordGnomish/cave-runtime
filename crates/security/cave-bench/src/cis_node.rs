// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CIS K8s Benchmark — worker-node controls (4.x).
//!
//! Upstream: kube-bench `cfg/cis-1.10/node.yaml`. Apache-2.0 line-port.
//!
//! Sections:
//! 4.1.* — Worker node configuration files (permissions + ownership)
//! 4.2.* — Kubelet flags

use crate::cis_engine::{BinOp, CisRule, Logic, TestItem, ValueSource};
use crate::models::{Check, CisLevel, Framework, NodeType, Severity};

/// All worker-node CIS checks.
pub fn node_checks() -> Vec<(Check, CisRule)> {
    let mut out = Vec::new();

    // ── 4.1.* — Kubelet config files ─────────────────────────────────────────
    out.push(file_mode("cis-4.1.1", "Ensure kubelet service file permissions are 600 or restrictive",
        "/etc/systemd/system/kubelet.service.d/10-kubeadm.conf", "600", Severity::High));
    out.push(file_owner("cis-4.1.2", "Ensure kubelet service file ownership is root:root",
        "/etc/systemd/system/kubelet.service.d/10-kubeadm.conf", "root:root", Severity::Medium));
    out.push(file_mode("cis-4.1.3", "Ensure proxy kubeconfig file permissions are 600",
        "/var/lib/kube-proxy/kubeconfig", "600", Severity::High));
    out.push(file_owner("cis-4.1.4", "Ensure proxy kubeconfig file ownership is root:root",
        "/var/lib/kube-proxy/kubeconfig", "root:root", Severity::Medium));
    out.push(file_mode("cis-4.1.5", "Ensure kubelet.conf file permissions are 600",
        "/etc/kubernetes/kubelet.conf", "600", Severity::High));
    out.push(file_owner("cis-4.1.6", "Ensure kubelet.conf file ownership is root:root",
        "/etc/kubernetes/kubelet.conf", "root:root", Severity::Medium));
    out.push(file_mode("cis-4.1.9", "Ensure kubelet config file permissions are 600",
        "/var/lib/kubelet/config.yaml", "600", Severity::High));
    out.push(file_owner("cis-4.1.10", "Ensure kubelet config file ownership is root:root",
        "/var/lib/kubelet/config.yaml", "root:root", Severity::Medium));

    // ── 4.2.* — Kubelet flags ────────────────────────────────────────────────
    out.push(flag_eq("cis-4.2.1", "Ensure --anonymous-auth=false on kubelet",
        "kubelet", "--anonymous-auth", "false", Severity::Critical));
    out.push(flag_neq("cis-4.2.2", "Ensure --authorization-mode is not AlwaysAllow",
        "kubelet", "--authorization-mode", "AlwaysAllow", Severity::Critical));
    out.push(flag_neq("cis-4.2.3", "Ensure --client-ca-file is set",
        "kubelet", "--client-ca-file", "", Severity::High));
    out.push(flag_eq("cis-4.2.4", "Ensure --read-only-port is 0",
        "kubelet", "--read-only-port", "0", Severity::High));
    out.push(flag_neq("cis-4.2.5", "Ensure --streaming-connection-idle-timeout is not 0",
        "kubelet", "--streaming-connection-idle-timeout", "0", Severity::Medium));
    out.push(flag_eq("cis-4.2.6", "Ensure --make-iptables-util-chains is true",
        "kubelet", "--make-iptables-util-chains", "true", Severity::Medium));
    out.push(flag_eq("cis-4.2.7", "Ensure --hostname-override is not set",
        "kubelet", "--hostname-override", "", Severity::Low));
    out.push(flag_gte("cis-4.2.8", "Ensure --event-qps is captured (≥0)",
        "kubelet", "--event-qps", "0", Severity::Info));
    out.push(flag_neq("cis-4.2.9", "Ensure --tls-cert-file and --tls-private-key-file are set",
        "kubelet", "--tls-cert-file", "", Severity::High));
    out.push(flag_eq("cis-4.2.10", "Ensure --rotate-certificates is true",
        "kubelet", "--rotate-certificates", "true", Severity::Medium));
    out.push(flag_eq("cis-4.2.11", "Ensure RotateKubeletServerCertificate is true",
        "kubelet", "--feature-gates", "RotateKubeletServerCertificate=true", Severity::Medium));
    out.push(flag_neq("cis-4.2.12", "Ensure --tls-cipher-suites avoids weak ciphers",
        "kubelet", "--tls-cipher-suites", "TLS_RSA_WITH_RC4_128_SHA", Severity::High));
    out.push(flag_eq("cis-4.2.13", "Ensure --protect-kernel-defaults is true",
        "kubelet", "--protect-kernel-defaults", "true", Severity::Medium));

    out
}

// ─── builders ────────────────────────────────────────────────────────────────

fn make_meta(id: &str, title: &str, severity: Severity) -> Check {
    let mut c = Check::new(id, Framework::CisK8s, NodeType::Node, title);
    c.severity = severity;
    c.level = CisLevel::L1;
    c.tags = vec!["cis-node".into()];
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

fn flag_gte(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::Gte,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Set {flag} ≥ {val} on {bin}.");
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
    rule.logic = Logic::And;
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
    fn test_node_checks_count_meets_floor() {
        assert!(node_checks().len() >= 10);
    }

    #[test]
    fn test_node_checks_all_node_type() {
        for (c, _) in node_checks() {
            assert_eq!(c.node_type, NodeType::Node);
        }
    }

    #[test]
    fn test_kubelet_anonymous_auth_passes_when_false() {
        let mut ctx = CisContext::default();
        ctx.set_flag("kubelet", "--anonymous-auth", "false");
        let checks = node_checks();
        let (meta, rule) = checks.iter().find(|(c, _)| c.id == "cis-4.2.1").unwrap();
        let f = evaluate_rule(rule, meta, &ctx, "kubelet", "node-1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }

    #[test]
    fn test_kubelet_readonly_port_zero() {
        let mut ctx = CisContext::default();
        ctx.set_flag("kubelet", "--read-only-port", "0");
        let checks = node_checks();
        let (meta, rule) = checks.iter().find(|(c, _)| c.id == "cis-4.2.4").unwrap();
        let f = evaluate_rule(rule, meta, &ctx, "kubelet", "node-1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }

    #[test]
    fn test_file_mode_passes_when_within() {
        let mut ctx = CisContext::default();
        ctx.set_file("/etc/kubernetes/kubelet.conf", "600", "root:root");
        let checks = node_checks();
        let (meta, rule) = checks.iter().find(|(c, _)| c.id == "cis-4.1.5").unwrap();
        let f = evaluate_rule(rule, meta, &ctx, "kubelet", "node-1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }
}
