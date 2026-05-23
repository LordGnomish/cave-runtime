// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-dependency-track smoke tests — exercise the user-required
//! end-to-end scenarios:
//!   1. Portfolio CRUD → CycloneDX BOM upload → component listing.
//!   2. SPDX 2.3 JSON ingest with externalRefs PURL+CPE pickup.
//!   3. NVD CVE 2.0 → severity bucketing → EPSS join → inherited-risk score.
//!   4. Policy engine — license + severity + age + coordinates combined.
//!   5. Audit state machine → CycloneDX VEX export → BOV summary.

use cave_dependency_track::audit::AuditStore;
use cave_dependency_track::bov::BovDocument;
use cave_dependency_track::components::ComponentIdentity;
use cave_dependency_track::models::{
    AnalysisState, Classifier, Component, Project, Severity, VulnSource, Vulnerability,
};
use cave_dependency_track::policy::engine::{
    Policy, PolicyAggregator, PolicyCondition, PolicyOperator, Subject,
};
use cave_dependency_track::policy::{
    evaluate_age, evaluate_coordinates, evaluate_license, evaluate_vulnerability,
};
use cave_dependency_track::portfolio::PortfolioStore;
use cave_dependency_track::risk::{RiskWeights, inherited_risk};
use cave_dependency_track::sbom::{ingest, parse_cyclonedx_json, parse_spdx_json};
use cave_dependency_track::vex::VexDocument;
use cave_dependency_track::vuln_intel::{join_epss, parse_epss_csv, parse_nvd_2_0};
use chrono::{Duration, Utc};
use std::collections::HashMap;

#[test]
fn smoke_1_portfolio_bom_upload_lists_components() {
    let portfolio = PortfolioStore::new();
    let proj = portfolio
        .insert(Project::new("cave-runtime", Classifier::Application))
        .unwrap();
    let bom_json = r#"{
        "bomFormat":"CycloneDX","specVersion":"1.6","version":1,
        "components":[
          {"type":"library","name":"serde","version":"1.0.200",
           "purl":"pkg:cargo/serde@1.0.200",
           "hashes":[{"alg":"SHA-256","content":"deadbeef"}],
           "licenses":[{"license":{"id":"MIT"}}]},
          {"type":"library","name":"tokio","version":"1.40",
           "purl":"pkg:cargo/tokio@1.40",
           "licenses":[{"expression":"Apache-2.0 OR MIT"}]},
          {"type":"container","name":"runtime","version":"0.1"}
        ]}"#;
    let bom = parse_cyclonedx_json(bom_json).unwrap();
    let report = ingest(&portfolio, proj.uuid, &bom).unwrap();
    assert_eq!(report.inserted, 3);
    assert_eq!(report.skipped, 0);

    let comps = portfolio.components_for(proj.uuid);
    assert_eq!(comps.len(), 3);
    let serde_c = comps.iter().find(|c| c.name == "serde").unwrap();
    assert_eq!(serde_c.sha256.as_deref(), Some("deadbeef"));
    assert_eq!(serde_c.license.as_deref(), Some("MIT"));
    let tokio_c = comps.iter().find(|c| c.name == "tokio").unwrap();
    assert_eq!(
        tokio_c.license_expression.as_deref(),
        Some("Apache-2.0 OR MIT")
    );
    let runtime_c = comps.iter().find(|c| c.name == "runtime").unwrap();
    assert_eq!(runtime_c.classifier, Classifier::Container);

    // Re-upload is idempotent — same PURL, no new rows.
    let report2 = ingest(&portfolio, proj.uuid, &bom).unwrap();
    assert_eq!(report2.inserted, 0);
    assert!(report2.updated >= 2);
}

#[test]
fn smoke_2_spdx_2_3_json_ingest_externalrefs_pickup() {
    let doc = r#"{
        "spdxVersion":"SPDX-2.3","dataLicense":"CC0-1.0","name":"cave-bom",
        "documentNamespace":"https://cave.svc/sbom/x",
        "creationInfo":{"creators":["Tool: cave-deptrack"]},
        "packages":[{
            "SPDXID":"SPDXRef-Package-openssl","name":"openssl","versionInfo":"3.2.1",
            "licenseConcluded":"OpenSSL-3.0",
            "externalRefs":[
              {"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl",
               "referenceLocator":"pkg:generic/openssl@3.2.1"},
              {"referenceCategory":"SECURITY","referenceType":"cpe23Type",
               "referenceLocator":"cpe:2.3:a:openssl:openssl:3.2.1:*:*:*:*:*:*:*"}
            ],
            "checksums":[{"algorithm":"SHA256","checksumValue":"cafebabe"}]
        }]
    }"#;
    let d = parse_spdx_json(doc).unwrap();
    assert_eq!(d.spdx_version, "SPDX-2.3");
    assert_eq!(d.packages.len(), 1);
    let p = &d.packages[0];
    assert_eq!(p.purl.as_deref(), Some("pkg:generic/openssl@3.2.1"));
    assert!(p.cpe.as_ref().unwrap().starts_with("cpe:2.3:a:openssl"));
    assert_eq!(
        p.checksums,
        vec![("SHA256".to_string(), "cafebabe".to_string())]
    );
}

