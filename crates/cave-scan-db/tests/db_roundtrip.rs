// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/db/db_test.go
//! Integration tests for cave-scan-db.

use cave_scan_db::matcher::{match_purl, version_cmp, version_satisfies, PackageRef};
use cave_scan_db::sources::{alpine, almalinux, debian, ghsa, nvd, redhat, FeedRecord, FeedPackage};
use cave_scan_db::{
    Advisory, IacRule, IacRuleDb, LangAdvisoryDb, OsAdvisoryDb, Severity, SledStore, VulnDb,
    Vulnerability,
};

fn mk_vuln(id: &str, sev: Severity) -> Vulnerability {
    Vulnerability {
        id: id.into(),
        title: "t".into(),
        description: "d".into(),
        severity: sev,
        cwe_ids: vec![],
        references: vec![],
        cvss_v3: None,
        published_date: None,
        last_modified_date: None,
    }
}

fn mk_adv(cve: &str, eco: &str, pkg: &str, affected: &str, fixed: &str) -> Advisory {
    Advisory {
        vulnerability_id: cve.into(),
        package_name: pkg.into(),
        ecosystem: eco.into(),
        fixed_version: fixed.into(),
        affected_version: affected.into(),
        severity: Severity::High,
        data_source: "test".into(),
    }
}

#[test]
fn test_severity_parse_roundtrip() {
    assert_eq!(Severity::parse("LOW"), Severity::Low);
    assert_eq!(Severity::parse("moderate"), Severity::Medium);
    assert_eq!(Severity::parse("Important"), Severity::High);
    assert_eq!(Severity::parse("critical"), Severity::Critical);
    assert_eq!(Severity::parse("?!"), Severity::Unknown);
    assert!(Severity::Critical > Severity::Low);
}

#[test]
fn test_store_put_get_vuln() {
    let s = SledStore::temporary().unwrap();
    let v = mk_vuln("CVE-2024-0001", Severity::High);
    s.put_vuln(&v).unwrap();
    let got = s.get_vuln("CVE-2024-0001").unwrap().unwrap();
    assert_eq!(got, v);
    assert!(s.get_vuln("CVE-1999-9999").unwrap().is_none());
    assert_eq!(s.count_vulns().unwrap(), 1);
}

#[test]
fn test_store_advisory_index() {
    let s = SledStore::temporary().unwrap();
    s.put_advisory(&mk_adv("CVE-2024-1", "debian:12", "openssl", "*", "3.0.11"))
        .unwrap();
    s.put_advisory(&mk_adv("CVE-2024-2", "debian:12", "openssl", "*", "3.0.12"))
        .unwrap();
    s.put_advisory(&mk_adv("CVE-2024-3", "debian:12", "curl", "*", "8.4.0"))
        .unwrap();
    let openssl = s.advisories_for_pkg("debian:12", "openssl").unwrap();
    assert_eq!(openssl.len(), 2);
    let curl = s.advisories_for_pkg("debian:12", "curl").unwrap();
    assert_eq!(curl.len(), 1);
    let absent = s.advisories_for_pkg("debian:12", "bash").unwrap();
    assert!(absent.is_empty());
}

#[test]
fn test_store_advisory_dedup_on_repeat_put() {
    let s = SledStore::temporary().unwrap();
    let a = mk_adv("CVE-X", "alpine:3.19", "musl", "*", "1.2.4");
    s.put_advisory(&a).unwrap();
    s.put_advisory(&a).unwrap();
    s.put_advisory(&a).unwrap();
    let r = s.advisories_for_pkg("alpine:3.19", "musl").unwrap();
    // Same key — replaced, not duplicated. Index also stays size-1.
    assert_eq!(r.len(), 1);
}

#[test]
fn test_lang_advisory_lookup() {
    let s = SledStore::temporary().unwrap();
    s.put_advisory(&mk_adv("GHSA-aaaa", "npm", "lodash", "<4.17.21", "4.17.21"))
        .unwrap();
    let r = s.advisories_for_lang_pkg("npm", "lodash").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].vulnerability_id, "GHSA-aaaa");
}

