// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD: failing tests for the three missing policy evaluators:
//!   - LicenseGroupPolicyEvaluator (FSF/OSI/etc group membership)
//!   - VersionDistancePolicyEvaluator (n-versions-behind)
//!   - VulnerabilityIdPolicyEvaluator (block-specific CVE ID)

use cave_sbom::components::ComponentRecord;
use cave_sbom::models::{AffectedRange, AnalysisState, Severity, VulnIntel, VulnSource};
use cave_sbom::policy::{
    Operator, Policy, PolicyCondition, ViolationState, evaluate_pipeline,
    license_group::{LicenseGroup, license_belongs_to_group, KNOWN_GROUPS},
    version_distance::versions_behind,
    vuln_id::component_has_vuln_id,
};
use chrono::Utc;
use uuid::Uuid;

// ─── helpers ────────────────────────────────────────────────────────────────

fn mk_comp(name: &str, license: Option<&str>, version: &str) -> ComponentRecord {
    let p = Uuid::new_v4();
    let mut c = ComponentRecord::new(p, name, version);
    c.license = license.map(|s| s.into());
    c.purl = Some(format!("pkg:npm/{}@{}", name, version));
    c
}

fn mk_vuln(purl_name: &str, vuln_id: &str) -> VulnIntel {
    VulnIntel {
        id: Uuid::new_v4(),
        vuln_id: vuln_id.into(),
        source: VulnSource::Nvd,
        title: "t".into(),
        description: "d".into(),
        severity: Severity::Critical,
        cvss_v3_base: Some(9.0),
        cvss_v3_vector: None,
        cvss_v2_base: None,
        epss_score: None,
        epss_percentile: None,
        cwes: vec![],
        references: vec![],
        affected: vec![AffectedRange {
            purl_type: "npm".into(),
            namespace: None,
            name: purl_name.into(),
            vers: "*".into(),
            fixed: None,
        }],
        published: None,
        modified: None,
        state: AnalysisState::NotSet,
    }
}

// ─── 1. LicenseGroup membership ─────────────────────────────────────────────

#[test]
fn gpl3_belongs_to_copyleft_group() {
    assert!(license_belongs_to_group("GPL-3.0", LicenseGroup::Copyleft));
}

#[test]
fn apache2_belongs_to_permissive_group() {
    assert!(license_belongs_to_group("Apache-2.0", LicenseGroup::Permissive));
}

#[test]
fn mit_belongs_to_permissive_group() {
    assert!(license_belongs_to_group("MIT", LicenseGroup::Permissive));
}

#[test]
fn agpl3_belongs_to_copyleft_group() {
    assert!(license_belongs_to_group("AGPL-3.0-only", LicenseGroup::Copyleft));
}

#[test]
fn unknown_license_belongs_to_no_group() {
    assert!(!license_belongs_to_group("PROPRIETARY-X", LicenseGroup::Copyleft));
    assert!(!license_belongs_to_group("PROPRIETARY-X", LicenseGroup::Permissive));
}

#[test]
fn lgpl21_belongs_to_weakcopyleft_group() {
    assert!(license_belongs_to_group("LGPL-2.1", LicenseGroup::WeakCopyleft));
}

#[test]
fn known_groups_is_non_empty() {
    assert!(!KNOWN_GROUPS.is_empty());
}

// ─── 2. LicenseInGroup PolicyCondition fires on copyleft component ──────────

#[test]
fn policy_license_in_copyleft_group_fires_on_gpl3() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "no-copyleft".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::LicenseInGroup {
            group_name: "Copyleft".into(),
        }],
    };
    let c = mk_comp("lib", Some("GPL-3.0"), "1.0");
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert_eq!(viols.len(), 1, "GPL-3.0 component must violate Copyleft group policy");
}

#[test]
fn policy_license_in_copyleft_group_does_not_fire_on_mit() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "no-copyleft".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::LicenseInGroup {
            group_name: "Copyleft".into(),
        }],
    };
    let c = mk_comp("lib", Some("MIT"), "1.0");
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert!(viols.is_empty(), "MIT component must not violate Copyleft group policy");
}

#[test]
fn policy_license_in_group_no_license_does_not_fire() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "no-copyleft".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::LicenseInGroup {
            group_name: "Copyleft".into(),
        }],
    };
    let c = mk_comp("lib", None, "1.0");
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert!(viols.is_empty(), "component with no license must not violate group policy");
}

// ─── 3. VersionDistance evaluator ───────────────────────────────────────────

