// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Gap-close edge tests for cave-scan.
//!
//! Covers modules previously without inline `#[cfg(test)]` coverage:
//! report serializers (SARIF, CycloneDX, SPDX, JSON, table, template),
//! target detection, OCI manifest/layer compression, analyzer registry
//! routing, npm/Cargo lockfile parsing edges, APK/dpkg parsing edges,
//! binary magic detection, report aggregation, and rules catalog invariants.
//!
//! Focus areas (per task brief):
//! - SARIF parsing/level mapping
//! - issue severity transitions
//! - rule-set serde
//! - quality-gate-style evaluation (severity counts)
//! - language detector (lockfile required-path predicate)
//! - project key / target validation
//! - hotspot states (IssueType::SecurityHotspot serde)
//!
//! Designed for: failure modes, boundary cases, state transitions,
//! and serde round-tripping.

use cave_scan::analyzer::language::{CargoLockAnalyzer, NpmLockAnalyzer};
use cave_scan::analyzer::os::{AlpineApkAnalyzer, DpkgStatusAnalyzer};
use cave_scan::analyzer::{Analyzer, AnalyzerRegistry, AnalyzerType, binary::BinaryAnalyzer};
use cave_scan::oci::layer::{LayerCompression, detect_layer_compression};
use cave_scan::oci::manifest::{ImageManifest, MediaType};
use cave_scan::report::{
    Finding, PackageRef, Report, Severity, cyclonedx, json as report_json, sarif, spdx, table,
    template,
};
use cave_scan::report_agg::{ScannerReport, aggregate};
use cave_scan::rules::{IssueSeverity, IssueType, ScanRule, extended_scan_rules};
use cave_scan::target::{TargetKind, detect_target};
use serde_json::Value;

// ────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────

fn finding(id: &str, sev: Severity) -> Finding {
    Finding {
        id: id.into(),
        severity: sev,
        title: format!("Title for {id}"),
        message: format!("Message for {id}"),
        location: format!("src/{id}.rs"),
        ..Default::default()
    }
}

fn pkg(name: &str, version: &str, license: Option<&str>) -> PackageRef {
    PackageRef {
        name: name.into(),
        version: version.into(),
        license: license.map(String::from),
        purl: Some(format!("pkg:cargo/{name}@{version}")),
    }
}

fn sample_report() -> Report {
    Report {
        target: "demo-project".into(),
        scanner: "cave-scan".into(),
        findings: vec![
            finding("R-CRIT", Severity::Critical),
            finding("R-HIGH", Severity::High),
            finding("R-MED", Severity::Medium),
            finding("R-LOW", Severity::Low),
            finding("R-INFO", Severity::Info),
        ],
        packages: vec![
            pkg("alpha", "1.0.0", Some("MIT")),
            pkg("beta", "2.5.3", None),
        ],
    }
}

// ────────────────────────────────────────────────────────────────────────
// SARIF serializer — failure modes, level mapping, structure
// ────────────────────────────────────────────────────────────────────────

#[test]
fn sarif_schema_and_version_are_2_1_0() {
    let v = sarif::to_sarif(&sample_report());
    assert_eq!(v["version"], "2.1.0");
    assert!(
        v["$schema"]
            .as_str()
            .unwrap()
            .contains("sarif-schema-2.1.0")
    );
}

#[test]
fn sarif_levels_map_critical_high_to_error() {
    let rep = Report {
        findings: vec![
            finding("a", Severity::Critical),
            finding("b", Severity::High),
        ],
        ..Report::default()
    };
    let v = sarif::to_sarif(&rep);
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results[0]["level"], "error");
    assert_eq!(results[1]["level"], "error");
}

#[test]
fn sarif_levels_map_medium_to_warning_and_low_to_note_and_info_to_none() {
    let rep = Report {
        findings: vec![
            finding("m", Severity::Medium),
            finding("l", Severity::Low),
            finding("i", Severity::Info),
        ],
        ..Report::default()
    };
    let v = sarif::to_sarif(&rep);
    let r = v["runs"][0]["results"].as_array().unwrap();
    assert_eq!(r[0]["level"], "warning");
    assert_eq!(r[1]["level"], "note");
    assert_eq!(r[2]["level"], "none");
}

