// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/utils/utils.go
//! Failing tests for go-module pseudo-version normalisation.
//!
//! Go module pseudo-versions (rfc: https://go.dev/ref/mod#pseudo-versions)
//! look like `v0.0.0-20210423082822-c015be86a520`.  They sort by the
//! timestamp component — NOT by semver rules.  The base version is
//! always either 0.0.0, or a tagged major+minor with a patch suffix.
//!
//! These tests MUST FAIL until src/go_pseudo.rs is implemented.

use cave_scan_db::go_pseudo::{is_pseudo_version, normalize_pseudo_version, pseudo_version_cmp};
use cave_scan_db::{Advisory, OsAdvisoryDb, Severity, SledStore, VulnDb, Vulnerability};
use cave_scan_db::matcher::match_purl_go;

// ── is_pseudo_version ─────────────────────────────────────────────────────────

#[test]
fn pseudo_version_detected_v000() {
    // Canonical pseudo-version form.
    assert!(is_pseudo_version("v0.0.0-20210423082822-c015be86a520"));
}

#[test]
fn pseudo_version_detected_vXY0() {
    // Pre-release pseudo-version: base vX.Y.0-0.yyyymmddhhmmss-abcdefabcdef.
    assert!(is_pseudo_version("v1.2.0-0.20220101120000-deadbeefcafe"));
}

#[test]
fn pseudo_version_detected_patch_pre() {
    // Tagged pre-release: vX.Y.Z-0.yyyymmddhhmmss-abcdefabcdef.
    assert!(is_pseudo_version("v2.3.4-0.20230601000000-aabbccddeeff"));
}

#[test]
fn non_pseudo_version_regular_semver() {
    assert!(!is_pseudo_version("v1.2.3"));
    assert!(!is_pseudo_version("1.2.3"));
    assert!(!is_pseudo_version("v1.2.3-beta.1"));
}

// ── normalize_pseudo_version ──────────────────────────────────────────────────

#[test]
fn normalize_returns_timestamp_key() {
    // normalize_pseudo_version should return the yyyymmddhhmmss segment so
    // comparisons work chronologically.
    let ts = normalize_pseudo_version("v0.0.0-20210423082822-c015be86a520");
    assert_eq!(ts, "20210423082822");
}

#[test]
fn normalize_pre_release_form() {
    let ts = normalize_pseudo_version("v1.2.0-0.20220101120000-deadbeefcafe");
    assert_eq!(ts, "20220101120000");
}

#[test]
fn normalize_returns_empty_for_non_pseudo() {
    // Non-pseudo versions: return the version unchanged (passthrough).
    assert_eq!(normalize_pseudo_version("v1.2.3"), "v1.2.3");
    assert_eq!(normalize_pseudo_version("1.0.0"), "1.0.0");
}

// ── pseudo_version_cmp ────────────────────────────────────────────────────────

#[test]
fn pseudo_version_cmp_older_before_newer() {
    // Older timestamp < newer timestamp.
    let older = "v0.0.0-20210101000000-aabbccddeeff";
    let newer = "v0.0.0-20220101000000-aabbccddeeff";
    assert!(pseudo_version_cmp(older, newer) < 0);
    assert!(pseudo_version_cmp(newer, older) > 0);
}

#[test]
fn pseudo_version_cmp_same_timestamp_equal() {
    let a = "v0.0.0-20210101000000-aabbccddeeff";
    let b = "v0.0.0-20210101000000-112233445566";
    // Same timestamp → equal (commit hash is not ordered).
    assert_eq!(pseudo_version_cmp(a, b), 0);
}

#[test]
fn pseudo_version_cmp_mixed_pseudo_and_regular() {
    // A real tagged version (v1.0.0) vs a pseudo-version (v0.0.0-...):
    // by go module rules a pseudo v0.0.0 is before v1.0.0.
    let pseudo = "v0.0.0-20210101000000-aabbccddeeff";
    let real = "v1.0.0";
    // The pseudo comes chronologically before the real release.
    assert!(pseudo_version_cmp(pseudo, real) < 0);
}

// ── match_purl_go: end-to-end ─────────────────────────────────────────────────

fn put_vuln(s: &SledStore, id: &str) {
    s.put_vuln(&Vulnerability {
        id: id.into(),
        title: "t".into(),
        description: "d".into(),
        severity: Severity::High,
        cwe_ids: vec![],
        references: vec![],
        cvss_v3: None,
        published_date: None,
        last_modified_date: None,
    })
    .unwrap();
}

fn put_go_advisory(s: &SledStore, vuln_id: &str, pkg: &str, fixed: &str) {
    s.put_advisory(&Advisory {
        vulnerability_id: vuln_id.into(),
        package_name: pkg.into(),
        ecosystem: "go".into(),
        fixed_version: fixed.into(),
        affected_version: String::new(),
        severity: Severity::High,
        data_source: "ghsa".into(),
    })
    .unwrap();
}

#[test]
fn match_purl_go_pseudo_version_vulnerable() {
    // A package at pseudo-version 20210101 has a fixed_version of a real tag
    // 1.0.0.  The pseudo-version pre-dates that tag, so it IS vulnerable.
    let s = SledStore::temporary().unwrap();
    put_vuln(&s, "GHSA-go-1");
    put_go_advisory(&s, "GHSA-go-1", "github.com/foo/bar", "1.0.0");

    let purl = "pkg:golang/github.com/foo/bar@v0.0.0-20210101000000-aabbccddeeff";
    let hits = match_purl_go(&s, purl).unwrap();
    assert_eq!(hits.len(), 1, "pseudo-version before fix should match");
}

#[test]
fn match_purl_go_real_version_patched() {
    // A package at patched real version 1.0.0 should NOT be vulnerable.
    let s = SledStore::temporary().unwrap();
    put_vuln(&s, "GHSA-go-2");
    put_go_advisory(&s, "GHSA-go-2", "github.com/foo/bar", "1.0.0");

    let purl = "pkg:golang/github.com/foo/bar@v1.0.0";
    let hits = match_purl_go(&s, purl).unwrap();
    assert!(hits.is_empty(), "exact fixed version should not match");
}

#[test]
fn match_purl_go_newer_pseudo_version_patched() {
    // A pseudo-version that is chronologically AFTER a pseudo fixed_version
    // is NOT vulnerable.
    let s = SledStore::temporary().unwrap();
    put_vuln(&s, "GHSA-go-3");
    put_go_advisory(&s, "GHSA-go-3", "github.com/foo/baz", "v0.0.0-20220601000000-aabbccddeeff");

    let purl = "pkg:golang/github.com/foo/baz@v0.0.0-20230101000000-112233445566";
    let hits = match_purl_go(&s, purl).unwrap();
    assert!(hits.is_empty(), "newer pseudo-version than fixed should not match");
}

#[test]
fn match_purl_go_older_pseudo_version_than_fixed_pseudo_vulnerable() {
    let s = SledStore::temporary().unwrap();
    put_vuln(&s, "GHSA-go-4");
    put_go_advisory(&s, "GHSA-go-4", "github.com/foo/qux", "v0.0.0-20230101000000-aabbccddeeff");

    let purl = "pkg:golang/github.com/foo/qux@v0.0.0-20210101000000-112233445566";
    let hits = match_purl_go(&s, purl).unwrap();
    assert_eq!(hits.len(), 1, "older pseudo-version before pseudo fixed should match");
}