#[test]
fn test_iac_rule_storage() {
    let s = SledStore::temporary().unwrap();
    let r1 = IacRule {
        id: "AVD-AWS-0001".into(),
        provider: "terraform".into(),
        title: "S3 bucket public".into(),
        description: "S3 buckets should not allow public access".into(),
        severity: Severity::High,
        cis_ids: vec!["1.20".into()],
        csp_control: Some("AWS-S3-001".into()),
    };
    let r2 = IacRule {
        id: "AVD-KSV-001".into(),
        provider: "kubernetes".into(),
        title: "Privileged container".into(),
        description: "containers should not run privileged".into(),
        severity: Severity::Critical,
        cis_ids: vec!["5.2.1".into()],
        csp_control: None,
    };
    s.put_rule(&r1).unwrap();
    s.put_rule(&r2).unwrap();
    assert_eq!(s.get_rule("AVD-AWS-0001").unwrap().unwrap(), r1);
    let tf = s.rules_for_provider("terraform").unwrap();
    assert_eq!(tf.len(), 1);
    let k8s = s.rules_for_provider("kubernetes").unwrap();
    assert_eq!(k8s.len(), 1);
    assert!(s.rules_for_provider("dockerfile").unwrap().is_empty());
}

#[test]
fn test_version_cmp_basic() {
    assert_eq!(version_cmp("1.0.0", "1.0.0"), 0);
    assert_eq!(version_cmp("1.0.0", "1.0.1"), -1);
    assert_eq!(version_cmp("2.0.0", "1.99.99"), 1);
    assert_eq!(version_cmp("1.10.0", "1.2.0"), 1); // numeric, not lexical
    assert_eq!(version_cmp("1.0", "1.0.0"), 0); // padding
}

#[test]
fn test_version_satisfies_operators() {
    assert!(version_satisfies("1.2.3", "*"));
    assert!(version_satisfies("1.2.3", ""));
    assert!(version_satisfies("1.2.3", "<2.0.0"));
    assert!(!version_satisfies("2.0.0", "<2.0.0"));
    assert!(version_satisfies("2.0.0", "<=2.0.0"));
    assert!(version_satisfies("2.0.1", ">=2.0.0"));
    assert!(version_satisfies("2.0.0", "=2.0.0"));
    assert!(version_satisfies("2.0.0", "2.0.0"));
    assert!(version_satisfies("1.5.0", ">=1.0.0,<2.0.0"));
    assert!(!version_satisfies("2.0.0", ">=1.0.0,<2.0.0"));
    assert!(version_satisfies("1.0.0", "1.0.0 2.0.0 3.0.0"));
    assert!(!version_satisfies("1.5.0", "1.0.0 2.0.0 3.0.0"));
}

#[test]
fn test_purl_parse() {
    let r = PackageRef::parse_purl("pkg:npm/lodash@4.17.20").unwrap();
    assert_eq!(r.ecosystem, "npm");
    assert_eq!(r.name, "lodash");
    assert_eq!(r.version, "4.17.20");
    assert!(PackageRef::parse_purl("not-a-purl").is_none());
    assert!(PackageRef::parse_purl("pkg:npm/lodash").is_none()); // no version
}

#[test]
fn test_match_purl_e2e() {
    let s = SledStore::temporary().unwrap();
    s.put_advisory(&mk_adv("GHSA-1", "npm", "lodash", "<4.17.21", "4.17.21"))
        .unwrap();
    s.put_advisory(&mk_adv("GHSA-2", "npm", "lodash", "<4.17.10", "4.17.10"))
        .unwrap();
    // lodash 4.17.5 is vulnerable to both
    let hits = match_purl(&s, "pkg:npm/lodash@4.17.5").unwrap();
    assert_eq!(hits.len(), 2);
    // lodash 4.17.15 is vulnerable to GHSA-1 only (>=4.17.10, <4.17.21)
    let hits = match_purl(&s, "pkg:npm/lodash@4.17.15").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].vulnerability_id, "GHSA-1");
    // lodash 4.17.21 is patched on both
    let hits = match_purl(&s, "pkg:npm/lodash@4.17.21").unwrap();
    assert!(hits.is_empty());
}

#[test]
fn test_feed_record_decompose() {
    let r = FeedRecord {
        id: "CVE-2026-0001".into(),
        title: "broken".into(),
        description: "ouch".into(),
        severity: "High".into(),
        references: vec!["https://x".into()],
        packages: vec![
            FeedPackage {
                ecosystem: "debian:12".into(),
                name: "openssl".into(),
                fixed_version: "3.0.11".into(),
                affected_version: "<3.0.11".into(),
            },
            FeedPackage {
                ecosystem: "alpine:3.19".into(),
                name: "openssl".into(),
                fixed_version: "3.1.4".into(),
                affected_version: "<3.1.4".into(),
            },
        ],
    };
    let (v, adv) = r.into_vuln_and_advisories();
    assert_eq!(v.severity, Severity::High);
    assert_eq!(adv.len(), 2);
}

#[test]
fn test_source_alpine_parse() {
    let data = br#"{
      "distroversion": "v3.19",
      "packages": [
        { "pkg": { "name": "openssl", "secfixes": { "3.1.4-r0": ["CVE-2024-2511"] } } }
      ]
    }"#;
    let adv = alpine::parse(data).unwrap();
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].ecosystem, "alpine:3.19");
    assert_eq!(adv[0].fixed_version, "3.1.4-r0");
}