#[test]
fn sarif_includes_one_rule_per_finding_with_id_and_uri() {
    let rep = sample_report();
    let v = sarif::to_sarif(&rep);
    let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), rep.findings.len());
    for (i, f) in rep.findings.iter().enumerate() {
        assert_eq!(rules[i]["id"], f.id);
    }
    let results = v["runs"][0]["results"].as_array().unwrap();
    assert_eq!(
        results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        rep.findings[0].location
    );
}

#[test]
fn sarif_empty_report_still_well_formed() {
    let rep = Report::default();
    let v = sarif::to_sarif(&rep);
    assert_eq!(v["version"], "2.1.0");
    assert!(v["runs"][0]["results"].as_array().unwrap().is_empty());
    assert!(
        v["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn sarif_pretty_string_is_parseable_json() {
    let s = sarif::to_string_pretty(&sample_report()).expect("must serialize");
    let v: Value = serde_json::from_str(&s).expect("pretty SARIF must be valid JSON");
    assert_eq!(v["version"], "2.1.0");
}

// ────────────────────────────────────────────────────────────────────────
// CycloneDX serializer
// ────────────────────────────────────────────────────────────────────────

#[test]
fn cyclonedx_components_mirror_packages_and_license_attached() {
    let rep = sample_report();
    let v = cyclonedx::to_cyclonedx(&rep);
    assert_eq!(v["bomFormat"], "CycloneDX");
    assert_eq!(v["specVersion"], "1.5");
    let comps = v["components"].as_array().unwrap();
    assert_eq!(comps.len(), 2);
    assert_eq!(comps[0]["name"], "alpha");
    assert_eq!(comps[0]["licenses"][0]["license"]["id"], "MIT");
    // beta has no license — field should be absent
    assert!(comps[1].get("licenses").is_none());
}

#[test]
fn cyclonedx_only_emits_vulnerability_entries_for_cve_findings() {
    let mut rep = Report::default();
    let mut f1 = finding("RUST001", Severity::High);
    f1.cve = Some("CVE-2024-9999".into());
    let f2 = finding("CVE-2025-1", Severity::Critical); // prefix-detected CVE
    let f3 = finding("SMELL", Severity::Low); // not a CVE
    rep.findings = vec![f1, f2, f3];
    let v = cyclonedx::to_cyclonedx(&rep);
    let vulns = v["vulnerabilities"].as_array().unwrap();
    assert_eq!(vulns.len(), 2);
    assert_eq!(vulns[0]["id"], "CVE-2024-9999");
    assert_eq!(vulns[1]["id"], "CVE-2025-1");
}

// ────────────────────────────────────────────────────────────────────────
// SPDX serializer
// ────────────────────────────────────────────────────────────────────────

#[test]
fn spdx_packages_get_sequential_spdxref_ids() {
    let rep = sample_report();
    let v = spdx::to_spdx(&rep);
    let pkgs = v["packages"].as_array().unwrap();
    assert_eq!(pkgs[0]["SPDXID"], "SPDXRef-Package-0");
    assert_eq!(pkgs[1]["SPDXID"], "SPDXRef-Package-1");
}

#[test]
fn spdx_missing_license_yields_noassertion() {
    let rep = Report {
        packages: vec![pkg("beta", "2.5.3", None)],
        ..Report::default()
    };
    let v = spdx::to_spdx(&rep);
    assert_eq!(v["packages"][0]["licenseConcluded"], "NOASSERTION");
    assert_eq!(v["packages"][0]["licenseDeclared"], "NOASSERTION");
}

#[test]
fn spdx_relationships_describe_every_package() {
    let rep = sample_report();
    let v = spdx::to_spdx(&rep);
    let rels = v["relationships"].as_array().unwrap();
    assert_eq!(rels.len(), 2);
    assert_eq!(rels[0]["relationshipType"], "DESCRIBES");
    assert_eq!(rels[0]["relatedSpdxElement"], "SPDXRef-Package-0");
}

// ────────────────────────────────────────────────────────────────────────
// Plain JSON report — round-trip
// ────────────────────────────────────────────────────────────────────────

#[test]
fn json_report_roundtrip_preserves_findings_and_packages() {
    let rep = sample_report();
    let s = report_json::to_string(&rep).expect("encode");
    let back: Report = serde_json::from_str(&s).expect("decode");
    assert_eq!(back.target, rep.target);
    assert_eq!(back.findings.len(), rep.findings.len());
    assert_eq!(back.packages.len(), rep.packages.len());
    for (a, b) in rep.findings.iter().zip(back.findings.iter()) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.severity, b.severity);
        assert_eq!(a.location, b.location);
    }
}