#[test]
fn versions_behind_zero_when_latest() {
    let available = vec!["1.0.0", "1.1.0", "2.0.0"];
    let behind = versions_behind("2.0.0", &available);
    assert_eq!(behind, 0, "component at latest version is 0 behind");
}

#[test]
fn versions_behind_one_when_second_latest() {
    let available = vec!["1.0.0", "1.1.0", "2.0.0"];
    let behind = versions_behind("1.1.0", &available);
    assert_eq!(behind, 1);
}

#[test]
fn versions_behind_counts_from_sorted_position() {
    let available = vec!["1.0.0", "2.0.0", "3.0.0", "4.0.0"];
    let behind = versions_behind("2.0.0", &available);
    assert_eq!(behind, 2, "2.0.0 is 2 versions behind 4.0.0");
}

#[test]
fn versions_behind_returns_max_when_version_not_in_list() {
    // If the component version isn't in the list, assume maximum lag.
    let available = vec!["1.0.0", "2.0.0"];
    let behind = versions_behind("0.5.0", &available);
    assert_eq!(behind, 2, "unknown version should be treated as maximally old");
}

// ─── 4. VersionDistance PolicyCondition fires on old component ──────────────

#[test]
fn policy_version_distance_fires_when_too_far_behind() {
    // Component at "1.0.0", latest is "3.0.0" — 2 versions behind.
    // Policy requires at most 1 version behind.
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "must-be-recent".into(),
        violation_state: ViolationState::Warn,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::VersionDistanceAtLeast {
            max_versions_behind: 1,
            available_versions: vec!["1.0.0".into(), "2.0.0".into(), "3.0.0".into()],
        }],
    };
    let c = mk_comp("pkg", Some("MIT"), "1.0.0");
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert_eq!(viols.len(), 1, "component 2 versions behind threshold=1 should violate");
}

#[test]
fn policy_version_distance_does_not_fire_when_at_latest() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "must-be-recent".into(),
        violation_state: ViolationState::Warn,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::VersionDistanceAtLeast {
            max_versions_behind: 2,
            available_versions: vec!["1.0.0".into(), "2.0.0".into(), "3.0.0".into()],
        }],
    };
    let c = mk_comp("pkg", Some("MIT"), "3.0.0");
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert!(viols.is_empty(), "component at latest should not violate");
}

// ─── 5. VulnerabilityId evaluator ───────────────────────────────────────────

#[test]
fn vuln_id_match_returns_some_on_exact_cve() {
    let c = mk_comp("openssl", Some("Apache-2.0"), "1.0.0");
    let v = mk_vuln("openssl", "CVE-2014-0160");
    assert!(
        component_has_vuln_id(&c, &[v], "CVE-2014-0160").is_some(),
        "exact CVE match should fire"
    );
}

#[test]
fn vuln_id_no_match_returns_none_for_different_id() {
    let c = mk_comp("openssl", Some("Apache-2.0"), "1.0.0");
    let v = mk_vuln("openssl", "CVE-2014-0160");
    assert!(
        component_has_vuln_id(&c, &[v], "CVE-9999-9999").is_none(),
        "different CVE should not fire"
    );
}

#[test]
fn vuln_id_no_match_returns_none_when_component_unaffected() {
    let c = mk_comp("other-lib", Some("MIT"), "1.0.0");
    let v = mk_vuln("openssl", "CVE-2014-0160");
    assert!(
        component_has_vuln_id(&c, &[v], "CVE-2014-0160").is_none(),
        "unaffected component should not fire even if vuln ID matches"
    );
}

// ─── 6. VulnerabilityId PolicyCondition in pipeline ─────────────────────────

#[test]
fn policy_vuln_id_condition_fires_on_specific_cve() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "block-heartbleed".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::VulnerabilityId {
            vuln_id: "CVE-2014-0160".into(),
        }],
    };
    let c = mk_comp("openssl", Some("Apache-2.0"), "1.0.0");
    let v = mk_vuln("openssl", "CVE-2014-0160");
    let viols = evaluate_pipeline(&[p], &[c], &[v], Utc::now());
    assert_eq!(viols.len(), 1);
    assert!(viols[0].message.contains("CVE-2014-0160"));
}

#[test]
fn policy_vuln_id_condition_does_not_fire_when_vuln_absent() {
    let p = Policy {
        uuid: Uuid::new_v4(),
        name: "block-heartbleed".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::VulnerabilityId {
            vuln_id: "CVE-2014-0160".into(),
        }],
    };
    let c = mk_comp("clean", Some("MIT"), "1.0.0");
    // No vulns at all.
    let viols = evaluate_pipeline(&[p], &[c], &[], Utc::now());
    assert!(viols.is_empty());
}
