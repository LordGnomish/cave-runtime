// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Edge-case integration coverage to close gaps left by the in-module
//! unit tests. The pattern matches the workspace test_gap_close_edges
//! convention introduced in the 2026-05-19 sprint.

use cave_trivy::cache::ScanCache;
use cave_trivy::engine::{Engine, Renderer};
use cave_trivy::filter::Filter;
use cave_trivy::ignore::IgnorePolicy;
use cave_trivy::k8s_operator::{ConfigAuditReport, VulnerabilityReport};
use cave_trivy::misconf::MisconfRegistry;
use cave_trivy::models::{LicenseCategory, OsFamily, Package, Report, ScanResult, Vulnerability};
use cave_trivy::osv::OsvAdvisory;
use cave_trivy::pkg_lang::{
    parse_cargo_lock, parse_composer_lock, parse_gemfile_lock, parse_go_mod, parse_go_sum,
    parse_lockfile, parse_package_lock, parse_pipfile_lock, parse_pnpm_lock, parse_pom_xml,
    parse_pubspec_lock, parse_requirements_txt, parse_yarn_lock,
};
use cave_trivy::pkg_os::{parse_apk_installed, parse_dpkg_status, parse_rpm_textdump, OsRelease};
use cave_trivy::purl::{ecosystem_to_purl_type, PackageUrl};
use cave_trivy::report_table::write as table_write;
use cave_trivy::report_template::render as tpl_render;
use cave_trivy::sbom_cyclonedx::emit_from_packages;
use cave_trivy::sbom_spdx::emit as spdx_emit;
use cave_trivy::scan_fs::FsTree;
use cave_trivy::scan_iac::{detect_kind, scan_iac_tree, validate_hcl};
use cave_trivy::scan_image::{scan_image, ImageArtifact, ScanImageOpts};
use cave_trivy::scan_k8s::{filter_by_kinds, scan_cluster, K8sClusterSnapshot, K8sResource};
use cave_trivy::scan_license::{classify, detect_in_text, is_license_file, scan_licenses};
use cave_trivy::scan_sbom::{detect_format, ecosystem_from_purl, scan_sbom, SbomFormat};
use cave_trivy::scan_secret::{scan_secrets_in_tree, SecretRules};
use cave_trivy::server::{handle, ReportFormat, ScanRequest, ScanTarget};
use cave_trivy::severity::{any_at_least, parse_csv, Severity};
use cave_trivy::store::ScanStore;
use cave_trivy::vex::{apply, VexDocument, VexIndex, VexStatement, VexStatus};
use cave_trivy::vulndb::{compare_versions, version_in_range, VulnDb};

// ── Severity edges ──────────────────────────────────────────────────────────

#[test]
fn severity_csv_parse_lowercase() {
    assert!(parse_csv("low").is_ok());
}

#[test]
fn severity_csv_parse_blanks_compacted() {
    let s = parse_csv(",,LOW,").unwrap();
    assert_eq!(s.len(), 1);
}

#[test]
fn severity_any_at_least_empty() {
    let v: Vec<Severity> = vec![];
    assert!(!any_at_least(&v, Severity::Low));
}

#[test]
fn severity_serde_unknown_variant_rejects() {
    let r: Result<Severity, _> = serde_json::from_str("\"bogus\"");
    assert!(r.is_err());
}

// ── purl edges ──────────────────────────────────────────────────────────────

#[test]
fn purl_subpath_only() {
    let p = PackageUrl::parse("pkg:generic/x@1#sp").unwrap();
    assert_eq!(p.subpath.as_deref(), Some("sp"));
}

#[test]
fn purl_multiple_qualifiers() {
    let p = PackageUrl::parse("pkg:rpm/rhel/x@1?arch=x86_64&distro=rhel-9").unwrap();
    assert_eq!(p.qualifiers.len(), 2);
}