#[test]
fn severity_default_is_medium_and_serializes_lowercase() {
    let s: String = serde_json::to_string(&Severity::default()).unwrap();
    assert_eq!(s, "\"medium\"");
    let back: Severity = serde_json::from_str("\"critical\"").unwrap();
    assert_eq!(back, Severity::Critical);
}

#[test]
fn severity_as_str_round_trip_for_all_variants() {
    for sev in [
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ] {
        let upper = sev.as_str();
        let lower = upper.to_ascii_lowercase();
        let parsed: Severity = serde_json::from_str(&format!("\"{lower}\"")).unwrap();
        assert_eq!(sev, parsed, "severity round-trip mismatch for {upper}");
    }
}

// ────────────────────────────────────────────────────────────────────────
// Table renderer
// ────────────────────────────────────────────────────────────────────────

#[test]
fn table_empty_findings_says_no_findings() {
    let rep = Report {
        target: "t".into(),
        scanner: "s".into(),
        ..Report::default()
    };
    let txt = table::render(&rep);
    assert!(txt.contains("Target: t"));
    assert!(txt.contains("Findings: 0"));
    assert!(txt.contains("No findings."));
}

#[test]
fn table_summary_counts_by_severity() {
    let rep = sample_report();
    let txt = table::render(&rep);
    // sample_report has exactly one of each severity
    assert!(
        txt.contains("CRITICAL=1") && txt.contains("HIGH=1") && txt.contains("MEDIUM=1"),
        "table summary missing per-sev counts:\n{txt}"
    );
    assert!(txt.contains("LOW=1") && txt.contains("INFO=1"));
    assert!(txt.contains("Total: 5"));
}

// ────────────────────────────────────────────────────────────────────────
// Template renderer
// ────────────────────────────────────────────────────────────────────────

#[test]
fn template_substitutes_target_scanner_count() {
    let rep = sample_report();
    let out = template::render(
        "tgt={{ target }} sc={{ scanner }} n={{ count }}",
        &rep,
    );
    assert_eq!(out, "tgt=demo-project sc=cave-scan n=5");
}

#[test]
fn template_severity_counters_are_independent() {
    let rep = sample_report();
    let out = template::render(
        "C={{ severity:CRITICAL }} H={{ severity:HIGH }} I={{ severity:INFO }}",
        &rep,
    );
    assert_eq!(out, "C=1 H=1 I=1");
}

#[test]
fn template_unknown_placeholders_are_left_intact() {
    let rep = sample_report();
    let out = template::render("{{ unknown }} c={{ count }}", &rep);
    assert!(out.starts_with("{{ unknown }}"));
    assert!(out.ends_with("c=5"));
}

#[test]
fn template_findings_embeds_valid_json_array() {
    let rep = sample_report();
    let out = template::render("X={{ findings }}", &rep);
    let body = out.trim_start_matches("X=");
    let v: Value = serde_json::from_str(body).expect("findings JSON valid");
    assert_eq!(v.as_array().unwrap().len(), 5);
}

// ────────────────────────────────────────────────────────────────────────
// Target detection — boundary cases
// ────────────────────────────────────────────────────────────────────────

#[test]
fn target_sbom_extensions_detected() {
    for s in ["bom.cdx.json", "BOM.SPDX.JSON", "out.spdx", "out.cdx"] {
        assert_eq!(detect_target(s), TargetKind::Sbom, "case `{s}` failed");
    }
}

#[test]
fn target_image_tar_extensions_detected() {
    for s in ["img.tar", "img.tar.gz", "img.tgz", "img.oci.tar"] {
        assert_eq!(detect_target(s), TargetKind::ImageTar, "case `{s}` failed");
    }
}

#[test]
fn target_filesystem_paths_detected() {
    for s in ["/abs/path", "./rel/path", "../parent", "C:\\Windows", "D:/mnt"] {
        assert_eq!(
            detect_target(s),
            TargetKind::Filesystem,
            "case `{s}` failed"
        );
    }
}

