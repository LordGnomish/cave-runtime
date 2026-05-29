// SPDX-License-Identifier: AGPL-3.0-or-later
// Characterization tests for pre-existing cave-scan-db modules.
//
// These tests assert the observable behaviour of code that already existed on
// origin/main before this uplift branch. They do NOT represent red-green TDD;
// they are honest characterisation that the code does what it claims.

use cave_scan_db::matcher::{PackageRef, version_cmp, version_satisfies};
use cave_scan_db::sources::{FeedPackage, FeedRecord, alpine, debian, ghsa, nvd, redhat, almalinux};
use cave_scan_db::{
    Advisory, IacRule, IacRuleDb, LangAdvisoryDb, OsAdvisoryDb, Severity, SledStore, VulnDb,
    Vulnerability,
};

// ── lib.rs: type model & Severity ────────────────────────────────────────────

#[test]
fn characterize_severity_ordering() {
    // Severity implements Ord — Critical > High > Medium > Low > Unknown.
    assert!(Severity::Critical > Severity::High);
    assert!(Severity::High > Severity::Medium);
    assert!(Severity::Medium > Severity::Low);
    assert!(Severity::Low > Severity::Unknown);
}

#[test]
fn characterize_severity_parse_all_variants() {
    // All six input aliases the parser is documented to accept.
    assert_eq!(Severity::parse("LOW"), Severity::Low);
    assert_eq!(Severity::parse("low"), Severity::Low);
    assert_eq!(Severity::parse("MEDIUM"), Severity::Medium);
    assert_eq!(Severity::parse("moderate"), Severity::Medium);
    assert_eq!(Severity::parse("HIGH"), Severity::High);
    assert_eq!(Severity::parse("important"), Severity::High);
    assert_eq!(Severity::parse("CRITICAL"), Severity::Critical);
    assert_eq!(Severity::parse(""), Severity::Unknown);
}

#[test]
fn characterize_vulnerability_serde_roundtrip() {
    let v = Vulnerability {
        id: "CVE-2099-0001".into(),
        title: "test vuln".into(),
        description: "desc".into(),
        severity: Severity::High,
        cwe_ids: vec!["CWE-79".into()],
        references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2099-0001".into()],
        cvss_v3: None,
        published_date: Some("2099-01-01".into()),
        last_modified_date: None,
    };
    let json = serde_json::to_string(&v).unwrap();
    let back: Vulnerability = serde_json::from_str(&json).unwrap();
    assert_eq!(back, v);
}

#[test]
fn characterize_advisory_serde_roundtrip() {
    let a = Advisory {
        vulnerability_id: "CVE-2099-0001".into(),
        package_name: "openssl".into(),
        ecosystem: "debian:12".into(),
        fixed_version: "3.0.11".into(),
        affected_version: "<3.0.11".into(),
        severity: Severity::Medium,
        data_source: "debian".into(),
    };
    let json = serde_json::to_string(&a).unwrap();
    let back: Advisory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, a);
}

#[test]
fn characterize_iac_rule_serde_roundtrip() {
    let r = IacRule {
        id: "AVD-AWS-0001".into(),
        provider: "terraform".into(),
        title: "S3 public".into(),
        description: "S3 should not be public".into(),
        severity: Severity::High,
        cis_ids: vec!["1.20".into()],
        csp_control: Some("AWS-S3-001".into()),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: IacRule = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
}

// ── storage.rs: SledStore ────────────────────────────────────────────────────

#[test]
fn characterize_sled_store_vuln_count() {
    let s = SledStore::temporary().unwrap();
    assert_eq!(s.count_vulns().unwrap(), 0);
    assert_eq!(s.count_advisories().unwrap(), 0);
    let v = Vulnerability {
        id: "CVE-CHAR-1".into(),
        title: "x".into(),
        description: "y".into(),
        severity: Severity::Low,
        cwe_ids: vec![],
        references: vec![],
        cvss_v3: None,
        published_date: None,
        last_modified_date: None,
    };
    s.put_vuln(&v).unwrap();
    assert_eq!(s.count_vulns().unwrap(), 1);
}

#[test]
fn characterize_sled_store_missing_vuln_returns_none() {
    let s = SledStore::temporary().unwrap();
    assert!(s.get_vuln("CVE-DOES-NOT-EXIST").unwrap().is_none());
}

#[test]
fn characterize_sled_store_lang_advisory() {
    let s = SledStore::temporary().unwrap();
    s.put_advisory(&Advisory {
        vulnerability_id: "GHSA-xyz".into(),
        package_name: "requests".into(),
        ecosystem: "pypi".into(),
        fixed_version: "2.31.0".into(),
        affected_version: "<2.31.0".into(),
        severity: Severity::High,
        data_source: "ghsa".into(),
    })
    .unwrap();
    let r = s.advisories_for_lang_pkg("pypi", "requests").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].vulnerability_id, "GHSA-xyz");
    // Unknown ecosystem → empty.
    let empty = s.advisories_for_lang_pkg("cargo", "requests").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn characterize_sled_store_iac_rules_for_provider() {
    let s = SledStore::temporary().unwrap();
    let r = IacRule {
        id: "AVD-K8S-0001".into(),
        provider: "kubernetes".into(),
        title: "No privileged containers".into(),
        description: "containers should not be privileged".into(),
        severity: Severity::Critical,
        cis_ids: vec!["5.2.1".into()],
        csp_control: None,
    };
    s.put_rule(&r).unwrap();
    let list = s.rules_for_provider("kubernetes").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "AVD-K8S-0001");
    let empty = s.rules_for_provider("cloudformation").unwrap();
    assert!(empty.is_empty());
}

