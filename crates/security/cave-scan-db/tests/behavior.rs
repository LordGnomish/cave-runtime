// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portable-coverage integration tests for `cave-scan-db`.
//!
//! Upstream: aquasecurity/trivy-db (https://github.com/aquasecurity/trivy-db).
//! `trivy-db` is a Go feed-build pipeline; cave ports only a small backend
//! subset. These tests target three already-implemented public Rust functions
//! that the existing suite never exercises at their own boundary:
//!
//!   * `cave_scan_db::go_version_cmp` — Go pseudo-version-aware ordering
//!     (re-exported from `matcher`), modelled on `pkg/utils/utils.go`.
//!   * `cave_scan_db::sources::FeedRecord::from_json` — public JSON feed parser
//!     (`pkg/vulnsrc/vulnsrc.go` feed-file shape).
//!   * `cave_scan_db::sources::nvd::advisories_for` — documented NVD invariant
//!     that NVD entries carry no per-package advisories
//!     (`pkg/vulnsrc/nvd/nvd.go`).
//!
//! Every expected value is derived directly from the implementation logic, not
//! from a re-statement of the upstream Go behaviour.

use cave_scan_db::go_version_cmp;
use cave_scan_db::sources::nvd::advisories_for;
use cave_scan_db::sources::{FeedPackage, FeedRecord};
use cave_scan_db::{Advisory, DbError, Severity, Vulnerability};

// ---------------------------------------------------------------------------
// go_version_cmp — public re-export (matcher::go_version_cmp)
// ---------------------------------------------------------------------------
// Impl: if either side is a pseudo-version, delegate to pseudo_version_cmp and
// clamp the i32 result to -1/0/1; otherwise fall back to version_cmp.

/// Two standard pseudo-versions are ordered purely by their 14-digit timestamp.
/// "20210101000000" < "20220101000000" as ASCII → pseudo_version_cmp returns -1.
#[test]
fn go_version_cmp_two_pseudos_ordered_by_timestamp() {
    let older = "v0.0.0-20210101000000-aabbccddeeff";
    let newer = "v0.0.0-20220101000000-aabbccddeeff";
    assert_eq!(go_version_cmp(older, newer), -1);
    assert_eq!(go_version_cmp(newer, older), 1);
    assert_eq!(go_version_cmp(older, older), 0);
}

/// Pseudo-vs-real-tag: the pseudo's `v0.0.0` base ranks below a real `v1.0.0`.
/// Here `b` is a pseudo with base v0.0.0, `a` is the real tag v1.0.0 →
/// cmp_semver("v1.0.0","v0.0.0") = 1, clamped to 1.
#[test]
fn go_version_cmp_real_tag_outranks_v0_pseudo_base() {
    let real = "v1.0.0";
    let pseudo = "v0.0.0-20210423082822-c015be86a520";
    assert_eq!(go_version_cmp(real, pseudo), 1);
    assert_eq!(go_version_cmp(pseudo, real), -1);
}

/// Two real (non-pseudo) tags fall through to version_cmp, which splits on
/// `.`/`-`/`+` and compares numerically: [2,0,0] vs [1,9,0] → 1; equal → 0.
#[test]
fn go_version_cmp_two_real_tags_use_version_cmp() {
    assert_eq!(go_version_cmp("2.0.0", "1.9.0"), 1);
    assert_eq!(go_version_cmp("1.2.3", "1.2.3"), 0);
    assert_eq!(go_version_cmp("1.2.3", "1.2.4"), -1);
}

// ---------------------------------------------------------------------------
// FeedRecord::from_json — public JSON feed parser
// ---------------------------------------------------------------------------
// Impl: thin `serde_json::from_slice` with `?`; the `From<serde_json::Error>`
// conversion produces `DbError::Serde`. `#[serde(default)]` fields are optional.