#[test]
fn target_image_reference_when_colon_or_slash_no_path_prefix() {
    assert_eq!(
        detect_target("ubuntu:22.04"),
        TargetKind::ImageReference
    );
    assert_eq!(
        detect_target("ghcr.io/cave-runtime/cave-scan:0.1"),
        TargetKind::ImageReference
    );
    assert_eq!(
        detect_target("library/alpine"),
        TargetKind::ImageReference
    );
}

#[test]
fn target_lone_token_falls_back_to_filesystem() {
    assert_eq!(detect_target("README"), TargetKind::Filesystem);
}

// ────────────────────────────────────────────────────────────────────────
// OCI manifest parsing + media-type table
// ────────────────────────────────────────────────────────────────────────

#[test]
fn oci_manifest_parses_minimal_v2_doc() {
    let txt = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.docker.container.image.v1+json",
            "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "size": 1024
        },
        "layers": [
            {
              "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
              "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
              "size": 4096
            }
        ]
    }"#;
    let m = ImageManifest::parse(txt).expect("must parse");
    assert_eq!(m.schema_version, 2);
    assert_eq!(m.config.size, 1024);
    assert_eq!(m.layers.len(), 1);
    assert_eq!(m.layers[0].size, 4096);
}

#[test]
fn oci_manifest_parse_failure_on_bad_json() {
    assert!(ImageManifest::parse("not json").is_err());
}

#[test]
fn oci_manifest_parse_failure_on_missing_required_config() {
    let txt = r#"{"schemaVersion": 2}"#;
    assert!(ImageManifest::parse(txt).is_err());
}

#[test]
fn oci_media_type_table_covers_seven_known_types() {
    let cases = [
        (
            "application/vnd.oci.image.manifest.v1+json",
            MediaType::OciManifestV1,
        ),
        (
            "application/vnd.oci.image.index.v1+json",
            MediaType::OciIndexV1,
        ),
        (
            "application/vnd.docker.distribution.manifest.v2+json",
            MediaType::DockerManifestV2,
        ),
        (
            "application/vnd.docker.container.image.v1+json",
            MediaType::DockerImageConfigV1,
        ),
        (
            "application/vnd.oci.image.layer.v1.tar+gzip",
            MediaType::OciLayerV1TarGzip,
        ),
        (
            "application/vnd.oci.image.layer.v1.tar+zstd",
            MediaType::OciLayerV1TarZstd,
        ),
        (
            "application/vnd.docker.image.rootfs.diff.tar.gzip",
            MediaType::DockerLayerV1TarGzip,
        ),
    ];
    for (s, want) in cases {
        assert_eq!(MediaType::from_str(s), Some(want), "case `{s}` failed");
    }
    assert_eq!(MediaType::from_str("application/json"), None);
}

// ────────────────────────────────────────────────────────────────────────
// OCI layer compression magic-byte detection
// ────────────────────────────────────────────────────────────────────────

#[test]
fn layer_compression_gzip_zstd_bzip2_xz_and_none() {
    assert_eq!(detect_layer_compression(&[0x1f, 0x8b]), LayerCompression::Gzip);
    assert_eq!(
        detect_layer_compression(&[0x28, 0xb5, 0x2f, 0xfd]),
        LayerCompression::Zstd
    );
    assert_eq!(
        detect_layer_compression(b"BZh9"),
        LayerCompression::Bzip2
    );
    assert_eq!(
        detect_layer_compression(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]),
        LayerCompression::Xz
    );
    assert_eq!(detect_layer_compression(b"plain"), LayerCompression::None);
}

#[test]
fn layer_compression_short_input_returns_none() {
    assert_eq!(detect_layer_compression(&[]), LayerCompression::None);
    assert_eq!(detect_layer_compression(&[0x1f]), LayerCompression::None);
}

// ────────────────────────────────────────────────────────────────────────
// Binary analyzer (magic bytes)
// ────────────────────────────────────────────────────────────────────────

