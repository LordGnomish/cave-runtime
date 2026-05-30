// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for cave-compliance.
//!
//! These exercise the framework-control audit engine's already-implemented public
//! behaviors that map to conceptually-analogous OPA Gatekeeper v3.17.1 test symbols
//! (enforcement-exception suppression/expiry, rule/policy lookup negative paths,
//! event filtering by secondary key, freshness/staleness). They cover the
//! cave-specific compliance domain (CIS/SOC2/PCI/HIPAA control catalogues), not
//! Gatekeeper's K8s admission/mutation/watch machinery, which is out of scope.
//!
//! Targets are restricted to items reachable through `cave_compliance::{audit,
//! evidence, policy, reports}` — the only `pub mod`s declared in lib.rs. The
//! `monitor`/`engine`/`mapping`/`store` modules are NOT public and are deliberately
//! not exercised here.

use cave_compliance::audit::{filter_events, record_event};
use cave_compliance::evidence::{create_snapshot_evidence, is_fresh};
use cave_compliance::frameworks::cis_kubernetes_framework;
use cave_compliance::models::{ControlException, Finding, FindingStatus};
use cave_compliance::policy::suggested_mappings;
use cave_compliance::reports::generate_report;
use uuid::Uuid;

/// Build a single Finding against a real control id with the given status.
fn finding_for(control_id: Uuid, control_ref: &str, status: FindingStatus) -> Finding {
    Finding {
        id: Uuid::new_v4(),
        control_id,
        control_ref: control_ref.to_string(),
        status,
        target: "cluster".to_string(),
        details: "checked".to_string(),
        remediation: None,
        evidence_ids: vec![],
        checked_at: chrono::Utc::now(),
        exception_id: None,
    }
}

/// An active (un-expired) exception for `control_id` — expires_at one hour in the future.
fn active_exception(control_id: Uuid, control_ref: &str) -> ControlException {
    ControlException {
        id: Uuid::new_v4(),
        control_id,
        control_ref: control_ref.to_string(),
        reason: "risk accepted".to_string(),
        approved_by: "ciso".to_string(),
        expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
        created_at: chrono::Utc::now(),
    }
}

/// An already-expired exception for `control_id` — expires_at one hour in the past.
fn expired_exception(control_id: Uuid, control_ref: &str) -> ControlException {
    ControlException {
        id: Uuid::new_v4(),
        control_id,
        control_ref: control_ref.to_string(),
        reason: "lapsed waiver".to_string(),
        approved_by: "ciso".to_string(),
        expires_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
        created_at: chrono::Utc::now() - chrono::Duration::hours(2),
    }
}

// ---------------------------------------------------------------------------
// reports::generate_report — enforcement-exception suppression + expiry filter
// (analogue of Gatekeeper TestValidateEnforcementAction / TestOverrideEnforcementAction)
// ---------------------------------------------------------------------------

#[test]
fn test_generate_report_active_exception_excludes_failure() {
    let fw = cis_kubernetes_framework();
    let ctrl = &fw.controls[0];
    let findings = vec![finding_for(
        ctrl.id,
        &ctrl.control_id,
        FindingStatus::Fail,
    )];
    let exceptions = vec![active_exception(ctrl.id, &ctrl.control_id)];

    let report = generate_report(&fw, &findings, &exceptions);

    // The single Fail is suppressed by the active exception => failed == 0.
    assert_eq!(report.failed, 0);
    assert_eq!(report.passed, 0);
}

#[test]
fn test_generate_report_expired_exception_still_counts_failure() {
    let fw = cis_kubernetes_framework();
    let ctrl = &fw.controls[0];
    let findings = vec![finding_for(
        ctrl.id,
        &ctrl.control_id,
        FindingStatus::Fail,
    )];
    let exceptions = vec![expired_exception(ctrl.id, &ctrl.control_id)];

    let report = generate_report(&fw, &findings, &exceptions);

    // Expiry filter (`now < exp`) drops the expired exception => the Fail counts.
    assert_eq!(report.failed, 1);
    assert_eq!(report.passed, 0);
}

#[test]
fn test_generate_report_irrelevant_findings_are_excluded() {
    // A finding whose control_id is not part of the framework is filtered out
    // before counting (the `framework.controls.iter().any(...)` guard).
    let fw = cis_kubernetes_framework();
    let orphan_id = Uuid::new_v4();
    let findings = vec![finding_for(orphan_id, "NOT-IN-FW", FindingStatus::Pass)];

    let report = generate_report(&fw, &findings, &[]);

    assert_eq!(report.passed, 0);
    assert_eq!(report.findings.len(), 0);
    assert_eq!(report.total_controls, fw.controls.len());
}

// ---------------------------------------------------------------------------
// policy::suggested_mappings — known + unknown lookup
// (analogue of Gatekeeper rule/match lookup negative path Test_namesMatch / TestFilter)
// ---------------------------------------------------------------------------

#[test]
fn test_suggested_mappings_unknown_control_is_empty() {
    let result = suggested_mappings("CIS-9.9.9-DOES-NOT-EXIST");
    assert!(result.is_empty());
}

#[test]
fn test_suggested_mappings_known_controls_resolve_exact_tuples() {
    // CIS-5.2.1 -> a single kyverno mapping with the exact policy name.
    let priv_containers = suggested_mappings("CIS-5.2.1");
    assert_eq!(priv_containers.len(), 1);
    assert_eq!(priv_containers[0].0, "kyverno");
    assert_eq!(priv_containers[0].1, "disallow-privileged-containers");

    // CIS-5.1.1 -> a single opa mapping (the only opa arm).
    let rbac = suggested_mappings("CIS-5.1.1");
    assert_eq!(rbac.len(), 1);
    assert_eq!(rbac[0].0, "opa");
    assert_eq!(rbac[0].1, "rbac-required");
}

// ---------------------------------------------------------------------------
// evidence::is_fresh — staleness branch
// ---------------------------------------------------------------------------

#[test]
fn test_is_fresh_false_when_older_than_max_age() {
    let ev = create_snapshot_evidence(Uuid::new_v4(), None, serde_json::json!({}));
    // Evidence collected ~now; with max_age_hours = 0 the age (0h) is NOT < 0 => stale.
    assert!(!is_fresh(&ev, 0));
    // Sanity: the same evidence IS fresh under a positive window.
    assert!(is_fresh(&ev, 24));
}

// ---------------------------------------------------------------------------
// audit::filter_events — resource_type secondary-key predicate
// (analogue of Gatekeeper TestFilter_MatchesCase secondary-key filter)
// ---------------------------------------------------------------------------

#[test]
fn test_filter_events_by_resource_type() {
    let events = vec![
        record_event("alice", "read", "control", "1", serde_json::json!({})),
        record_event("bob", "update", "finding", "2", serde_json::json!({})),
        record_event("carol", "delete", "finding", "3", serde_json::json!({})),
    ];

    // Filter purely on resource_type => both "finding" events, neither "control".
    let findings_only = filter_events(&events, Some("finding"), None);
    assert_eq!(findings_only.len(), 2);
    assert!(findings_only.iter().all(|e| e.resource_type == "finding"));

    // Combined predicate (resource_type AND actor) narrows to exactly one.
    let bob_findings = filter_events(&events, Some("finding"), Some("bob"));
    assert_eq!(bob_findings.len(), 1);
    assert_eq!(bob_findings[0].actor, "bob");

    // No predicates => all events returned (map_or(true, ..) default).
    let all = filter_events(&events, None, None);
    assert_eq!(all.len(), 3);
}
