// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CIS K8s Benchmark — master-node controls (1.x).
//!
//! Upstream: kube-bench `cfg/cis-1.10/master.yaml`.
//! License: Apache-2.0 (line-port of titles + remediations).
//!
//! Sections covered:
//! 1.1.* — Master node configuration files (permissions + ownership)
//! 1.2.* — API Server flags
//! 1.3.* — Controller Manager flags
//! 1.4.* — Scheduler flags

use crate::cis_engine::{BinOp, CisRule, Logic, TestItem, ValueSource};
use crate::models::{Check, CisLevel, Framework, NodeType, Severity};

/// (Check metadata, CIS rule) pairs for the master-node controls.
pub fn master_checks() -> Vec<(Check, CisRule)> {
    let mut out = Vec::new();

    // ── 1.1.* — Master node configuration files ──────────────────────────────
    out.push(file_mode_check(
        "cis-1.1.1",
        "Ensure that the API server pod specification file permissions are set to 600 or more restrictive",
        "/etc/kubernetes/manifests/kube-apiserver.yaml",
        "600",
        Severity::High,
    ));
    out.push(file_mode_check(
        "cis-1.1.2",
        "Ensure that the API server pod specification file ownership is set to root:root",
        "/etc/kubernetes/manifests/kube-apiserver.yaml",
        "root:root",
        Severity::Medium,
    ));
    out.push(file_mode_check(
        "cis-1.1.3",
        "Ensure that the controller manager pod spec file permissions are 600 or more restrictive",
        "/etc/kubernetes/manifests/kube-controller-manager.yaml",
        "600",
        Severity::High,
    ));
    out.push(file_mode_check(
        "cis-1.1.5",
        "Ensure that the scheduler pod specification file permissions are 600 or more restrictive",
        "/etc/kubernetes/manifests/kube-scheduler.yaml",
        "600",
        Severity::High,
    ));
    out.push(file_mode_check(
        "cis-1.1.7",
        "Ensure that the etcd pod specification file permissions are 600 or more restrictive",
        "/etc/kubernetes/manifests/etcd.yaml",
        "600",
        Severity::High,
    ));
    out.push(file_mode_check(
        "cis-1.1.11",
        "Ensure that the etcd data directory permissions are set to 700 or more restrictive",
        "/var/lib/etcd",
        "700",
        Severity::Critical,
    ));

    // ── 1.2.* — API Server flags ──────────────────────────────────────────────
    out.push(flag_eq_check(
        "cis-1.2.1",
        "Ensure that the --anonymous-auth argument is set to false",
        "apiserver",
        "--anonymous-auth",
        "false",
        Severity::Critical,
    ));
    out.push(flag_neq_check(
        "cis-1.2.2",
        "Ensure that the --token-auth-file parameter is not set",
        "apiserver",
        "--token-auth-file",
        "",
        Severity::Critical,
    ));
    out.push(flag_eq_check(
        "cis-1.2.3",
        "Ensure that the --DenyServiceExternalIPs is set",
        "apiserver",
        "--enable-admission-plugins",
        "DenyServiceExternalIPs",
        Severity::High,
    ));
    out.push(flag_has_check(
        "cis-1.2.4",
        "Ensure that the --kubelet-client-certificate and --kubelet-client-key arguments are set",
        "apiserver",
        "--kubelet-client-certificate",
        ".crt",
        Severity::High,
    ));
    out.push(flag_eq_check(
        "cis-1.2.5",
        "Ensure that the --kubelet-certificate-authority argument is set",
        "apiserver",
        "--kubelet-certificate-authority",
        "/etc/kubernetes/pki/ca.crt",
        Severity::High,
    ));
    out.push(flag_has_check(
        "cis-1.2.6",
        "Ensure that the --authorization-mode argument is not set to AlwaysAllow",
        "apiserver",
        "--authorization-mode",
        "RBAC",
        Severity::Critical,
    ));
    out.push(flag_has_check(
        "cis-1.2.7",
        "Ensure that the --authorization-mode argument includes Node",
        "apiserver",
        "--authorization-mode",
        "Node",
        Severity::High,
    ));
    out.push(flag_has_check(
        "cis-1.2.8",
        "Ensure that the --authorization-mode argument includes RBAC",
        "apiserver",
        "--authorization-mode",
        "RBAC",
        Severity::High,
    ));
    out.push(flag_has_check(
        "cis-1.2.9",
        "Ensure that the admission control plugin EventRateLimit is set",
        "apiserver",
        "--enable-admission-plugins",
        "EventRateLimit",
        Severity::Medium,
    ));
    out.push(flag_has_check(
        "cis-1.2.10",
        "Ensure that the admission control plugin AlwaysAdmit is not set",
        "apiserver",
        "--disable-admission-plugins",
        "AlwaysAdmit",
        Severity::High,
    ));
    out.push(flag_has_check(
        "cis-1.2.13",
        "Ensure that the admission control plugin NamespaceLifecycle is set",
        "apiserver",
        "--enable-admission-plugins",
        "NamespaceLifecycle",
        Severity::Medium,
    ));
    out.push(flag_has_check(
        "cis-1.2.14",
        "Ensure that the admission control plugin NodeRestriction is set",
        "apiserver",
        "--enable-admission-plugins",
        "NodeRestriction",
        Severity::High,
    ));
    out.push(flag_eq_check(
        "cis-1.2.15",
        "Ensure that the --profiling argument is set to false",
        "apiserver",
        "--profiling",
        "false",
        Severity::Low,
    ));
    out.push(flag_neq_check(
        "cis-1.2.16",
        "Ensure that the --audit-log-path argument is set",
        "apiserver",
        "--audit-log-path",
        "",
        Severity::High,
    ));
    out.push(flag_gte_check(
        "cis-1.2.17",
        "Ensure that the --audit-log-maxage argument is set to 30 or as appropriate",
        "apiserver",
        "--audit-log-maxage",
        "30",
        Severity::Medium,
    ));
    out.push(flag_gte_check(
        "cis-1.2.18",
        "Ensure that the --audit-log-maxbackup argument is set to 10 or as appropriate",
        "apiserver",
        "--audit-log-maxbackup",
        "10",
        Severity::Medium,
    ));
    out.push(flag_gte_check(
        "cis-1.2.19",
        "Ensure that the --audit-log-maxsize argument is set to 100 or as appropriate",
        "apiserver",
        "--audit-log-maxsize",
        "100",
        Severity::Medium,
    ));
    out.push(flag_eq_check(
        "cis-1.2.20",
        "Ensure that the --service-account-lookup argument is set to true",
        "apiserver",
        "--service-account-lookup",
        "true",
        Severity::High,
    ));
    out.push(flag_neq_check(
        "cis-1.2.21",
        "Ensure that the --service-account-key-file argument is set as appropriate",
        "apiserver",
        "--service-account-key-file",
        "",
        Severity::High,
    ));
    out.push(flag_eq_check(
        "cis-1.2.22",
        "Ensure that the --tls-cert-file and --tls-private-key-file arguments are set",
        "apiserver",
        "--tls-cert-file",
        "/etc/kubernetes/pki/apiserver.crt",
        Severity::High,
    ));
    out.push(flag_eq_check(
        "cis-1.2.23",
        "Ensure that the --client-ca-file argument is set",
        "apiserver",
        "--client-ca-file",
        "/etc/kubernetes/pki/ca.crt",
        Severity::High,
    ));
    out.push(flag_neq_check(
        "cis-1.2.24",
        "Ensure that the --etcd-certfile and --etcd-keyfile arguments are set",
        "apiserver",
        "--etcd-certfile",
        "",
        Severity::Critical,
    ));
    out.push(flag_eq_check(
        "cis-1.2.25",
        "Ensure that the --encryption-provider-config argument is set as appropriate",
        "apiserver",
        "--encryption-provider-config",
        "/etc/kubernetes/encryption-config.yaml",
        Severity::Critical,
    ));

    // ── 1.3.* — Controller Manager ─────────────────────────────────────────────
    out.push(flag_gte_check(
        "cis-1.3.1",
        "Ensure that the --terminated-pod-gc-threshold argument is set as appropriate",
        "controller-manager",
        "--terminated-pod-gc-threshold",
        "12500",
        Severity::Low,
    ));
    out.push(flag_eq_check(
        "cis-1.3.2",
        "Ensure that the --profiling argument is set to false",
        "controller-manager",
        "--profiling",
        "false",
        Severity::Low,
    ));
    out.push(flag_eq_check(
        "cis-1.3.3",
        "Ensure that the --use-service-account-credentials is true",
        "controller-manager",
        "--use-service-account-credentials",
        "true",
        Severity::High,
    ));
    out.push(flag_neq_check(
        "cis-1.3.4",
        "Ensure that the --service-account-private-key-file is set as appropriate",
        "controller-manager",
        "--service-account-private-key-file",
        "",
        Severity::High,
    ));
    out.push(flag_eq_check(
        "cis-1.3.5",
        "Ensure that the --root-ca-file is set as appropriate",
        "controller-manager",
        "--root-ca-file",
        "/etc/kubernetes/pki/ca.crt",
        Severity::High,
    ));
    out.push(flag_eq_check(
        "cis-1.3.6",
        "Ensure that the RotateKubeletServerCertificate argument is set to true",
        "controller-manager",
        "--feature-gates",
        "RotateKubeletServerCertificate=true",
        Severity::Medium,
    ));
    out.push(flag_eq_check(
        "cis-1.3.7",
        "Ensure that the --bind-address is set to 127.0.0.1",
        "controller-manager",
        "--bind-address",
        "127.0.0.1",
        Severity::Medium,
    ));

    // ── 1.4.* — Scheduler ──────────────────────────────────────────────────────
    out.push(flag_eq_check(
        "cis-1.4.1",
        "Ensure that the --profiling argument is set to false (scheduler)",
        "scheduler",
        "--profiling",
        "false",
        Severity::Low,
    ));
    out.push(flag_eq_check(
        "cis-1.4.2",
        "Ensure that the --bind-address is set to 127.0.0.1 (scheduler)",
        "scheduler",
        "--bind-address",
        "127.0.0.1",
        Severity::Medium,
    ));

    out
}

