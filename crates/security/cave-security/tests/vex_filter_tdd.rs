// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD cycle (2026-05-30): VEX (Vulnerability Exploitability eXchange)
//! statement filtering — faithful line-port of trivy/pkg/vex/vex.go (OpenVEX +
//! CycloneDX `Filter`) plus openvex/go-vex matching/sort algorithms
//! (pkg/vex/{vex.go,statement.go,product.go,component.go}).
//!
//! RED first: references `cave_security::trivy::vex` which does not yet exist.

use cave_security::trivy::vex::{
    OpenVexDocument, PurlMatches, Status, VexStatement, VexVulnerability,
};

fn vuln(id: &str, pkg_ref: &str) -> VexVulnerability {
    VexVulnerability {
        vulnerability_id: id.to_string(),
        pkg_ref: pkg_ref.to_string(),
    }
}

/// trivy/pkg/vex/vex_test.go::TestVEX/openvex/not-affected → vulnerability filtered out.
#[test]
fn openvex_not_affected_filtered_out() {
    let doc = OpenVexDocument {
        statements: vec![VexStatement {
            vulnerability_id: "CVE-2021-44228".into(),
            products: vec!["pkg:maven/org.apache.logging.log4j/log4j-core".into()],
            status: Status::NotAffected,
            justification: "vulnerable_code_not_in_execute_path".into(),
            timestamp: None,
        }],
        timestamp: None,
    };
    let input = vec![
        vuln(
            "CVE-2021-44228",
            "pkg:maven/org.apache.logging.log4j/log4j-core",
        ),
        vuln("CVE-2022-22965", "pkg:maven/org.springframework/spring-core"),
    ];
    let out = doc.filter(&input);
    // not_affected statement removes the matching vuln; the other remains.
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].vulnerability_id, "CVE-2022-22965");
}

/// `fixed` status also filters out (vex.go OpenVEX.Filter).
#[test]
fn openvex_fixed_filtered_out() {
    let doc = OpenVexDocument {
        statements: vec![VexStatement {
            vulnerability_id: "CVE-2021-44228".into(),
            products: vec!["pkg:maven/org.apache.logging.log4j/log4j-core".into()],
            status: Status::Fixed,
            justification: String::new(),
            timestamp: None,
        }],
        timestamp: None,
    };
    let input = vec![vuln(
        "CVE-2021-44228",
        "pkg:maven/org.apache.logging.log4j/log4j-core",
    )];
    assert!(doc.filter(&input).is_empty());
}

/// `affected` status keeps the vulnerability (vex.go OpenVEX.Filter returns true).
#[test]
fn openvex_affected_kept() {
    let doc = OpenVexDocument {
        statements: vec![VexStatement {
            vulnerability_id: "CVE-2021-44228".into(),
            products: vec!["pkg:maven/org.apache.logging.log4j/log4j-core".into()],
            status: Status::Affected,
            justification: String::new(),
            timestamp: None,
        }],
        timestamp: None,
    };
    let input = vec![vuln(
        "CVE-2021-44228",
        "pkg:maven/org.apache.logging.log4j/log4j-core",
    )];
    assert_eq!(doc.filter(&input).len(), 1);
}

/// No matching statement → vulnerability is kept (Matches returns empty).
#[test]
fn no_statement_keeps_vuln() {
    let doc = OpenVexDocument {
        statements: vec![VexStatement {
            vulnerability_id: "CVE-9999-0001".into(),
            products: vec!["pkg:maven/foo/bar".into()],
            status: Status::NotAffected,
            justification: String::new(),
            timestamp: None,
        }],
        timestamp: None,
    };
    let input = vec![vuln("CVE-2021-44228", "pkg:maven/org.apache/x")];
    assert_eq!(doc.filter(&input).len(), 1);
}

/// Latest statement wins: an earlier `not_affected` overridden by a later
/// `affected` keeps the vuln (vex.go: `stmt := stmts[len(stmts)-1]` after
/// SortStatements by timestamp).
#[test]
fn latest_statement_overrides() {
    let doc = OpenVexDocument {
        statements: vec![
            VexStatement {
                vulnerability_id: "CVE-2021-44228".into(),
                products: vec!["pkg:maven/a/b".into()],
                status: Status::NotAffected,
                justification: String::new(),
                timestamp: Some(100),
            },
            VexStatement {
                vulnerability_id: "CVE-2021-44228".into(),
                products: vec!["pkg:maven/a/b".into()],
                status: Status::Affected,
                justification: String::new(),
                timestamp: Some(200),
            },
        ],
        timestamp: Some(50),
    };
    let input = vec![vuln("CVE-2021-44228", "pkg:maven/a/b")];
    // latest (ts=200) is `affected` → kept.
    assert_eq!(doc.filter(&input).len(), 1);
}

/// PurlMatches: same type/namespace/name match; version-mismatch fails.
/// Port of openvex/go-vex pkg/vex/vex.go::PurlMatches.
#[test]
fn purl_matches_algorithm() {
    // Identical purls match.
    assert!(PurlMatches(
        "pkg:maven/org.apache/log4j-core@2.14.1",
        "pkg:maven/org.apache/log4j-core@2.14.1",
    ));
    // p1 has no version → wildcard match against any version of same name.
    assert!(PurlMatches(
        "pkg:maven/org.apache/log4j-core",
        "pkg:maven/org.apache/log4j-core@2.14.1",
    ));
    // Different versions (both set) → no match.
    assert!(!PurlMatches(
        "pkg:maven/org.apache/log4j-core@2.14.1",
        "pkg:maven/org.apache/log4j-core@2.17.0",
    ));
    // Different name → no match.
    assert!(!PurlMatches(
        "pkg:maven/org.apache/log4j-core@2.14.1",
        "pkg:maven/org.apache/log4j-api@2.14.1",
    ));
}

/// Product matching uses PurlMatches so a versionless VEX product matches a
/// versioned package ref (Component.Matches → PurlMatches path).
#[test]
fn versionless_product_matches_versioned_pkg() {
    let doc = OpenVexDocument {
        statements: vec![VexStatement {
            vulnerability_id: "CVE-2021-44228".into(),
            products: vec!["pkg:maven/org.apache.logging.log4j/log4j-core".into()],
            status: Status::NotAffected,
            justification: String::new(),
            timestamp: None,
        }],
        timestamp: None,
    };
    let input = vec![vuln(
        "CVE-2021-44228",
        "pkg:maven/org.apache.logging.log4j/log4j-core@2.14.1",
    )];
    assert!(doc.filter(&input).is_empty());
}
