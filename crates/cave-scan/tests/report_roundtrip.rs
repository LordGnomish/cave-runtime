// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

use cave_scan::report::{
    Finding, PackageRef, Report, Severity, cyclonedx, json as json_fmt, sarif, spdx, table,
    template,
};

fn sample_report() -> Report {
    Report {
        target: "alpine:3.20".into(),
        scanner: "image".into(),
        findings: vec![
            Finding {
                id: "CVE-2024-1234".into(),
                severity: Severity::Critical,
                title: "remote code execution".into(),
                message: "musl version 1.2.4 vulnerable; fixed in 1.2.5".into(),
                location: "lib/apk/db/installed".into(),
                cve: Some("CVE-2024-1234".into()),
                package: Some("musl".into()),
                installed_version: Some("1.2.4".into()),
                fixed_version: Some("1.2.5".into()),
                cwe: Some(94),
                references: vec!["https://nvd.nist.gov/vuln/detail/CVE-2024-1234".into()],
            },
            Finding {
                id: "AVD-K8S-0001".into(),
                severity: Severity::High,
                title: "privileged container".into(),
                message: "container runs as privileged".into(),
                location: "manifests/pod.yaml".into(),
                ..Default::default()
            },
            Finding {
                id: "SEC-0001".into(),
                severity: Severity::Medium,
                title: "aws key".into(),
                message: "secret matches AWS access key pattern".into(),
                location: ".env".into(),
                ..Default::default()
            },
        ],
        packages: vec![
            PackageRef {
                name: "musl".into(),
                version: "1.2.4".into(),
                license: Some("MIT".into()),
                purl: Some("pkg:apk/alpine/musl@1.2.4".into()),
            },
            PackageRef {
                name: "busybox".into(),
                version: "1.36.1".into(),
                license: Some("GPL-2.0-only".into()),
                purl: None,
            },
        ],
    }
}

// ── SARIF ──────────────────────────────────────────────────────────────────
#[test]
fn sarif_top_level_shape() {
    let s = sarif::to_sarif(&sample_report());
    assert_eq!(s["version"], "2.1.0");
    assert_eq!(s["runs"][0]["tool"]["driver"]["name"], "cave-scan");
    assert_eq!(s["runs"][0]["results"].as_array().unwrap().len(), 3);
}

#[test]
fn sarif_critical_maps_to_error() {
    let s = sarif::to_sarif(&sample_report());
    let lvl = &s["runs"][0]["results"][0]["level"];
    assert_eq!(lvl, "error");
}

#[test]
fn sarif_medium_maps_to_warning() {
    let s = sarif::to_sarif(&sample_report());
    let lvl = &s["runs"][0]["results"][2]["level"];
    assert_eq!(lvl, "warning");
}

#[test]
fn sarif_roundtrip_valid_json() {
    let s = sarif::to_string_pretty(&sample_report()).unwrap();
    let _: serde_json::Value = serde_json::from_str(&s).unwrap();
}

// ── CycloneDX ──────────────────────────────────────────────────────────────
#[test]
fn cyclonedx_top_level_shape() {
    let v = cyclonedx::to_cyclonedx(&sample_report());
    assert_eq!(v["bomFormat"], "CycloneDX");
    assert_eq!(v["specVersion"], "1.5");
    assert_eq!(v["components"].as_array().unwrap().len(), 2);
}

#[test]
fn cyclonedx_vulnerabilities_only_include_cve() {
    let v = cyclonedx::to_cyclonedx(&sample_report());
    assert_eq!(v["vulnerabilities"].as_array().unwrap().len(), 1);
    assert_eq!(v["vulnerabilities"][0]["id"], "CVE-2024-1234");
}

#[test]
fn cyclonedx_component_carries_license() {
    let v = cyclonedx::to_cyclonedx(&sample_report());
    let musl = &v["components"][0];
    assert_eq!(musl["name"], "musl");
    assert_eq!(musl["licenses"][0]["license"]["id"], "MIT");
}

// ── SPDX ───────────────────────────────────────────────────────────────────
#[test]
fn spdx_top_level_shape() {
    let v = spdx::to_spdx(&sample_report());
    assert_eq!(v["spdxVersion"], "SPDX-2.3");
    assert_eq!(v["packages"].as_array().unwrap().len(), 2);
}

#[test]
fn spdx_relationships_describe_each_package() {
    let v = spdx::to_spdx(&sample_report());
    assert_eq!(v["relationships"].as_array().unwrap().len(), 2);
    assert_eq!(v["relationships"][0]["relationshipType"], "DESCRIBES");
}

#[test]
fn spdx_no_license_becomes_noassertion() {
    let mut r = sample_report();
    r.packages[1].license = None;
    let v = spdx::to_spdx(&r);
    assert_eq!(v["packages"][1]["licenseConcluded"], "NOASSERTION");
}

// ── JSON ───────────────────────────────────────────────────────────────────
#[test]
fn json_roundtrip_preserves_fields() {
    let r = sample_report();
    let s = json_fmt::to_string(&r).unwrap();
    let parsed: Report = serde_json::from_str(&s).unwrap();
    assert_eq!(parsed.findings.len(), 3);
    assert_eq!(parsed.packages.len(), 2);
}

#[test]
fn json_pretty_is_indented() {
    let s = json_fmt::to_string_pretty(&sample_report()).unwrap();
    assert!(s.contains('\n'));
    assert!(s.contains("  ")); // 2-space indent from serde_json
}

// ── Table ──────────────────────────────────────────────────────────────────
#[test]
fn table_includes_target_and_count() {
    let s = table::render(&sample_report());
    assert!(s.contains("alpine:3.20"));
    assert!(s.contains("Findings: 3"));
}

#[test]
fn table_includes_severity_totals() {
    let s = table::render(&sample_report());
    assert!(s.contains("CRITICAL=1"));
    assert!(s.contains("HIGH=1"));
    assert!(s.contains("MEDIUM=1"));
}

#[test]
fn table_empty_no_findings_line() {
    let r = Report {
        target: "empty".into(),
        scanner: "fs".into(),
        ..Default::default()
    };
    let s = table::render(&r);
    assert!(s.contains("No findings."));
}

// ── Template ───────────────────────────────────────────────────────────────
#[test]
fn template_substitutes_target_and_count() {
    let s = template::render("tgt={{ target }} count={{ count }}", &sample_report());
    assert_eq!(s, "tgt=alpine:3.20 count=3");
}

#[test]
fn template_substitutes_severity_count() {
    let s = template::render(
        "crit={{ severity:CRITICAL }} high={{ severity:HIGH }}",
        &sample_report(),
    );
    assert_eq!(s, "crit=1 high=1");
}

#[test]
fn template_unknown_placeholder_passthrough() {
    let s = template::render("{{ unknown }}", &sample_report());
    assert_eq!(s, "{{ unknown }}");
}
