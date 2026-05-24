// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CIS K8s Benchmark — control-plane policy controls (3.x).
//!
//! Upstream: kube-bench `cfg/cis-1.10/controlplane.yaml`. Apache-2.0 line-port.
//! Covers authn/authz policy + audit-policy + scheduler/controller defaults.

use crate::cis_engine::{BinOp, CisRule, TestItem, ValueSource};
use crate::models::{Check, CisLevel, Framework, NodeType, Severity};

pub fn control_plane_checks() -> Vec<(Check, CisRule)> {
    let mut out = Vec::new();

    out.push(flag_neq("cis-3.1.1", "Client certificate authentication should not be used for users",
        "apiserver", "--client-ca-file", "", Severity::Medium));
    out.push(flag_neq("cis-3.2.1", "Ensure that a minimal audit policy is created",
        "apiserver", "--audit-policy-file", "", Severity::High));
    out.push(file_exists("cis-3.2.2", "Ensure that the audit-policy file exists on disk",
        "/etc/kubernetes/audit/audit-policy.yaml", Severity::High));
    out.push(flag_has("cis-3.2.3", "Ensure audit-policy covers sensitive resources (secrets)",
        "apiserver", "--audit-policy-file", "audit-policy.yaml", Severity::Medium));
    out.push(flag_eq("cis-3.3.1", "Ensure the scheduler --bind-address is 127.0.0.1",
        "scheduler", "--bind-address", "127.0.0.1", Severity::Medium));
    out.push(flag_eq("cis-3.3.2", "Ensure the controller-manager --bind-address is 127.0.0.1",
        "controller-manager", "--bind-address", "127.0.0.1", Severity::Medium));
    out.push(flag_eq("cis-3.4.1", "Ensure --profiling is false on control-plane (apiserver)",
        "apiserver", "--profiling", "false", Severity::Low));

    out
}

fn make_meta(id: &str, title: &str, severity: Severity) -> Check {
    let mut c = Check::new(id, Framework::CisK8s, NodeType::ControlPlane, title);
    c.severity = severity;
    c.level = CisLevel::L1;
    c.tags = vec!["cis-control-plane".into()];
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

fn flag_has(id: &str, title: &str, bin: &str, flag: &str, val: &str, sev: Severity) -> (Check, CisRule) {
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

fn file_exists(id: &str, title: &str, path: &str, sev: Severity) -> (Check, CisRule) {
    let meta = make_meta(id, title, sev);
    let mut rule = CisRule::new(id, title);
    rule.items.push(TestItem {
        source: ValueSource::FileExists(path.into()),
        op: BinOp::Eq,
        value: "true".into(),
        set: Some(true),
    });
    rule.remediation = format!("Create {path} with a baseline audit policy.");
    (meta, rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cis_engine::{CisContext, evaluate_rule};

    #[test]
    fn test_control_plane_count_meets_floor() {
        assert!(control_plane_checks().len() >= 6);
    }

    #[test]
    fn test_all_node_type_control_plane() {
        for (c, _) in control_plane_checks() {
            assert_eq!(c.node_type, NodeType::ControlPlane);
        }
    }

    #[test]
    fn test_audit_policy_file_exists() {
        let mut ctx = CisContext::default();
        ctx.set_file("/etc/kubernetes/audit/audit-policy.yaml", "600", "root:root");
        let (meta, rule) = control_plane_checks().into_iter().find(|(c, _)| c.id == "cis-3.2.2").unwrap();
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, crate::Verdict::Pass);
    }
}