/// Happy path: a full feed record round-trips through the public parser, and the
/// decoded fields plus the derived (Vulnerability, Vec<Advisory>) are exactly
/// what the struct literals would have produced.
#[test]
fn feed_record_from_json_decodes_all_fields() {
    let bytes = br#"{
        "id": "CVE-2024-0001",
        "title": "example flaw",
        "description": "an example vulnerability",
        "severity": "HIGH",
        "references": ["https://example.test/adv"],
        "packages": [
            {
                "ecosystem": "npm",
                "name": "lodash",
                "fixed_version": "4.17.21",
                "affected_version": "<4.17.21"
            }
        ]
    }"#;

    let rec = FeedRecord::from_json(bytes).expect("valid feed JSON parses");
    assert_eq!(rec.id, "CVE-2024-0001");
    assert_eq!(rec.title, "example flaw");
    assert_eq!(rec.description, "an example vulnerability");
    assert_eq!(rec.severity, "HIGH");
    assert_eq!(rec.references, vec!["https://example.test/adv".to_string()]);
    assert_eq!(rec.packages.len(), 1);
    assert_eq!(
        rec.packages[0],
        FeedPackage {
            ecosystem: "npm".to_string(),
            name: "lodash".to_string(),
            fixed_version: "4.17.21".to_string(),
            affected_version: "<4.17.21".to_string(),
        }
    );

    // Full ingest path: severity string "HIGH" → Severity::High; one advisory.
    let (vuln, advisories) = rec.into_vuln_and_advisories();
    assert_eq!(vuln.id, "CVE-2024-0001");
    assert_eq!(vuln.severity, Severity::High);
    assert_eq!(advisories.len(), 1);
    assert_eq!(advisories[0].vulnerability_id, "CVE-2024-0001");
    assert_eq!(advisories[0].package_name, "lodash");
    assert_eq!(advisories[0].ecosystem, "npm");
    assert_eq!(advisories[0].fixed_version, "4.17.21");
    assert_eq!(advisories[0].affected_version, "<4.17.21");
    assert_eq!(advisories[0].severity, Severity::High);
    assert_eq!(advisories[0].data_source, "feed");
}

/// `#[serde(default)]` covers description/severity/references/packages, so a
/// record with only the two required fields parses, with empty defaults.
#[test]
fn feed_record_from_json_applies_serde_defaults() {
    let bytes = br#"{"id": "GHSA-xxxx", "title": "minimal"}"#;
    let rec = FeedRecord::from_json(bytes).expect("minimal feed JSON parses");
    assert_eq!(rec.id, "GHSA-xxxx");
    assert_eq!(rec.title, "minimal");
    assert_eq!(rec.description, "");
    assert_eq!(rec.severity, "");
    assert!(rec.references.is_empty());
    assert!(rec.packages.is_empty());

    // Empty severity string → Severity::parse falls to Unknown; no packages.
    let (vuln, advisories) = rec.into_vuln_and_advisories();
    assert_eq!(vuln.severity, Severity::Unknown);
    assert!(advisories.is_empty());
}

/// Error path: invalid bytes surface as `DbError::Serde`, since `from_json`
/// uses `?` over `serde_json::from_slice` and the `#[from]` conversion maps the
/// serde error onto the `Serde` variant.
#[test]
fn feed_record_from_json_invalid_bytes_yield_serde_error() {
    let err = FeedRecord::from_json(b"not json at all").expect_err("invalid JSON must error");
    assert!(
        matches!(err, DbError::Serde(_)),
        "expected DbError::Serde, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// nvd::advisories_for — documented placeholder invariant
// ---------------------------------------------------------------------------
// Impl: always returns an empty Vec — NVD carries CVE metadata only, never
// per-package advisories. Lock the contract so a refactor can't silently emit.

/// NVD entries yield zero per-package advisories regardless of the vulnerability
/// content.
#[test]
fn nvd_advisories_for_always_empty() {
    let vuln = Vulnerability {
        id: "CVE-2024-9999".to_string(),
        title: "nvd entry".to_string(),
        description: "metadata only".to_string(),
        severity: Severity::Critical,
        cwe_ids: vec!["CWE-79".to_string()],
        references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2024-9999".to_string()],
        cvss_v3: None,
        published_date: None,
        last_modified_date: None,
    };
    let advisories: Vec<Advisory> = advisories_for(&vuln);
    assert!(
        advisories.is_empty(),
        "NVD must not emit per-package advisories"
    );
}