// ── matcher.rs: version_cmp ───────────────────────────────────────────────────

#[test]
fn characterize_version_cmp_semver() {
    assert_eq!(version_cmp("1.0.0", "1.0.0"), 0);
    assert_eq!(version_cmp("2.0.0", "1.99.99"), 1);
    assert_eq!(version_cmp("1.10.0", "1.9.0"), 1); // numeric not lexical
    assert_eq!(version_cmp("0.1.0", "0.2.0"), -1);
}

#[test]
fn characterize_version_satisfies_complex_range() {
    // AND range: >=1.0.0,<2.0.0
    assert!(version_satisfies("1.5.0", ">=1.0.0,<2.0.0"));
    assert!(!version_satisfies("0.9.0", ">=1.0.0,<2.0.0"));
    assert!(!version_satisfies("2.0.0", ">=1.0.0,<2.0.0"));
    // OR
    assert!(version_satisfies("1.0.0", "<0.5.0||>=1.0.0"));
    assert!(!version_satisfies("0.7.0", "<0.5.0||>=1.0.0"));
}

#[test]
fn characterize_purl_parse_deb_nested_namespace() {
    // Debian pURLs: pkg:deb/debian/openssl@1.1.1n — the namespace+name are
    // kept together as the name component (eco=deb, name=debian/openssl).
    let r = PackageRef::parse_purl("pkg:deb/debian/openssl@1.1.1n-0+deb11u3").unwrap();
    assert_eq!(r.ecosystem, "deb");
    assert_eq!(r.name, "debian/openssl");
    assert_eq!(r.version, "1.1.1n-0+deb11u3");
}

#[test]
fn characterize_purl_parse_cargo() {
    let r = PackageRef::parse_purl("pkg:cargo/serde@1.0.150").unwrap();
    assert_eq!(r.ecosystem, "cargo");
    assert_eq!(r.name, "serde");
    assert_eq!(r.version, "1.0.150");
}

// ── sources/mod.rs: FeedRecord ────────────────────────────────────────────────

#[test]
fn characterize_feed_record_into_vuln_and_advisories() {
    let r = FeedRecord {
        id: "CVE-CHAR-9999".into(),
        title: "char test".into(),
        description: "desc".into(),
        severity: "critical".into(),
        references: vec!["https://example.com".into()],
        packages: vec![
            FeedPackage {
                ecosystem: "debian:12".into(),
                name: "bash".into(),
                fixed_version: "5.2.15".into(),
                affected_version: "<5.2.15".into(),
            },
        ],
    };
    let (v, adv) = r.into_vuln_and_advisories();
    assert_eq!(v.id, "CVE-CHAR-9999");
    assert_eq!(v.severity, Severity::Critical);
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].package_name, "bash");
    assert_eq!(adv[0].fixed_version, "5.2.15");
}

// ── sources/nvd.rs ────────────────────────────────────────────────────────────

#[test]
fn characterize_nvd_score_to_severity_buckets() {
    use cave_scan_db::sources::nvd::score_to_severity;
    use cave_scan_db::CvssV3;
    let mk = |s: f32| score_to_severity(&CvssV3 { vector: String::new(), score: s });
    assert_eq!(mk(9.8), Severity::Critical);
    assert_eq!(mk(7.5), Severity::High);
    assert_eq!(mk(5.0), Severity::Medium);
    assert_eq!(mk(2.0), Severity::Low);
    assert_eq!(mk(0.0), Severity::Unknown);
}