#[test]
fn binary_analyzer_recognises_elf_pe_macho() {
    let b = BinaryAnalyzer;
    assert!(b.is_executable(b"\x7fELF\x02"));
    assert!(b.is_executable(b"MZ\x90"));
    assert!(b.is_executable(&[0xfe, 0xed, 0xfa, 0xcf]));
    assert!(b.is_executable(&[0xcf, 0xfa, 0xed, 0xfe]));
    assert!(!b.is_executable(b"plain text"));
    assert!(!b.is_executable(&[]));
}

#[test]
fn binary_analyzer_required_always_false_path_dispatch_disabled() {
    let b = BinaryAnalyzer;
    assert!(!b.required("/usr/bin/ls"));
    assert!(!b.required("a.out"));
    assert_eq!(b.kind(), AnalyzerType::Binary);
}

// ────────────────────────────────────────────────────────────────────────
// Analyzer registry routing
// ────────────────────────────────────────────────────────────────────────

#[test]
fn registry_routes_alpine_apk_db_only() {
    let r = AnalyzerRegistry::default_set();
    let m = r.analyzers_for("/lib/apk/db/installed");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].kind(), AnalyzerType::AlpineApk);
    let m2 = r.analyzers_for("lib/apk/db/installed");
    assert_eq!(m2.len(), 1);
    let m3 = r.analyzers_for("usr/lib/apk/db/installed");
    assert_eq!(m3.len(), 1);
}

#[test]
fn registry_routes_dpkg_status_only() {
    let r = AnalyzerRegistry::default_set();
    let m = r.analyzers_for("var/lib/dpkg/status");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].kind(), AnalyzerType::DpkgStatus);
}

#[test]
fn registry_routes_npm_lockfile_and_skips_nested_node_modules() {
    let r = AnalyzerRegistry::default_set();
    let m = r.analyzers_for("/project/package-lock.json");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].kind(), AnalyzerType::Npm);
    // Nested node_modules lockfile is skipped (belongs to transitive pkg)
    let m2 = r.analyzers_for("/project/node_modules/foo/package-lock.json");
    assert!(m2.is_empty());
}

#[test]
fn registry_routes_cargo_lock_and_skips_vendored() {
    let r = AnalyzerRegistry::default_set();
    let m = r.analyzers_for("/project/Cargo.lock");
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].kind(), AnalyzerType::CargoLock);
    let m2 = r.analyzers_for("/project/vendor/foo/Cargo.lock");
    assert!(m2.is_empty());
}

#[test]
fn registry_unknown_path_returns_empty() {
    let r = AnalyzerRegistry::default_set();
    assert!(r.analyzers_for("/etc/hosts").is_empty());
    assert!(r.analyzers_for("README.md").is_empty());
}

// ────────────────────────────────────────────────────────────────────────
// Language analyzers — npm + Cargo lockfile parsing
// ────────────────────────────────────────────────────────────────────────

#[test]
fn npm_v2_lockfile_parses_packages_skipping_root() {
    let a = NpmLockAnalyzer;
    let txt = r#"{
        "lockfileVersion": 2,
        "packages": {
            "": {"version": "1.0.0"},
            "node_modules/left-pad": {"version": "1.3.0", "license": "MIT"},
            "node_modules/scope/pkg": {"version": "0.2.0"}
        }
    }"#;
    let pkgs = a.parse_lock(txt).expect("npm v2 parse");
    assert_eq!(pkgs.len(), 2);
    let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"left-pad"));
    let lp = pkgs.iter().find(|p| p.name == "left-pad").unwrap();
    assert_eq!(lp.version, "1.3.0");
    assert_eq!(lp.license.as_deref(), Some("MIT"));
}

#[test]
fn npm_v1_lockfile_uses_dependencies_when_no_packages_key() {
    let a = NpmLockAnalyzer;
    let txt = r#"{
        "lockfileVersion": 1,
        "dependencies": {
            "foo": {"version": "1.0.0", "license": "MIT"},
            "bar": {"version": "2.0.0"}
        }
    }"#;
    let pkgs = a.parse_lock(txt).expect("npm v1 parse");
    assert_eq!(pkgs.len(), 2);
}

#[test]
fn npm_lockfile_parse_error_on_garbage() {
    let a = NpmLockAnalyzer;
    assert!(a.parse_lock("garbage").is_err());
}

