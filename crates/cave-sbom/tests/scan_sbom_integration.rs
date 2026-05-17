// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! Integration: ingest a CycloneDX BOM (the kind a code-scan tool such as
//! cave-scan could produce as part of its repo inventory), persist
//! components, and exercise the policy pipeline.
//!
//! Pure-state test — no live cave-scan dep (cave-scan focuses on secret/regex
//! finds, not BOM emission). The shape is what matters here.

use cave_sbom::components::{ComponentRecord, Project};
use cave_sbom::policy::{evaluate_pipeline, Operator, Policy, PolicyCondition, ViolationState};
use cave_sbom::sbom;
use chrono::Utc;
use uuid::Uuid;

const SAMPLE_BOM: &str = r#"{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "serialNumber": "urn:uuid:11111111-1111-1111-1111-111111111111",
  "metadata": { "component": {
    "type":"application", "bom-ref":"pkg:my-app",
    "name":"my-app", "version":"1.0.0"
  }},
  "components": [
    { "type":"library", "bom-ref":"pkg:npm/lodash@4.17.21",
      "name":"lodash", "version":"4.17.21",
      "purl":"pkg:npm/lodash@4.17.21",
      "licenses":[{"license":{"id":"MIT"}}] },
    { "type":"library", "bom-ref":"pkg:npm/evil@0.0.1",
      "name":"evil", "version":"0.0.1",
      "purl":"pkg:npm/evil@0.0.1",
      "licenses":[{"license":{"id":"GPL-3.0"}}] }
  ]
}"#;

#[test]
fn scan_emits_cyclonedx_persisted_into_components() {
    let r = sbom::cyclonedx::parse_json(SAMPLE_BOM.as_bytes()).unwrap();
    assert_eq!(r.components.len(), 2);
    // Promote parsed components into stored ComponentRecord under a project.
    let project = Project::new(r.project_name.clone().unwrap(), r.project_version.clone());
    let pu = project.uuid;
    let stored: Vec<ComponentRecord> = r
        .components
        .iter()
        .map(|c| {
            let mut rec = ComponentRecord::new(pu, c.name.clone(), c.version.clone());
            rec.purl = c.purl.clone();
            rec.license = c.license.clone();
            rec
        })
        .collect();
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().any(|c| c.name == "evil"));
}

#[test]
fn license_deny_policy_fires_on_gpl_component_from_bom() {
    let r = sbom::cyclonedx::parse_json(SAMPLE_BOM.as_bytes()).unwrap();
    let project = Project::new(r.project_name.clone().unwrap(), r.project_version.clone());
    let pu = project.uuid;
    let comps: Vec<ComponentRecord> = r
        .components
        .iter()
        .map(|c| {
            let mut rec = ComponentRecord::new(pu, c.name.clone(), c.version.clone());
            rec.license = c.license.clone();
            rec
        })
        .collect();
    let policy = Policy {
        uuid: Uuid::new_v4(),
        name: "no-gpl".into(),
        violation_state: ViolationState::Fail,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::LicenseDeny {
            deny: vec!["GPL-3.0".into()],
        }],
    };
    let viols = evaluate_pipeline(&[policy], &comps, &[], Utc::now());
    assert_eq!(viols.len(), 1, "expected exactly one GPL-3.0 violation");
    assert_eq!(viols[0].violation_state, ViolationState::Fail);
    assert!(viols[0].message.contains("GPL-3.0"));
}

#[test]
fn allow_list_policy_clears_mit_component() {
    let r = sbom::cyclonedx::parse_json(SAMPLE_BOM.as_bytes()).unwrap();
    let project = Project::new(r.project_name.clone().unwrap(), r.project_version.clone());
    let pu = project.uuid;
    let comps: Vec<ComponentRecord> = r
        .components
        .iter()
        .filter(|c| c.name == "lodash")
        .map(|c| {
            let mut rec = ComponentRecord::new(pu, c.name.clone(), c.version.clone());
            rec.license = c.license.clone();
            rec
        })
        .collect();
    let policy = Policy {
        uuid: Uuid::new_v4(),
        name: "mit-only".into(),
        violation_state: ViolationState::Warn,
        operator: Operator::Any,
        conditions: vec![PolicyCondition::LicenseAllow {
            allow: vec!["MIT".into()],
        }],
    };
    let viols = evaluate_pipeline(&[policy], &comps, &[], Utc::now());
    assert!(viols.is_empty(), "MIT should pass an MIT allow-list");
}

#[test]
fn format_autodetect_then_ingest_round_trip() {
    let fmt = sbom::detect_format(SAMPLE_BOM.as_bytes()).unwrap();
    assert_eq!(fmt, sbom::BomFormat::CycloneDxJson);
    let r = sbom::cyclonedx::parse_json(SAMPLE_BOM.as_bytes()).unwrap();
    assert_eq!(r.spec_version.as_deref(), Some("1.5"));
}