// ─── helpers (templated check builders) ──────────────────────────────────────

fn make_meta(id: &str, title: &str, severity: Severity) -> Check {
    let mut c = Check::new(id, Framework::CisK8s, NodeType::Master, title);
    c.severity = severity;
    c.level = CisLevel::L1;
    c.tags = vec!["cis-master".into()];
    c
}

fn flag_eq_check(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::Eq,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Set {flag}={val} on the {bin} process.");
    (meta, rule)
}

fn flag_neq_check(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::NotEq,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Ensure {flag} on {bin} is not equal to '{val}'.");
    (meta, rule)
}

fn flag_has_check(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::Has,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Include '{val}' in {flag} on {bin}.");
    (meta, rule)
}

fn flag_gte_check(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::Flag(flag.into()),
        op: BinOp::Gte,
        value: val.into(),
        set: Some(true),
    });
    rule.remediation = format!("Set {flag} ≥ {val} on the {bin} process.");
    (meta, rule)
}

fn file_mode_check(id: &str, title: &str, path: &str, expected: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    let op = if expected.contains(':') {
        // owner check
        rule.items.push(TestItem {
            source: ValueSource::FileOwner(path.into()),
            op: BinOp::Eq,
            value: expected.into(),
            set: Some(true),
        });
        rule.logic = Logic::And;
        BinOp::Eq
    } else {
        rule.items.push(TestItem {
            source: ValueSource::FileMode(path.into()),
            op: BinOp::Lte,
            value: expected.into(),
            set: Some(true),
        });
        BinOp::Lte
    };
    rule.remediation = format!("chmod/chown {path} to {expected} (op {op:?}).");
    (meta, rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cis_engine::{CisContext, evaluate_rule};

    #[test]
    fn test_master_checks_count_meets_floor() {
        let m = master_checks();
        assert!(m.len() >= 20, "expected ≥20 master checks, got {}", m.len());
    }

    #[test]
    fn test_master_checks_have_unique_ids() {
        let m = master_checks();
        let mut ids: Vec<_> = m.iter().map(|(c, _)| c.id.clone()).collect();
        let n_before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n_before, "duplicate IDs in master_checks");
    }

    #[test]
    fn test_anonymous_auth_fail() {
        let (meta, rule) = flag_eq_check(
            "cis-1.2.1",
            "anonymous-auth",
            "apiserver",
            "--anonymous-auth",
            "false",
            Severity::Critical,
        );
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--anonymous-auth", "true");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, crate::Verdict::Fail);
    }

    #[test]
    fn test_authz_mode_has_rbac() {
        let (meta, rule) = flag_has_check("cis-1.2.8", "rbac", "apiserver", "--authorization-mode", "RBAC", Severity::High);
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--authorization-mode", "Node,RBAC,Webhook");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }

    #[test]
    fn test_audit_log_maxage_gte() {
        let (meta, rule) = flag_gte_check("cis-1.2.17", "maxage", "apiserver", "--audit-log-maxage", "30", Severity::Medium);
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--audit-log-maxage", "60");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }

    #[test]
    fn test_file_mode_lte() {
        let (meta, rule) = file_mode_check("cis-1.1.1", "perms", "/etc/k.yaml", "600", Severity::High);
        let mut ctx = CisContext::default();
        ctx.set_file("/etc/k.yaml", "400", "root:root");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }

    #[test]
    fn test_master_checks_all_node_type_master() {
        for (c, _) in master_checks() {
            assert_eq!(c.node_type, NodeType::Master, "{} expected NodeType::Master", c.id);
        }
    }
}