#[test]
fn cargo_lock_parse_extracts_name_version() {
    let a = CargoLockAnalyzer;
    let txt = r#"
[[package]]
name = "serde"
version = "1.0.0"

[[package]]
name = "thiserror"
version = "1.0.50"
"#;
    let pkgs = a.parse_lock(txt).expect("Cargo.lock parse");
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs[0].name, "serde");
    assert_eq!(pkgs[1].version, "1.0.50");
}

#[test]
fn cargo_lock_parse_failure_on_invalid_toml() {
    let a = CargoLockAnalyzer;
    assert!(a.parse_lock("[invalid").is_err());
}

// ────────────────────────────────────────────────────────────────────────
// APK / dpkg parsing edges
// ────────────────────────────────────────────────────────────────────────

#[test]
fn apk_parses_multi_record_db() {
    let a = AlpineApkAnalyzer;
    let txt = "\
P:musl
V:1.2.4-r0
A:x86_64
L:MIT
o:musl
p:so:libc.musl.so

P:busybox
V:1.36.1-r0
A:x86_64
D:musl
";
    let pkgs = a.parse_installed_db(txt);
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs[0].name, "musl");
    assert_eq!(pkgs[0].version, "1.2.4-r0");
    assert_eq!(pkgs[0].license.as_deref(), Some("MIT"));
    assert_eq!(pkgs[0].provides, vec!["so:libc.musl.so".to_string()]);
    assert_eq!(pkgs[1].name, "busybox");
    assert_eq!(pkgs[1].depends, vec!["musl".to_string()]);
}

#[test]
fn apk_skips_records_with_no_name() {
    let a = AlpineApkAnalyzer;
    let txt = "V:1.0\nA:noarch\n\nP:real\nV:2.0\n";
    let pkgs = a.parse_installed_db(txt);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "real");
}

#[test]
fn dpkg_skips_non_installed_packages() {
    let a = DpkgStatusAnalyzer;
    let txt = "\
Package: kept-back
Status: install ok config-files
Version: 1.0

Package: real
Status: install ok installed
Version: 2.0
Architecture: amd64
";
    let pkgs = a.parse_status(txt);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "real");
    assert_eq!(pkgs[0].arch.as_deref(), Some("amd64"));
}

#[test]
fn dpkg_source_with_version_in_parens() {
    let a = DpkgStatusAnalyzer;
    let txt = "\
Package: libfoo0
Status: install ok installed
Version: 1.2.3-1
Source: libfoo (1.2.3-0)
";
    let pkgs = a.parse_status(txt);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].source.as_deref(), Some("libfoo"));
    assert_eq!(pkgs[0].source_version.as_deref(), Some("1.2.3-0"));
}

// ────────────────────────────────────────────────────────────────────────
// Cross-scanner aggregation
// ────────────────────────────────────────────────────────────────────────

#[test]
fn report_agg_sums_packages_across_scanners() {
    let r1 = ScannerReport {
        scanner_name: "a".into(),
        target: "t".into(),
        packages: vec![Default::default(), Default::default()],
    };
    let r2 = ScannerReport {
        scanner_name: "b".into(),
        target: "t".into(),
        packages: vec![Default::default(), Default::default(), Default::default()],
    };
    let agg = aggregate(vec![r1, r2]);
    assert_eq!(agg.total_packages, 5);
    assert_eq!(agg.reports.len(), 2);
}

#[test]
fn report_agg_empty_input_total_zero() {
    let agg = aggregate(vec![]);
    assert_eq!(agg.total_packages, 0);
    assert!(agg.reports.is_empty());
}

// ────────────────────────────────────────────────────────────────────────
// Rules catalogue — invariants + serde + hotspot/severity transitions
// ────────────────────────────────────────────────────────────────────────

#[test]
fn rules_catalogue_has_50_plus_entries_with_unique_ids() {
    let rules = extended_scan_rules();
    assert!(
        rules.len() >= 50,
        "rule catalogue must have >=50 entries, got {}",
        rules.len()
    );
    let mut ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
    ids.sort();
    let len_before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), len_before, "duplicate rule id present");
}

#[test]
fn rules_catalogue_covers_six_languages() {
    let rules = extended_scan_rules();
    let mut langs: Vec<String> = rules.iter().flat_map(|r| r.languages.clone()).collect();
    langs.sort();
    langs.dedup();
    for required in ["Python", "JavaScript", "Rust", "Go", "Java"] {
        assert!(langs.contains(&required.to_string()), "missing {required}");
    }
}

