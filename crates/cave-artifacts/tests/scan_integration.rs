// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts cross-crate integration tests (cave-scan)
//! Integration test — push artifact → trigger scan via cave-scan →
//! persist findings as `core::Vulnerability`.
//!
//! Wires the real `cave-scan` engine (workspace dep, not a mock) through
//! `cave_artifacts::integrations::trivy::CaveScanAdapter`. The Harbor
//! storage layer holds the blob; the adapter scans it; the ScanSink
//! aggregates the normalised findings.

use cave_artifacts::core::{Severity, VulnerabilitySource};
use cave_artifacts::harbor::storage::{compute_digest, RegistryStorage};
use cave_artifacts::integrations::trivy::{CaveScanAdapter, ScanSink, Scanner};
use bytes::Bytes;
use cave_scan::models::{FindingSeverity, RuleType, ScanRule};
use uuid::Uuid;

fn rules() -> Vec<ScanRule> {
    vec![
        ScanRule {
            id: Uuid::new_v4(),
            name: "hardcoded-password".into(),
            description: "Detect hardcoded passwords in artifact payloads".into(),
            pattern: "password".into(),
            rule_type: RuleType::Keyword,
            severity: FindingSeverity::Critical,
            enabled: true,
        },
        ScanRule {
            id: Uuid::new_v4(),
            name: "debug-flag".into(),
            description: "Detect debug=true left in shipped configs".into(),
            pattern: "debug=true".into(),
            rule_type: RuleType::Keyword,
            severity: FindingSeverity::Major,
            enabled: true,
        },
    ]
}

#[tokio::test]
async fn push_artifact_then_scan_records_normalised_vulnerabilities() {
    // 1. Push a Harbor blob carrying scannable content.
    let storage = RegistryStorage::default();
    let payload =
        Bytes::from(b"# config\nendpoint=api.example.com\npassword=hunter2\ndebug=true\n".to_vec());
    let digest = compute_digest(&payload);
    storage.store_blob(digest.clone(), payload.clone(), "library/sample").await;

    // 2. Hand the blob bytes to the cave-scan-backed adapter.
    let scanner = CaveScanAdapter::new(rules());
    let vulns = scanner.scan(&digest, &payload).unwrap();
    assert!(
        vulns.len() >= 2,
        "expected at least one finding per rule, got {}",
        vulns.len()
    );

    // 3. Drop into the shared sink — Harbor + Pulp container plugin both
    //    write into this same sink so the dashboard counts once.
    let sink = ScanSink::new();
    sink.record(&digest, vulns.clone());
    assert_eq!(sink.count(), vulns.len());
    assert!(sink.count_blocking() >= 1, "critical finding should be blocking");

    // 4. Findings round-trip through core::Vulnerability shape.
    let stored = sink.findings_for(&digest);
    assert!(stored.iter().any(|v| v.severity == Severity::Critical));
    assert!(stored.iter().all(|v| v.source == VulnerabilitySource::Native));
    assert!(stored.iter().any(|v| v.id.contains("hardcoded-password")));
}

#[tokio::test]
async fn scan_against_clean_payload_finds_nothing() {
    let storage = RegistryStorage::default();
    let payload = Bytes::from(b"clean configuration with no secrets".to_vec());
    let digest = compute_digest(&payload);
    storage.store_blob(digest.clone(), payload.clone(), "library/clean").await;

    let scanner = CaveScanAdapter::new(rules());
    let vulns = scanner.scan(&digest, &payload).unwrap();
    assert!(vulns.is_empty(), "clean payload should yield no findings, got {vulns:?}");
}

#[tokio::test]
async fn trivy_json_report_is_normalised_into_core_vulnerability() {
    use cave_artifacts::integrations::trivy::TrivyJsonScanner;

    // A real-shape Trivy report — same JSON shape Harbor's pipeline would
    // receive from an out-of-process `trivy image -f json` invocation.
    let report = br#"{
        "ArtifactName": "library/test@sha256:cafe",
        "Results": [{
            "Target": "library/test (alpine 3.18.0)",
            "Vulnerabilities": [{
                "VulnerabilityID": "CVE-2024-1111",
                "PkgName": "openssl",
                "InstalledVersion": "3.1.0",
                "FixedVersion": "3.1.4",
                "Severity": "HIGH",
                "Cvss": {"nvd": {"V3Score": 7.5}}
            }]
        }]
    }"#;
    let vulns = TrivyJsonScanner.scan("sha256:cafe", report).unwrap();
    assert_eq!(vulns.len(), 1);
    let v = &vulns[0];
    assert_eq!(v.id, "CVE-2024-1111");
    assert_eq!(v.cve.as_deref(), Some("CVE-2024-1111"));
    assert_eq!(v.severity, Severity::High);
    assert_eq!(v.cvss_v3, Some(7.5));
    assert_eq!(v.fixed_in.as_deref(), Some("3.1.4"));
    assert_eq!(v.source, VulnerabilitySource::Trivy);
    assert!(v.is_blocking(), "HIGH must be blocking");
}