#[test]
fn characterize_nvd_parse_no_impact() {
    // Item without baseMetricV3 should get Unknown severity.
    let data = br#"{"CVE_Items":[{"cve":{"CVE_data_meta":{"ID":"CVE-2024-0099"},"description":{"description_data":[{"lang":"en","value":"test"}]}},"impact":null}]}"#;
    let v = nvd::parse(data).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].severity, Severity::Unknown);
    assert!(v[0].cvss_v3.is_none());
}

// ── sources/ghsa.rs ───────────────────────────────────────────────────────────

#[test]
fn characterize_ghsa_parse_no_ranges() {
    // Affected entry with no ranges → empty affected/fixed strings.
    let data = br#"{
      "id": "GHSA-test-1234",
      "summary": "test",
      "details": "details",
      "severity": [],
      "affected": [{"package":{"ecosystem":"npm","name":"foo"},"ranges":[],"database_specific":{"severity":"HIGH"}}],
      "references": []
    }"#;
    let (v, adv) = ghsa::parse(data).unwrap();
    assert_eq!(v.id, "GHSA-test-1234");
    assert_eq!(v.severity, Severity::High);
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].package_name, "foo");
    // No range events → empty affected_version.
    assert!(adv[0].affected_version.is_empty());
}

// ── sources/debian.rs ─────────────────────────────────────────────────────────

#[test]
fn characterize_debian_parse_open_unfixed() {
    // "open" status (not yet fixed) → advisory with empty fixed_version.
    let data = br#"{
      "bash": {
        "CVE-2019-0001": {
          "scope": "local",
          "description": "",
          "releases": {
            "bullseye": { "status": "open", "fixed_version": "", "urgency": "unimportant" }
          }
        }
      }
    }"#;
    let adv = debian::parse(data).unwrap();
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].package_name, "bash");
    assert_eq!(adv[0].ecosystem, "debian:bullseye");
    assert!(adv[0].fixed_version.is_empty());
}

// ── sources/redhat.rs ─────────────────────────────────────────────────────────

#[test]
fn characterize_redhat_parse_package_state_unfixed() {
    // package_state entries (unfixed) get affected_version="*".
    let data = br#"{
      "name": "CVE-2024-9999",
      "threat_severity": "Low",
      "affected_release": [],
      "package_state": [
        { "product_name": "RHEL 9", "package_name": "curl", "fix_state": "Will not fix", "cpe": "cpe:/o:redhat:enterprise_linux:9" }
      ]
    }"#;
    let adv = redhat::parse(data).unwrap();
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].package_name, "curl");
    assert_eq!(adv[0].affected_version, "*");
    assert_eq!(adv[0].severity, Severity::Low);
}

// ── sources/alpine.rs ─────────────────────────────────────────────────────────

#[test]
fn characterize_alpine_parse_multiple_cves_one_fix() {
    let data = br#"{
      "distroversion": "v3.20",
      "packages": [
        { "pkg": { "name": "curl", "secfixes": {
          "8.5.0-r0": ["CVE-2024-0001", "CVE-2024-0002"]
        }}}
      ]
    }"#;
    let adv = alpine::parse(data).unwrap();
    assert_eq!(adv.len(), 2);
    assert!(adv.iter().all(|a| a.ecosystem == "alpine:3.20"));
    assert!(adv.iter().all(|a| a.fixed_version == "8.5.0-r0"));
    let ids: Vec<&str> = adv.iter().map(|a| a.vulnerability_id.as_str()).collect();
    assert!(ids.contains(&"CVE-2024-0001"));
    assert!(ids.contains(&"CVE-2024-0002"));
}

// ── sources/almalinux.rs ──────────────────────────────────────────────────────

#[test]
fn characterize_almalinux_parse_multiple_cves_multiple_pkgs() {
    let data = br#"{
      "id": "ALSA-2024:9876",
      "severity": "Critical",
      "release": "9",
      "references": [
        { "id": "CVE-2024-0010", "type": "cve" },
        { "id": "CVE-2024-0011", "type": "cve" }
      ],
      "packages": [
        { "name": "openssl", "version": "3.0.7-27.el9" },
        { "name": "openssl-libs", "version": "3.0.7-27.el9" }
      ]
    }"#;
    let adv = almalinux::parse(data).unwrap();
    // 2 CVEs × 2 packages = 4 advisories.
    assert_eq!(adv.len(), 4);
    assert!(adv.iter().all(|a| a.severity == Severity::Critical));
    assert!(adv.iter().all(|a| a.ecosystem == "alma:9"));
}