#[test]
fn smoke_3_nvd_severity_epss_join_risk_score() {
    let nvd = r#"{
        "vulnerabilities":[
          {"cve":{"id":"CVE-2026-7777","descriptions":[{"lang":"en","value":"sample"}],
                  "metrics":{"cvssMetricV31":[{"cvssData":{"baseScore":9.1}}]}}},
          {"cve":{"id":"CVE-2026-8888","descriptions":[{"lang":"en","value":"x"}],
                  "metrics":{"cvssMetricV31":[{"cvssData":{"baseScore":4.5}}]}}}
        ]}"#;
    let mut vulns: Vec<Vulnerability> = parse_nvd_2_0(nvd)
        .unwrap()
        .into_iter()
        .map(|c| c.into_vuln())
        .collect();
    assert_eq!(vulns.len(), 2);
    assert_eq!(vulns[0].severity, Severity::Critical);
    assert_eq!(vulns[1].severity, Severity::Medium);

    let csv = "cve,epss,percentile\nCVE-2026-7777,0.95,0.99\nCVE-2026-8888,0.05,0.50\n";
    let epss = parse_epss_csv(csv).unwrap();
    let joined = join_epss(&mut vulns, &epss);
    assert_eq!(joined, 2);
    assert_eq!(vulns[0].epss_score, Some(0.95));
    assert_eq!(vulns[1].epss_score, Some(0.05));

    let risk = inherited_risk(&vulns, RiskWeights::default());
    // Critical (10) + Medium (3) = 13.
    assert!((risk - 13.0).abs() < f64::EPSILON);
}

#[test]
fn smoke_4_policy_engine_license_severity_age_coordinates() {
    let project_uuid = uuid::Uuid::new_v4();
    let mut comp = Component::new(project_uuid, "old-lib");
    comp.license = Some("GPL-3.0".into());
    comp.purl = Some("pkg:npm/event-stream@3.3.6".into());
    let vulns = vec![Vulnerability {
        severity: Severity::Critical,
        ..Vulnerability::new("CVE-2018-16487", VulnSource::Nvd)
    }];

    let p = Policy {
        uuid: uuid::Uuid::new_v4(),
        name: "block-everything-bad".into(),
        aggregator: PolicyAggregator::Any,
        conditions: vec![
            PolicyCondition {
                subject: Subject::License,
                operator: PolicyOperator::Is,
                value: "GPL-3.0".into(),
            },
            PolicyCondition {
                subject: Subject::Severity,
                operator: PolicyOperator::Is,
                value: "CRITICAL".into(),
            },
            PolicyCondition {
                subject: Subject::PackageUrl,
                operator: PolicyOperator::Matches,
                value: "^pkg:npm/event-stream@".into(),
            },
            PolicyCondition {
                subject: Subject::ComponentAge,
                operator: PolicyOperator::NumericGreaterThanOrEqual,
                value: "P1Y".into(),
            },
        ],
        violation_state: "FAIL".into(),
    };

    let lic_hits = evaluate_license(p.uuid, &p.conditions, &HashMap::new(), &comp);
    assert_eq!(lic_hits.len(), 1);
    let coord_hits = evaluate_coordinates(p.uuid, &p.conditions, &comp);
    assert!(!coord_hits.is_empty());
    let vuln_hits = evaluate_vulnerability(p.uuid, &p.conditions, comp.uuid, &vulns);
    assert_eq!(vuln_hits.len(), 1);
    let age_hits = evaluate_age(
        p.uuid,
        &p.conditions,
        comp.uuid,
        Some(Utc::now() - Duration::days(400)),
        Utc::now(),
    );
    assert_eq!(age_hits.len(), 1);

    // Component identity stable across re-evaluation.
    let id = ComponentIdentity::of(&comp);
    assert!(id.cache_key().starts_with("purl:"));
}

#[test]
fn smoke_5_audit_state_machine_vex_export_bov_summary() {
    let audit = AuditStore::new();
    let project = uuid::Uuid::new_v4();
    let comp_uuid = uuid::Uuid::new_v4();

    let mut v = Vulnerability::new("CVE-2026-0001", VulnSource::Nvd);
    v.severity = Severity::High;
    v.title = Some("Reflected XSS".into());
    let vuln_uuid = v.uuid;

    // Audit walks InTriage → Exploitable → NotAffected; the final state is
    // suppressed and excluded from BOV findings but tallied as `suppressed`.
    audit.upsert(comp_uuid, vuln_uuid, AnalysisState::InTriage);
    audit
        .set_state(comp_uuid, vuln_uuid, AnalysisState::Exploitable)
        .unwrap();
    audit
        .add_comment(comp_uuid, vuln_uuid, "alice", "investigating")
        .unwrap();
    audit
        .set_state(comp_uuid, vuln_uuid, AnalysisState::NotAffected)
        .unwrap();
    let a = audit.get(comp_uuid, vuln_uuid).unwrap();
    assert_eq!(a.state, AnalysisState::NotAffected);
    assert!(a.suppressed);
    assert_eq!(a.comments.len(), 1);

    let mut vex = VexDocument::new();
    vex.push_analysis(&v, &a);
    let vex_json = vex.to_json();
    assert_eq!(vex_json["bomFormat"], "CycloneDX");
    assert_eq!(vex_json["specVersion"], "1.6");
    let vulns_arr = vex_json["vulnerabilities"].as_array().unwrap();
    assert_eq!(vulns_arr.len(), 1);
    assert_eq!(vulns_arr[0]["analysis"]["state"], "not_affected");

    let bov = BovDocument::build(project, &[(comp_uuid, vec![v])], &audit);
    assert_eq!(bov.summary.total, 0);
    assert_eq!(bov.summary.suppressed, 1);
}
