// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! End-to-end smoke for cave-trivy: scan a synthetic alpine image, emit
//! reports in every format, ingest the resulting SBOM and verify vuln
//! correlation, apply VEX suppression + severity filter, and exercise
//! the K8s + SBOM + secret + IaC scanners.

use cave_trivy::engine::{Engine, Renderer};
use cave_trivy::filter::Filter;
use cave_trivy::ignore::IgnorePolicy;
use cave_trivy::misconf::MisconfRegistry;
use cave_trivy::models::{Package, Report};
use cave_trivy::scan_fs::FsTree;
use cave_trivy::scan_image::{scan_image, ImageArtifact, ScanImageOpts};
use cave_trivy::scan_k8s::{scan_cluster, K8sClusterSnapshot, K8sResource};
use cave_trivy::scan_sbom::scan_sbom;
use cave_trivy::scan_secret::{scan_secrets_in_tree, SecretRules};
use cave_trivy::sbom_cyclonedx::emit_from_packages;
use cave_trivy::sbom_spdx::emit as spdx_emit;
use cave_trivy::severity::Severity;
use cave_trivy::vex::{VexDocument, VexStatement, VexStatus};
use cave_trivy::vulndb::VulnDb;

fn fixture_image() -> ImageArtifact {
    ImageArtifact {
        name: "alpine:3.19".into(),
        digest: "sha256:deadbeef".into(),
        os_release: Some(
            "ID=alpine\nNAME=\"Alpine Linux\"\nVERSION_ID=3.19.1\n".into(),
        ),
        apk_db: Some("P:openssl\nV:3.0.0\n\nP:musl\nV:1.2.4\n".into()),
        lockfiles: vec![(
            "/app/package-lock.json".into(),
            r#"{"dependencies":{"lodash":{"version":"4.17.20"},"express":{"version":"4.10.0"}}}"#
                .into(),
        )],
        ..Default::default()
    }
}

#[test]
fn smoke_image_scan_finds_known_cves() {
    let r = scan_image(&fixture_image(), &VulnDb::cave_default(), ScanImageOpts::default()).unwrap();
    let ids: Vec<&str> = r
        .results
        .iter()
        .flat_map(|s| s.vulnerabilities.iter().map(|v| v.id.as_str()))
        .collect();
    assert!(ids.contains(&"CVE-2026-0001")); // openssl
    assert!(ids.contains(&"CVE-2026-0002")); // musl
    assert!(ids.contains(&"CVE-2026-0010")); // lodash
    assert!(ids.contains(&"CVE-2026-0011")); // express
}

#[test]
fn smoke_engine_renders_all_formats() {
    let e = Engine::default();
    let mut r = e
        .scan_image(&fixture_image(), ScanImageOpts::default())
        .unwrap();
    let f = Filter::default().min_severity(Severity::Low);

    for renderer in [Renderer::Json, Renderer::Table, Renderer::Sarif] {
        let s = e.filter_and_render(&mut r.clone(), &f, renderer).unwrap();
        assert!(!s.is_empty());
    }
}

#[test]
fn smoke_vex_suppresses_known() {
    let mut e = Engine::default();
    let doc = VexDocument {
        context: "https://openvex.dev/ns/v0.2.0".into(),
        statements: vec![VexStatement {
            vulnerability: "CVE-2026-0001".into(),
            products: vec!["pkg:oci/alpine:3.19".into()],
            status: VexStatus::NotAffected,
            justification: Some("vulnerable_code_not_in_execute_path".into()),
        }],
    };
    e = e.with_vex_document(&doc);
    let r = e
        .scan_image(&fixture_image(), ScanImageOpts::default())
        .unwrap();
    let has_1 = r
        .results
        .iter()
        .any(|s| s.vulnerabilities.iter().any(|v| v.id == "CVE-2026-0001"));
    assert!(!has_1, "VEX NotAffected statement must suppress CVE-2026-0001");
}

#[test]
fn smoke_sbom_round_trip() {
    let pkgs = vec![
        Package::new("openssl-sys", "0.9.0", "cargo"),
        Package::new("lodash", "4.17.20", "npm"),
    ];
    let cdx = emit_from_packages("cave/runtime:0.1.0", &pkgs).unwrap();
    let r = scan_sbom("cave/runtime:0.1.0", &cdx, &VulnDb::cave_default()).unwrap();
    let ids: Vec<&str> = r
        .results
        .iter()
        .flat_map(|s| s.vulnerabilities.iter().map(|v| v.id.as_str()))
        .collect();
    assert!(ids.contains(&"CVE-2026-0030"));
    assert!(ids.contains(&"CVE-2026-0010"));

    let spdx = spdx_emit("cave/runtime:0.1.0", &pkgs).unwrap();
    let r2 = scan_sbom("cave/runtime:0.1.0", &spdx, &VulnDb::cave_default()).unwrap();
    let ids2: Vec<&str> = r2
        .results
        .iter()
        .flat_map(|s| s.vulnerabilities.iter().map(|v| v.id.as_str()))
        .collect();
    assert!(ids2.contains(&"CVE-2026-0030"));
}

#[test]
fn smoke_k8s_cluster_scan() {
    let snap = K8sClusterSnapshot {
        context: "kind-cave".into(),
        resources: vec![
            K8sResource {
                kind: "Pod".into(),
                namespace: "default".into(),
                name: "bad".into(),
                manifest_yaml: "    privileged: true\n".into(),
            },
            K8sResource {
                kind: "Deployment".into(),
                namespace: "default".into(),
                name: "leaky".into(),
                manifest_yaml: "    hostNetwork: true\n".into(),
            },
        ],
    };
    let r = scan_cluster(&snap, &MisconfRegistry::builtin()).unwrap();
    let ids: Vec<&str> = r
        .results
        .iter()
        .flat_map(|s| s.misconfigurations.iter().map(|m| m.id.as_str()))
        .collect();
    assert!(ids.contains(&"AVD-KSV-0017"));
    assert!(ids.contains(&"AVD-KSV-0044"));
}

#[test]
fn smoke_secret_scan_repo() {
    let tree = FsTree::default()
        .push("README.md", "# normal")
        .push(".env", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nghp_abcdefghijklmnopqrstuvwxyz0123456789\n");
    let secrets = scan_secrets_in_tree(&tree, &SecretRules::default_rules());
    let cats: Vec<&str> = secrets.iter().map(|s| s.category.as_str()).collect();
    assert!(cats.contains(&"aws"));
    assert!(cats.contains(&"github"));
}

#[test]
fn smoke_filter_and_ignore_compose() {
    let mut r = scan_image(
        &fixture_image(),
        &VulnDb::cave_default(),
        ScanImageOpts::default(),
    )
    .unwrap();
    let mut ig = IgnorePolicy::new();
    ig.add("CVE-2026-0010");
    let f = Filter::default()
        .min_severity(Severity::Medium)
        .with_ignore(ig);
    let _suppressed = f.apply(&mut r);
    let has_lodash = r
        .results
        .iter()
        .any(|s| s.vulnerabilities.iter().any(|v| v.id == "CVE-2026-0010"));
    assert!(!has_lodash);
}

#[test]
fn smoke_report_total_helpers() {
    let r: Report = scan_image(
        &fixture_image(),
        &VulnDb::cave_default(),
        ScanImageOpts::default(),
    )
    .unwrap();
    assert!(r.total_vulns() >= 4);
    assert!(r.total_secrets() == 0);
    assert!(r.total_misconfigs() == 0);
}
