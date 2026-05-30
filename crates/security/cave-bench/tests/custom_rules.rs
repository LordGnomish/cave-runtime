// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 2 — custom benchmark rule authoring.
//!
//! Upstream:
//! - kube-bench custom checks (cfg/<version>/*.yaml authored by operators,
//!   loaded via --config-dir) — `check/controls.go` NewControls.
//! - kubescape custom frameworks (`kubescape scan framework <custom.json>`) —
//!   `core/pkg/policyhandler` custom-framework load + validate.
//!
//! cave-bench lets operators author custom controls that reuse the CIS engine
//! (TestItem/BinOp/ValueSource), validate them, and register them as a
//! runnable Profile.

use cave_bench::cis_engine::{BinOp, CisContext, TestItem, ValueSource};
use cave_bench::custom::{CustomFrameworkBuilder, CustomFrameworkSpec, CustomRegistry};
use cave_bench::models::{Framework, Severity, Verdict};

fn sample_json() -> &'static str {
    r#"{
        "id": "acme-baseline",
        "name": "ACME Internal Baseline",
        "version": "1.0.0",
        "description": "Org-specific hardening overlay.",
        "controls": [
            {
                "control_id": "ACME-1",
                "name": "API server audit log path set",
                "severity": "high",
                "node_type": "master",
                "remediation": "Set --audit-log-path.",
                "rule": {
                    "items": [
                        {"source": {"Flag": "--audit-log-path"}, "op": "Eq", "value": "/var/log/audit.log", "set": true}
                    ],
                    "logic": "And",
                    "manual": false
                }
            },
            {
                "control_id": "ACME-2",
                "name": "Manual review of break-glass accounts",
                "severity": "medium",
                "node_type": "policies",
                "remediation": "Review quarterly.",
                "rule": {"items": [], "logic": "And", "manual": true}
            }
        ]
    }"#
}

#[test]
fn test_parse_custom_framework_json() {
    let spec = CustomFrameworkSpec::from_json(sample_json()).expect("parse");
    assert_eq!(spec.id, "acme-baseline");
    assert_eq!(spec.controls.len(), 2);
    assert_eq!(spec.controls[0].control_id, "ACME-1");
    assert_eq!(spec.controls[0].severity, Severity::High);
}

#[test]
fn test_validate_rejects_duplicate_ids() {
    let mut spec = CustomFrameworkSpec::from_json(sample_json()).unwrap();
    spec.controls[1].control_id = "ACME-1".into();
    assert!(spec.validate().is_err());
}

#[test]
fn test_validate_rejects_empty_controls() {
    let spec = CustomFrameworkBuilder::new("empty", "Empty").build();
    assert!(spec.validate().is_err());
}

#[test]
fn test_validate_rejects_nonmanual_without_items() {
    let mut spec = CustomFrameworkSpec::from_json(sample_json()).unwrap();
    // ACME-1 is non-manual; strip its items → invalid.
    spec.controls[0].rule.items.clear();
    assert!(spec.validate().is_err());
}

#[test]
fn test_valid_spec_passes_validation() {
    let spec = CustomFrameworkSpec::from_json(sample_json()).unwrap();
    assert!(spec.validate().is_ok());
}

#[test]
fn test_builder_fluent_authoring() {
    let mut item = TestItem {
        source: ValueSource::Flag("--anonymous-auth".into()),
        op: BinOp::Eq,
        value: "false".into(),
        set: Some(true),
    };
    let spec = CustomFrameworkBuilder::new("org", "Org Baseline")
        .version("2.1")
        .control("ORG-1", "Anon auth off", Severity::Critical, "master", "Disable anon auth.", {
            let mut r = cave_bench::custom::CustomRule::default();
            r.items.push(std::mem::take(&mut item));
            r
        })
        .build();
    assert!(spec.validate().is_ok());
    assert_eq!(spec.controls.len(), 1);
    assert_eq!(spec.version, "2.1");
}

#[test]
fn test_into_profile_uses_custom_framework() {
    let spec = CustomFrameworkSpec::from_json(sample_json()).unwrap();
    let profile = spec.to_profile();
    assert_eq!(profile.framework, Framework::Custom);
    assert_eq!(profile.id, "acme-baseline");
    assert_eq!(profile.check_ids, vec!["ACME-1".to_string(), "ACME-2".to_string()]);
}

#[test]
fn test_run_custom_framework_evaluates_via_cis_engine() {
    let spec = CustomFrameworkSpec::from_json(sample_json()).unwrap();
    let mut ctx = CisContext::default();
    ctx.set_flag("master", "--audit-log-path", "/var/log/audit.log");
    let findings = spec.evaluate(&ctx, "n1");
    assert_eq!(findings.len(), 2);
    // ACME-1 passes (flag matches), ACME-2 is manual → Warn.
    let acme1 = findings.iter().find(|f| f.check_id == "ACME-1").unwrap();
    assert_eq!(acme1.verdict, Verdict::Pass);
    let acme2 = findings.iter().find(|f| f.check_id == "ACME-2").unwrap();
    assert_eq!(acme2.verdict, Verdict::Warn);
}

#[test]
fn test_registry_register_and_lookup() {
    let reg = CustomRegistry::default();
    let spec = CustomFrameworkSpec::from_json(sample_json()).unwrap();
    reg.register(spec).unwrap();
    assert_eq!(reg.count(), 1);
    assert!(reg.get("acme-baseline").is_some());
    assert!(reg.get("nope").is_none());
    let ids = reg.list_ids();
    assert_eq!(ids, vec!["acme-baseline".to_string()]);
}

#[test]
fn test_registry_rejects_invalid_spec() {
    let reg = CustomRegistry::default();
    let bad = CustomFrameworkBuilder::new("bad", "Bad").build(); // no controls
    assert!(reg.register(bad).is_err());
}