#[test]
fn purl_ecosystem_mapping_full_table() {
    for (eco, expected) in &[
        ("npm", "npm"),
        ("nodejs", "npm"),
        ("pip", "pypi"),
        ("rubygems", "gem"),
        ("gomodules", "golang"),
        ("crates", "cargo"),
        ("packagist", "composer"),
        ("gradle", "maven"),
        ("hex", "hex"),
        ("pubspec", "pub"),
        ("debian", "deb"),
        ("ubuntu", "deb"),
        ("centos", "rpm"),
        ("opensuse", "rpm"),
        ("anything", "generic"),
    ] {
        assert_eq!(ecosystem_to_purl_type(eco), *expected);
    }
}

// ── OSV edges ───────────────────────────────────────────────────────────────

#[test]
fn osv_empty_object_parses() {
    let a = OsvAdvisory::parse("{\"id\":\"X\"}").unwrap();
    assert_eq!(a.id, "X");
    assert!(a.affected.is_empty());
}

#[test]
fn osv_batch_ndjson_with_blanks() {
    let v = OsvAdvisory::parse_batch("{\"id\":\"A\"}\n\n{\"id\":\"B\"}\n").unwrap();
    assert_eq!(v.len(), 2);
}

// ── VulnDB edges ────────────────────────────────────────────────────────────

#[test]
fn version_in_range_only_introduced() {
    assert!(version_in_range("1.0.0", Some("0"), None));
    assert!(!version_in_range("0.0.1", Some("1.0.0"), None));
}

#[test]
fn version_compare_with_letters_only() {
    // "alpha" < "beta" lexicographic
    assert_eq!(compare_versions("alpha", "beta"), -1);
}

#[test]
fn vulndb_lookup_unknown() {
    let db = VulnDb::cave_default();
    assert!(db.match_pkg("alpine", "no-such-pkg", "1.0").is_empty());
    assert!(db.lookup_id("nonexistent").is_none());
}

#[test]
fn vulndb_is_empty_initial() {
    let db = VulnDb::new();
    assert!(db.is_empty());
}

// ── Pkg parsers — corner cases ──────────────────────────────────────────────

#[test]
fn apk_blank_input() {
    assert!(parse_apk_installed("").is_empty());
}

#[test]
fn dpkg_no_origin_defaults_debian() {
    let p = parse_dpkg_status("Package: x\nVersion: 1\n");
    assert_eq!(p[0].ecosystem, "debian");
}

#[test]
fn rpm_text_skips_short_lines() {
    let p = parse_rpm_textdump("just-one-field\nname|1\n", OsFamily::Centos);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].name, "name");
}

#[test]
fn pubspec_two_packages() {
    let txt = "packages:\n  flutter:\n    version: \"3.16.0\"\n  http:\n    version: \"1.2.0\"\n";
    let p = parse_pubspec_lock(txt);
    assert_eq!(p.len(), 2);
}

#[test]
fn lockfile_dispatch_unknown_returns_empty() {
    assert!(parse_lockfile("README.md", "anything").is_empty());
}

