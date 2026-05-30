// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 1 — kubescape v4.0.8 "security" framework (security baseline).
//!
//! Upstream: kubescape/regolibrary frameworks/security.json — 37 controls.
//! Apache-2.0 (line-port of control IDs + titles).

use cave_bench::kubescape_security::{security_controls, evaluate_security_control, SecurityFacts};
use cave_bench::models::{Framework, Verdict};

#[test]
fn test_security_framework_has_all_37_controls() {
    // frameworks/security.json lists exactly 37 controlsIDs.
    assert_eq!(security_controls().len(), 37);
}

#[test]
fn test_security_controls_unique_ids() {
    let mut ids: Vec<_> = security_controls().into_iter().map(|c| c.check.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

#[test]
fn test_security_controls_include_baseline_ids() {
    let ids: Vec<String> = security_controls().into_iter().map(|c| c.check.id).collect();
    // A spread across the security.json catalogue.
    for want in [
        "C-0005", "C-0013", "C-0035", "C-0057", "C-0066", "C-0069", "C-0070",
        "C-0074", "C-0211", "C-0260", "C-0262", "C-0270", "C-0271", "C-0273", "C-0292",
    ] {
        assert!(ids.iter().any(|id| id == want), "security framework missing {want}");
    }
}

#[test]
fn test_security_controls_framework_tag() {
    for c in security_controls() {
        assert_eq!(c.check.framework, Framework::SecurityBaseline);
        assert!(c.check.tags.iter().any(|t| t == "security"));
    }
}

#[test]
fn test_evaluate_kubelet_anonymous_auth_fail() {
    // C-0069 — anonymous auth must be disabled.
    let c = security_controls().into_iter().find(|c| c.check.id == "C-0069").unwrap();
    let mut facts = SecurityFacts::default();
    facts.kubelet_anonymous_auth_disabled = false;
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
    facts.kubelet_anonymous_auth_disabled = true;
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Pass);
}

#[test]
fn test_evaluate_container_runtime_socket_mount_fail() {
    // C-0074 — mounting the container runtime socket is a takeover risk.
    let c = security_controls().into_iter().find(|c| c.check.id == "C-0074").unwrap();
    let mut facts = SecurityFacts::default();
    facts.container_runtime_socket_mounted = true;
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
}

#[test]
fn test_evaluate_memory_limit_set() {
    // C-0271 — memory limits must be set; reuses the NSA fact surface.
    let c = security_controls().into_iter().find(|c| c.check.id == "C-0271").unwrap();
    let mut facts = SecurityFacts::default();
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
    facts.nsa.mem_limit_set = true;
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Pass);
}

#[test]
fn test_evaluate_outdated_k8s_version() {
    // C-0273 — the cluster must run a supported Kubernetes version.
    let c = security_controls().into_iter().find(|c| c.check.id == "C-0273").unwrap();
    let mut facts = SecurityFacts::default();
    facts.k8s_version_outdated = true;
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Fail);
    facts.k8s_version_outdated = false;
    assert_eq!(evaluate_security_control(&c, &facts, "h").verdict, Verdict::Pass);
}