#[test]
fn test_source_debian_parse() {
    let data = br#"{
      "openssl": {
        "CVE-2024-2511": {
          "scope": "remote",
          "description": "...",
          "releases": {
            "bookworm": { "status": "resolved", "fixed_version": "3.0.11", "urgency": "medium" }
          }
        }
      }
    }"#;
    let adv = debian::parse(data).unwrap();
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].ecosystem, "debian:bookworm");
    assert_eq!(adv[0].severity, Severity::Medium);
}

#[test]
fn test_source_redhat_parse() {
    let data = br#"{
      "name": "CVE-2024-2511",
      "threat_severity": "Important",
      "affected_release": [
        { "product_name": "RHEL 8", "package": "openssl-1.1.1k-12.el8_9", "cpe": "cpe:/o:redhat:enterprise_linux:8" }
      ]
    }"#;
    let adv = redhat::parse(data).unwrap();
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].package_name, "openssl");
    assert!(adv[0].fixed_version.starts_with("1.1.1k"));
    assert_eq!(adv[0].severity, Severity::High);
}

#[test]
fn test_source_almalinux_parse() {
    let data = br#"{
      "id": "ALSA-2024:1234",
      "severity": "Important",
      "release": "9",
      "references": [
        { "id": "CVE-2024-0001", "type": "cve" },
        { "id": "RHSA-2024:1234", "type": "rhsa" }
      ],
      "packages": [ { "name": "openssl", "version": "3.0.7-12.el9_3" } ]
    }"#;
    let adv = almalinux::parse(data).unwrap();
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].ecosystem, "alma:9");
    assert_eq!(adv[0].vulnerability_id, "CVE-2024-0001");
}

#[test]
fn test_source_ghsa_parse() {
    let data = br#"{
      "id": "GHSA-jfh8-c2jp-5v3q",
      "summary": "log4shell",
      "details": "very bad",
      "severity": [{ "type": "CVSS_V3", "score": "CRITICAL" }],
      "affected": [
        {
          "package": { "ecosystem": "Maven", "name": "org.apache.logging.log4j:log4j-core" },
          "ranges": [{ "type": "ECOSYSTEM", "events": [{ "introduced": "2.0.0" }, { "fixed": "2.16.0" }] }]
        }
      ],
      "references": []
    }"#;
    let (v, adv) = ghsa::parse(data).unwrap();
    assert_eq!(v.id, "GHSA-jfh8-c2jp-5v3q");
    assert_eq!(v.severity, Severity::Critical);
    assert_eq!(adv.len(), 1);
    assert_eq!(adv[0].fixed_version, "2.16.0");
    assert_eq!(adv[0].affected_version, ">=2.0.0,<2.16.0");
}

#[test]
fn test_source_nvd_parse_with_cvss() {
    let data = br#"{
      "CVE_Items": [
        {
          "cve": {
            "CVE_data_meta": { "ID": "CVE-2024-0001" },
            "description": { "description_data": [{ "lang": "en", "value": "boom" }] },
            "references": { "reference_data": [{ "url": "https://x" }] }
          },
          "impact": {
            "baseMetricV3": {
              "cvssV3": { "vectorString": "CVSS:3.1/AV:N", "baseScore": 9.8, "baseSeverity": "CRITICAL" }
            }
          }
        }
      ]
    }"#;
    let v = nvd::parse(data).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].severity, Severity::Critical);
    assert_eq!(v[0].cvss_v3.as_ref().unwrap().score, 9.8);
}

#[test]
fn test_persistence_across_open() {
    let dir = tempfile::tempdir().unwrap();
    {
        let s = SledStore::open(dir.path()).unwrap();
        s.put_vuln(&mk_vuln("CVE-PERSIST", Severity::High)).unwrap();
        s.put_advisory(&mk_adv("CVE-PERSIST", "alpine:3.19", "musl", "*", "1.2.5"))
            .unwrap();
        drop(s);
    }
    let s = SledStore::open(dir.path()).unwrap();
    assert!(s.get_vuln("CVE-PERSIST").unwrap().is_some());
    let r = s.advisories_for_pkg("alpine:3.19", "musl").unwrap();
    assert_eq!(r.len(), 1);
}

#[test]
fn test_match_purl_unknown_ecosystem_returns_empty() {
    let s = SledStore::temporary().unwrap();
    let r = match_purl(&s, "pkg:nonsense/whatever@1.0.0").unwrap();
    assert!(r.is_empty());
}