#[test]
fn package_lock_v2_skips_root_entry() {
    let p = parse_package_lock(r#"{"packages":{"":{"name":"app"},"node_modules/x":{"version":"1"}}}"#);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].name, "x");
}

#[test]
fn yarn_lock_skips_comment_and_blank_lines() {
    let p = parse_yarn_lock("# header\n\n\"x@1\":\n  version \"1.0\"\n");
    assert_eq!(p.len(), 1);
}

#[test]
fn pnpm_skip_unrelated_lines() {
    let p = parse_pnpm_lock("settings:\n  autoInstallPeers: true\n/lodash/1.0.0:\n  dev: false\n");
    assert_eq!(p[0].name, "lodash");
}

#[test]
fn requirements_skips_directives() {
    let p = parse_requirements_txt("-r other.txt\n--index-url https://x\nrequests==2.31.0\n");
    assert_eq!(p.len(), 1);
}

#[test]
fn pipfile_lock_develop_section() {
    let p = parse_pipfile_lock(r#"{"develop":{"pytest":{"version":"==8.1.0"}}}"#);
    assert_eq!(p[0].name, "pytest");
}

#[test]
fn gemfile_lock_no_specs_block() {
    let p = parse_gemfile_lock("PLATFORMS\n  ruby\n");
    assert!(p.is_empty());
}

#[test]
fn go_mod_top_level_require() {
    let p = parse_go_mod("require github.com/x/y v1\n");
    assert_eq!(p[0].name, "github.com/x/y");
}

#[test]
fn go_sum_trims_go_mod_suffix() {
    let p = parse_go_sum("github.com/x/y v1.2.3/go.mod h1:abc\n");
    assert_eq!(p[0].version, "v1.2.3");
}

#[test]
fn cargo_lock_invalid_toml_returns_empty() {
    assert!(parse_cargo_lock("not toml [[[ ").is_empty());
}

#[test]
fn composer_lock_dev_packages() {
    let p = parse_composer_lock(r#"{"packages-dev":[{"name":"phpunit/phpunit","version":"11.0.0"}]}"#);
    assert_eq!(p[0].name, "phpunit/phpunit");
}

#[test]
fn pom_xml_skips_partial_dependency() {
    let p = parse_pom_xml("<dependency>\n<groupId>g</groupId>\n</dependency>\n");
    assert!(p.is_empty());
}

// ── Image scanner edges ─────────────────────────────────────────────────────

#[test]
fn image_scan_no_os_release_returns_empty_os() {
    let art = ImageArtifact {
        name: "x".into(),
        ..Default::default()
    };
    let r = scan_image(&art, &VulnDb::cave_default(), ScanImageOpts::default()).unwrap();
    assert!(r.os.is_none());
}

#[test]
fn image_scan_skip_both() {
    let opts = ScanImageOpts {
        skip_lang_pkgs: true,
        skip_os_pkgs: true,
    };
    let mut art = ImageArtifact {
        name: "x".into(),
        ..Default::default()
    };
    art.lockfiles.push(("Cargo.lock".into(), "".into()));
    let r = scan_image(&art, &VulnDb::cave_default(), opts).unwrap();
    assert_eq!(r.results.len(), 1);
    assert!(r.results[0].vulnerabilities.is_empty());
}

// ── SBOM edges ──────────────────────────────────────────────────────────────

#[test]
fn sbom_format_detect_case_insensitive() {
    assert_eq!(
        detect_format(r#"{"BOMFORMAT":"CYCLONEDX"}"#),
        Some(SbomFormat::CycloneDx)
    );
}

#[test]
fn sbom_purl_invalid_falls_back_generic() {
    assert_eq!(ecosystem_from_purl("not-a-purl"), "generic");
}

#[test]
fn sbom_round_trip_cyclonedx_to_spdx_via_packages() {
    let pkgs = vec![Package::new("openssl", "3.0.0", "alpine")];
    let c = emit_from_packages("x", &pkgs).unwrap();
    let s = spdx_emit("x", &pkgs).unwrap();
    assert_eq!(detect_format(&c), Some(SbomFormat::CycloneDx));
    assert_eq!(detect_format(&s), Some(SbomFormat::Spdx));
}

#[test]
fn scan_sbom_spdx_no_purl_falls_back_generic() {
    let s = r#"{"spdxVersion":"SPDX-2.3","packages":[{"name":"x","versionInfo":"1"}]}"#;
    let r = scan_sbom("x", s, &VulnDb::cave_default()).unwrap();
    assert_eq!(r.results[0].class, "sbom-pkgs");
}

// ── IaC edges ───────────────────────────────────────────────────────────────

#[test]
fn iac_detect_kind_tf_json() {
    let kinds = detect_kind("main.tf.json", "");
    assert!(kinds.contains(&"terraform"));
}

#[test]
fn iac_detect_kind_dockerfile_extension() {
    let kinds = detect_kind("build.Dockerfile", "");
    assert!(kinds.contains(&"dockerfile"));
}

#[test]
fn iac_chart_yaml_helm() {
    let kinds = detect_kind("Chart.yaml", "");
    assert!(kinds.contains(&"helm"));
}

#[test]
fn iac_validate_hcl_empty_ok() {
    assert!(validate_hcl("").is_none());
}

#[test]
fn iac_scan_unmatched_yaml_skipped() {
    let t = FsTree::default().push("notes.yaml", "not k8s\n");
    let r = scan_iac_tree(&t, &MisconfRegistry::builtin());
    assert!(r.is_empty());
}

// ── K8s scanner edges ───────────────────────────────────────────────────────

#[test]
fn k8s_resource_no_namespace_path() {
    let r = K8sResource {
        kind: "ClusterRole".into(),
        namespace: "".into(),
        name: "admin".into(),
        manifest_yaml: "".into(),
    };
    let path = cave_trivy::scan_k8s::resource_path(&r);
    assert_eq!(path, "ClusterRole/admin");
}

#[test]
fn k8s_filter_case_insensitive() {
    let snap = K8sClusterSnapshot {
        context: "x".into(),
        resources: vec![K8sResource {
            kind: "DAEMONSET".into(),
            namespace: "ns".into(),
            name: "ds".into(),
            manifest_yaml: "".into(),
        }],
    };
    assert_eq!(filter_by_kinds(&snap, &["daemonset"]).len(), 1);
}

// ── License edges ───────────────────────────────────────────────────────────

#[test]
fn license_unknown_text_returns_unknown_category() {
    assert_eq!(classify("Funky-license"), LicenseCategory::Unknown);
}

#[test]
fn license_detect_text_returns_multiple() {
    let v = detect_in_text("Apache License Version 2.0 + GNU AFFERO");
    assert!(v.len() >= 2);
}

#[test]
fn license_is_license_file_basename_table() {
    for f in ["LICENSE.TXT", "COPYING.TXT", "NOTICE", "license.md"] {
        let upper = f.to_ascii_uppercase();
        assert_eq!(is_license_file(f), is_license_file(&upper));
    }
}

#[test]
fn license_scan_empty_tree() {
    let v = scan_licenses(&[]);
    assert!(v.is_empty());
}

// ── VEX edges ───────────────────────────────────────────────────────────────

#[test]
fn vex_lookup_returns_none_for_unmatched_product() {
    let doc = VexDocument {
        context: "".into(),
        statements: vec![VexStatement {
            vulnerability: "CVE-X".into(),
            products: vec!["pkg:oci/a".into()],
            status: VexStatus::NotAffected,
            justification: None,
        }],
    };
    let idx = VexIndex::from_document(&doc);
    assert!(idx.lookup("pkg:oci/other", "CVE-X").is_none());
}

#[test]
fn vex_apply_under_investigation_keeps_vuln() {
    let doc = VexDocument {
        context: "".into(),
        statements: vec![VexStatement {
            vulnerability: "CVE-X".into(),
            products: vec!["pkg:a".into()],
            status: VexStatus::UnderInvestigation,
            justification: None,
        }],
    };
    let idx = VexIndex::from_document(&doc);
    let mut sr = ScanResult::default();
    sr.vulnerabilities.push(Vulnerability::new(
        "CVE-X",
        "p",
        "1",
        Severity::High,
    ));
    let n = apply(&idx, "pkg:a", &mut sr);
    assert_eq!(n, 0);
}

// ── Filter edges ────────────────────────────────────────────────────────────

#[test]
fn filter_no_constraints_keeps_all() {
    let mut r = Report::new("x", "y");
    let mut sr = ScanResult::default();
    sr.vulnerabilities.push(Vulnerability::new("X", "p", "1", Severity::Low));
    r.results.push(sr);
    let f = Filter::default();
    assert_eq!(f.apply(&mut r), 0);
}

#[test]
fn filter_min_severity_filters_misconfigs_too() {
    let mut r = Report::new("x", "y");
    r.results.push(ScanResult {
        target: "x".into(),
        class: "config".into(),
        misconfigurations: vec![cave_trivy::models::Misconfiguration {
            id: "A".into(),
            r#type: "terraform".into(),
            title: "x".into(),
            description: "x".into(),
            severity: Severity::Low,
            resource: "x".into(),
            references: vec![],
        }],
        ..Default::default()
    });
    let f = Filter::default().min_severity(Severity::High);
    f.apply(&mut r);
    assert_eq!(r.total_misconfigs(), 0);
}

// ── Ignore edges ────────────────────────────────────────────────────────────

#[test]
fn ignore_yaml_secrets_section() {
    let p = IgnorePolicy::parse_yaml_block("ignore:\n  secrets: [gh-pat]\n");
    assert!(p.matches_id("gh-pat"));
}

#[test]
fn ignore_yaml_no_ignore_block_is_empty() {
    let p = IgnorePolicy::parse_yaml_block("scanners:\n  - vuln\n");
    assert!(p.is_empty());
}

// ── Cache edges ─────────────────────────────────────────────────────────────

#[test]
fn cache_keys_unique() {
    let c = ScanCache::new();
    c.put("a", Report::new("a", "x"));
    c.put("b", Report::new("b", "x"));
    let keys = c.keys();
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(keys.len(), unique.len());
}

// ── Store edges ─────────────────────────────────────────────────────────────

#[test]
fn store_delete_nonexistent_returns_false() {
    let s = ScanStore::new();
    assert!(!s.delete("nope"));
}

#[test]
fn store_history_preserves_insertion_order() {
    let s = ScanStore::new();
    s.insert(Report::new("a", "x")).unwrap();
    s.insert(Report::new("b", "x")).unwrap();
    s.insert(Report::new("c", "x")).unwrap();
    assert_eq!(s.ids(), vec!["a", "b", "c"]);
}

// ── Report writers ──────────────────────────────────────────────────────────

#[test]
fn table_writer_summary_includes_artifact_name() {
    let r = Report::new("foo:1", "container_image");
    let t = table_write(&r);
    assert!(t.contains("foo:1"));
}

#[test]
fn template_renderer_unknown_field_passthrough() {
    let r = Report::new("x", "y");
    let s = tpl_render(&r, "value={{ .Bogus }}").unwrap();
    assert!(s.contains(".Bogus"));
}

#[test]
fn template_renderer_handles_no_tags() {
    let r = Report::new("x", "y");
    let s = tpl_render(&r, "just plain text").unwrap();
    assert_eq!(s, "just plain text");
}

// ── Engine renderers ────────────────────────────────────────────────────────

#[test]
fn engine_template_renderer() {
    let e = Engine::default();
    let mut r = Report::new("x", "y");
    let s = e
        .filter_and_render(
            &mut r,
            &Filter::default(),
            Renderer::Template("name={{ .ArtifactName }}".into()),
        )
        .unwrap();
    assert_eq!(s, "name=x");
}

#[test]
fn engine_cyclonedx_renderer_emits_components() {
    let e = Engine::default();
    let mut r = Report::new("x", "container_image");
    r.results.push(ScanResult {
        target: "x".into(),
        class: "os".into(),
        vulnerabilities: vec![Vulnerability::new("CVE", "p", "1.0", Severity::High)],
        ..Default::default()
    });
    let s = e
        .filter_and_render(&mut r, &Filter::default(), Renderer::CycloneDx)
        .unwrap();
    assert!(s.contains("\"components\""));
}

// ── server.rs request shapes ────────────────────────────────────────────────

#[test]
fn server_handle_fs_target() {
    let req = ScanRequest {
        target: ScanTarget::Fs,
        artifact_name: "/x".into(),
        min_severity: None,
        only_fixed: false,
        format: ReportFormat::Table,
        body: serde_json::Value::Null,
    };
    let r = handle(&req);
    assert!(r.rendered.contains("/x"));
}

#[test]
fn server_handle_sarif_format() {
    let req = ScanRequest {
        target: ScanTarget::Image,
        artifact_name: "x".into(),
        min_severity: None,
        only_fixed: false,
        format: ReportFormat::Sarif,
        body: serde_json::Value::Null,
    };
    let r = handle(&req);
    assert!(r.rendered.contains("sarif"));
}

#[test]
fn server_handle_spdx_format() {
    let req = ScanRequest {
        target: ScanTarget::Sbom,
        artifact_name: "x".into(),
        min_severity: None,
        only_fixed: false,
        format: ReportFormat::Spdx,
        body: serde_json::Value::Null,
    };
    let r = handle(&req);
    assert!(r.rendered.contains("SPDXRef-DOCUMENT"));
}

// ── K8s operator CRD shapes ─────────────────────────────────────────────────

#[test]
fn vuln_report_empty_summary_all_zero() {
    let report = Report::new("x", "container_image");
    let r = VulnerabilityReport::from_scan("x", "ns", "ghcr.io/x", "1", "sha:1", &report);
    assert_eq!(r.report.summary.critical, 0);
    assert_eq!(r.report.summary.high, 0);
    assert_eq!(r.report.summary.medium, 0);
    assert_eq!(r.report.summary.low, 0);
}

#[test]
fn vuln_report_registry_falls_back_dockerhub() {
    let report = Report::new("x", "container_image");
    let r = VulnerabilityReport::from_scan("x", "ns", "nginx", "latest", "sha:1", &report);
    assert_eq!(r.report.registry, "docker.io");
}

#[test]
fn config_audit_kind_const() {
    let r = ConfigAuditReport::from_report("p", "ns", &Report::new("x", "y"));
    assert_eq!(r.kind, "ConfigAuditReport");
}

// ── Misconfig registry  ─────────────────────────────────────────────────────

#[test]
fn misconf_count_floor() {
    assert!(MisconfRegistry::builtin().len() >= 12);
}

#[test]
fn misconf_rules_for_unknown_type_empty() {
    let r = MisconfRegistry::builtin();
    assert!(r.rules_for("nope").is_empty());
}

// ── Secret rule rules-by-category and confidence ────────────────────────────

#[test]
fn secret_rules_default_count() {
    let s = SecretRules::default_rules();
    assert!(s.len() >= 25);
}

#[test]
fn secret_rules_scan_empty_clean() {
    let s = SecretRules::default_rules();
    assert!(s.scan("clean.txt", "").is_empty());
}

#[test]
fn secret_rules_push_custom_then_match() {
    let mut s = SecretRules::default_rules();
    s.push(cave_trivy::scan_secret::SecretRule {
        id: "demo",
        category: "demo",
        severity: Severity::Low,
        keyword: "DEMOSEC",
        pattern: r"DEMOSEC=[a-z]+",
    });
    let v = s.scan("f", "DEMOSEC=abc");
    assert!(v.iter().any(|x| x.rule_id == "demo"));
}

// ── Round-trip / smoke combiner ────────────────────────────────────────────

#[test]
fn end_to_end_image_then_serialise_then_correlate() {
    let art = ImageArtifact {
        name: "alpine:3.19".into(),
        digest: "sha".into(),
        os_release: Some("ID=alpine\nVERSION_ID=3.19".into()),
        apk_db: Some("P:openssl\nV:3.0.0\n".into()),
        ..Default::default()
    };
    let e = Engine::default();
    let r = e.scan_image(&art, ScanImageOpts::default()).unwrap();
    let pkgs: Vec<_> = r
        .results
        .iter()
        .flat_map(|s| s.vulnerabilities.iter())
        .map(|v| Package::new(&v.pkg_name, &v.installed_version, "alpine"))
        .collect();
    let sbom = emit_from_packages(&art.name, &pkgs).unwrap();
    let r2 = scan_sbom(&art.name, &sbom, &VulnDb::cave_default()).unwrap();
    assert!(r2.total_vulns() >= 1);
}

#[test]
fn osrelease_returns_unknown_for_unsupported_id() {
    let r = OsRelease::parse("ID=haiku\nVERSION_ID=r1\n").unwrap();
    assert_eq!(r.family(), OsFamily::Unknown);
}

#[test]
fn version_compare_equal_tail_zero_padding() {
    assert_eq!(compare_versions("1.0", "1.0.0"), 0);
    assert_eq!(compare_versions("1.0.0", "1.0"), 0);
}

#[test]
fn vulndb_aliases_carried_through() {
    let mut db = VulnDb::new();
    let osv = OsvAdvisory::parse(
        r#"{"id":"CVE-X","aliases":["GHSA-1","GHSA-2"],"affected":[
            {"package":{"ecosystem":"npm","name":"x"},"ranges":[
                {"type":"SEMVER","events":[{"introduced":"0"},{"fixed":"1.0"}]}]}]}"#,
    )
    .unwrap();
    db.ingest_osv(&[osv]).unwrap();
    let e = db.lookup("npm", "x").first().unwrap().clone();
    assert_eq!(e.aliases, vec!["GHSA-1", "GHSA-2"]);
}