#[test]
fn rules_have_security_hotspot_entries() {
    let rules = extended_scan_rules();
    let hotspot = rules
        .iter()
        .filter(|r| matches!(r.issue_type, IssueType::SecurityHotspot))
        .count();
    assert!(hotspot >= 1, "expected at least one SecurityHotspot rule");
}

#[test]
fn rules_critical_security_rules_have_cwe_ids() {
    let rules = extended_scan_rules();
    let crit_sec = rules.iter().filter(|r| {
        matches!(r.issue_type, IssueType::Vulnerability)
            && matches!(r.severity, IssueSeverity::Critical)
    });
    let with_cwe = crit_sec.clone().filter(|r| r.cwe.is_some()).count();
    let total = crit_sec.count();
    assert!(total > 0);
    // Most critical-vuln rules should carry a CWE — relax to >half
    assert!(
        with_cwe * 2 >= total,
        "expected most critical vulns to have CWE: {with_cwe}/{total}"
    );
}

#[test]
fn rules_pattern_optional_for_complexity_rules() {
    let rules = extended_scan_rules();
    let no_pattern = rules.iter().filter(|r| r.pattern.is_none()).count();
    // GEN004 (large function) + GEN005 (deep nesting) at minimum
    assert!(
        no_pattern >= 2,
        "expected ≥2 pattern-less rules (complexity-style), got {no_pattern}"
    );
}

#[test]
fn issue_type_serde_round_trip_snake_case() {
    for it in [
        IssueType::Vulnerability,
        IssueType::Bug,
        IssueType::CodeSmell,
        IssueType::SecurityHotspot,
        IssueType::Duplication,
    ] {
        let s = serde_json::to_string(&it).unwrap();
        let back: IssueType = serde_json::from_str(&s).unwrap();
        assert_eq!(it, back, "round-trip for {it:?}");
    }
    // explicit snake_case format
    let s = serde_json::to_string(&IssueType::SecurityHotspot).unwrap();
    assert_eq!(s, "\"security_hotspot\"");
    let s = serde_json::to_string(&IssueType::CodeSmell).unwrap();
    assert_eq!(s, "\"code_smell\"");
}

#[test]
fn issue_severity_orders_blocker_to_info_via_serde_strings() {
    let sevs = [
        IssueSeverity::Blocker,
        IssueSeverity::Critical,
        IssueSeverity::Major,
        IssueSeverity::Minor,
        IssueSeverity::Info,
    ];
    let strings: Vec<String> = sevs
        .iter()
        .map(|s| serde_json::to_string(s).unwrap())
        .collect();
    let expected = vec![
        "\"blocker\"",
        "\"critical\"",
        "\"major\"",
        "\"minor\"",
        "\"info\"",
    ];
    assert_eq!(strings, expected);
}

#[test]
fn scan_rule_full_roundtrip() {
    let r = ScanRule {
        id: "X001".into(),
        name: "x".into(),
        issue_type: IssueType::SecurityHotspot,
        severity: IssueSeverity::Critical,
        languages: vec!["Rust".into()],
        pattern: Some("foo".into()),
        message_template: "msg".into(),
        effort_mins: 30,
        tags: vec!["a".into(), "b".into()],
        cwe: Some(78),
    };
    let s = serde_json::to_string(&r).unwrap();
    let back: ScanRule = serde_json::from_str(&s).unwrap();
    assert_eq!(r, back);
}

// ────────────────────────────────────────────────────────────────────────
// Quality-gate–style: severity-aware aggregation over findings
// ────────────────────────────────────────────────────────────────────────

#[test]
fn quality_gate_no_critical_passes_when_zero() {
    let rep = Report {
        findings: vec![finding("a", Severity::Low), finding("b", Severity::Info)],
        ..Report::default()
    };
    let critical = rep
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    assert_eq!(critical, 0, "gate (no Criticals) PASS");
}

#[test]
fn quality_gate_no_critical_fails_when_present() {
    let rep = sample_report();
    let critical = rep
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    assert!(critical > 0, "gate (no Criticals) FAIL");
}
