// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/fanal/{analyzer,types,artifact}/...
//
// Line-by-line ports of Trivy v0.70.0 table-driven Go tests.
// Each test cites its upstream test path so the parity audit can
// link them back to the Go counterpart.

use cave_scan::analyzer::{
    Analyzer, AnalyzerRegistry, AnalyzerType, binary::BinaryAnalyzer, language::CargoLockAnalyzer,
    language::NpmLockAnalyzer, os::AlpineApkAnalyzer, os::DpkgStatusAnalyzer,
};
use cave_scan::oci::layer::{LayerCompression, detect_layer_compression};
use cave_scan::oci::manifest::{ImageManifest, MediaType};
use cave_scan::report_agg::{ScannerReport, aggregate};
use cave_scan::scanners::fs::FsScanner;
use cave_scan::scanners::{ScanRequest, ScanTarget, Scanner};
use cave_scan::target::{TargetKind, detect_target};

// ── Analyzer: Alpine APK installed DB ────────────────────────────────────────
// Upstream test: pkg/fanal/analyzer/pkg/apk/apk_test.go::TestParseApkInfo
#[test]
fn alpine_apk_parses_single_package() {
    let input = "C:Q1eVK4ohEvLBhAGRk++QM5ZWeXqQg=\nP:musl\nV:1.2.5-r0\nA:x86_64\no:musl\nL:MIT\n\n";
    let pkgs = AlpineApkAnalyzer.parse_installed_db(input);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "musl");
    assert_eq!(pkgs[0].version, "1.2.5-r0");
    assert_eq!(pkgs[0].arch.as_deref(), Some("x86_64"));
    assert_eq!(pkgs[0].license.as_deref(), Some("MIT"));
    assert_eq!(pkgs[0].origin.as_deref(), Some("musl"));
}

// Upstream test: pkg/fanal/analyzer/pkg/apk/apk_test.go::TestParseApkInfo (multi)
#[test]
fn alpine_apk_parses_multiple_packages() {
    let input = "P:musl\nV:1.2.5-r0\n\nP:busybox\nV:1.36.1-r29\nL:GPL-2.0-only\n\n";
    let pkgs = AlpineApkAnalyzer.parse_installed_db(input);
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs[0].name, "musl");
    assert_eq!(pkgs[1].name, "busybox");
    assert_eq!(pkgs[1].license.as_deref(), Some("GPL-2.0-only"));
}

// Upstream test: pkg/fanal/analyzer/pkg/apk/apk_test.go::TestParseApkInfo (empty)
#[test]
fn alpine_apk_empty_input_returns_no_packages() {
    let pkgs = AlpineApkAnalyzer.parse_installed_db("");
    assert!(pkgs.is_empty());
}

// Upstream test: pkg/fanal/analyzer/pkg/apk/apk_test.go::TestApk_Required
#[test]
fn alpine_apk_required_matches_lib_apk_db() {
    assert!(AlpineApkAnalyzer.required("lib/apk/db/installed"));
    assert!(AlpineApkAnalyzer.required("usr/lib/apk/db/installed"));
    assert!(!AlpineApkAnalyzer.required("etc/passwd"));
}

// Upstream test: pkg/fanal/analyzer/pkg/apk/apk_test.go::TestParseApkInfo (provides+deps)
#[test]
fn alpine_apk_parses_provides_and_deps() {
    let input = "P:nginx\nV:1.27.0\np:webserver httpd\nD:musl libssl3\n\n";
    let pkgs = AlpineApkAnalyzer.parse_installed_db(input);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].provides, vec!["webserver", "httpd"]);
    assert_eq!(pkgs[0].depends, vec!["musl", "libssl3"]);
}

// Upstream test: pkg/fanal/analyzer/pkg/apk/apk_test.go (trailing blank lines)
#[test]
fn alpine_apk_trailing_blank_lines_no_phantom_entry() {
    let input = "P:zlib\nV:1.3.1\n\n\n\n";
    let pkgs = AlpineApkAnalyzer.parse_installed_db(input);
    assert_eq!(pkgs.len(), 1);
}

// ── Analyzer: dpkg status ────────────────────────────────────────────────────
// Upstream test: pkg/fanal/analyzer/pkg/dpkg/dpkg_test.go::TestParseDpkgStatus
#[test]
fn dpkg_status_parses_single_installed_package() {
    let input = "Package: libc6\nStatus: install ok installed\nVersion: 2.36-9+deb12u8\nArchitecture: amd64\nSource: glibc\n\n";
    let pkgs = DpkgStatusAnalyzer.parse_status(input);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "libc6");
    assert_eq!(pkgs[0].version, "2.36-9+deb12u8");
    assert_eq!(pkgs[0].arch.as_deref(), Some("amd64"));
    assert_eq!(pkgs[0].source.as_deref(), Some("glibc"));
}

// Upstream test: pkg/fanal/analyzer/pkg/dpkg/dpkg_test.go (deinstall filter)
#[test]
fn dpkg_status_skips_deinstalled() {
    let input = "Package: gone\nStatus: deinstall ok config-files\nVersion: 1.0\n\nPackage: real\nStatus: install ok installed\nVersion: 2.0\n\n";
    let pkgs = DpkgStatusAnalyzer.parse_status(input);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "real");
}

// Upstream test: pkg/fanal/analyzer/pkg/dpkg/dpkg_test.go (Source with version)
#[test]
fn dpkg_status_parses_source_with_version() {
    let input =
        "Package: libfoo\nStatus: install ok installed\nVersion: 1.0-1\nSource: foo (0.9)\n\n";
    let pkgs = DpkgStatusAnalyzer.parse_status(input);
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].source.as_deref(), Some("foo"));
    assert_eq!(pkgs[0].source_version.as_deref(), Some("0.9"));
}

// Upstream test: pkg/fanal/analyzer/pkg/dpkg/dpkg_test.go::TestDpkg_Required
#[test]
fn dpkg_required_matches_status_path() {
    assert!(DpkgStatusAnalyzer.required("var/lib/dpkg/status"));
    assert!(!DpkgStatusAnalyzer.required("var/lib/rpm/Packages"));
}

// Upstream test: pkg/fanal/analyzer/pkg/dpkg/dpkg_test.go (empty)
#[test]
fn dpkg_status_empty_input() {
    assert!(DpkgStatusAnalyzer.parse_status("").is_empty());
}

// ── Analyzer: npm package-lock.json ──────────────────────────────────────────
// Upstream test: pkg/fanal/analyzer/language/nodejs/npm/npm_test.go (lockfile v2/v3)
#[test]
fn npm_lock_v3_parses_packages_field() {
    let lock = r#"{
        "name": "demo",
        "lockfileVersion": 3,
        "packages": {
            "": { "name": "demo", "version": "1.0.0" },
            "node_modules/lodash": { "version": "4.17.21", "license": "MIT" },
            "node_modules/semver": { "version": "7.5.4" }
        }
    }"#;
    let pkgs = NpmLockAnalyzer.parse_lock(lock).expect("valid lock");
    let names: Vec<_> = pkgs.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"lodash"));
    assert!(names.contains(&"semver"));
    let lodash = pkgs.iter().find(|p| p.name == "lodash").unwrap();
    assert_eq!(lodash.version, "4.17.21");
    assert_eq!(lodash.license.as_deref(), Some("MIT"));
}

// Upstream test: pkg/fanal/analyzer/language/nodejs/npm/npm_test.go (legacy v1)
#[test]
fn npm_lock_v1_parses_dependencies_field() {
    let lock = r#"{
        "name": "old",
        "lockfileVersion": 1,
        "dependencies": {
            "left-pad": { "version": "1.3.0" }
        }
    }"#;
    let pkgs = NpmLockAnalyzer.parse_lock(lock).expect("valid lock");
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].name, "left-pad");
    assert_eq!(pkgs[0].version, "1.3.0");
}

#[test]
fn npm_lock_invalid_json_errors() {
    assert!(NpmLockAnalyzer.parse_lock("not json").is_err());
}

#[test]
fn npm_lock_required_matches_package_lock_json() {
    assert!(NpmLockAnalyzer.required("package-lock.json"));
    assert!(NpmLockAnalyzer.required("app/package-lock.json"));
    // Trivy explicitly skips lockfiles inside node_modules.
    assert!(!NpmLockAnalyzer.required("node_modules/foo/package-lock.json"));
}

// ── Analyzer: Cargo.lock ─────────────────────────────────────────────────────
// Upstream test: pkg/fanal/analyzer/language/rust/cargo/cargo_test.go
#[test]
fn cargo_lock_parses_packages() {
    let lock = r#"
version = 3

[[package]]
name = "serde"
version = "1.0.210"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tokio"
version = "1.38.0"
"#;
    let pkgs = CargoLockAnalyzer
        .parse_lock(lock)
        .expect("valid Cargo.lock");
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs[0].name, "serde");
    assert_eq!(pkgs[0].version, "1.0.210");
    assert_eq!(pkgs[1].name, "tokio");
}

#[test]
fn cargo_lock_required_matches_root_lock_only() {
    assert!(CargoLockAnalyzer.required("Cargo.lock"));
    assert!(CargoLockAnalyzer.required("app/Cargo.lock"));
    // Trivy skips vendor lockfiles.
    assert!(!CargoLockAnalyzer.required("vendor/foo/Cargo.lock"));
}

// ── Analyzer registry ────────────────────────────────────────────────────────
// Upstream test: pkg/fanal/analyzer/analyzer_test.go::TestAnalyzerGroup_AnalyzeFile
#[test]
fn registry_dispatches_alpine_apk() {
    let reg = AnalyzerRegistry::default_set();
    let chosen = reg.analyzers_for("lib/apk/db/installed");
    assert!(chosen.iter().any(|a| a.kind() == AnalyzerType::AlpineApk));
}

#[test]
fn registry_dispatches_dpkg() {
    let reg = AnalyzerRegistry::default_set();
    let chosen = reg.analyzers_for("var/lib/dpkg/status");
    assert!(chosen.iter().any(|a| a.kind() == AnalyzerType::DpkgStatus));
}

#[test]
fn registry_dispatches_npm() {
    let reg = AnalyzerRegistry::default_set();
    let chosen = reg.analyzers_for("app/package-lock.json");
    assert!(chosen.iter().any(|a| a.kind() == AnalyzerType::Npm));
}

#[test]
fn registry_no_match_returns_empty() {
    let reg = AnalyzerRegistry::default_set();
    assert!(reg.analyzers_for("README.md").is_empty());
}

// ── Binary analyzer ──────────────────────────────────────────────────────────
// Upstream test: pkg/fanal/analyzer/executable/executable_test.go (ELF magic)
#[test]
fn binary_recognises_elf_magic() {
    let elf = [0x7f, b'E', b'L', b'F', 0x02, 0x01, 0x01, 0x00];
    assert!(BinaryAnalyzer.is_executable(&elf));
}

// Upstream test: PE magic check
#[test]
fn binary_recognises_pe_magic() {
    let pe = [b'M', b'Z', 0x90, 0x00];
    assert!(BinaryAnalyzer.is_executable(&pe));
}

#[test]
fn binary_rejects_text_file() {
    let txt = b"#!/bin/sh\necho hello\n";
    assert!(!BinaryAnalyzer.is_executable(txt));
}

// ── OCI manifest ─────────────────────────────────────────────────────────────
// Upstream test: pkg/fanal/image/daemon/image_test.go (manifest decode)
#[test]
fn oci_manifest_parses_v1_image_manifest() {
    let json = r#"{
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": "sha256:abc",
            "size": 1234
        },
        "layers": [
            {
                "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                "digest": "sha256:def",
                "size": 999
            }
        ]
    }"#;
    let m = ImageManifest::parse(json).expect("manifest decodes");
    assert_eq!(m.schema_version, 2);
    assert_eq!(m.config.digest, "sha256:abc");
    assert_eq!(m.layers.len(), 1);
    assert_eq!(m.layers[0].size, 999);
}

#[test]
fn oci_manifest_recognises_docker_media_type() {
    assert_eq!(
        MediaType::from_str("application/vnd.docker.distribution.manifest.v2+json"),
        Some(MediaType::DockerManifestV2)
    );
    assert_eq!(
        MediaType::from_str("application/vnd.oci.image.manifest.v1+json"),
        Some(MediaType::OciManifestV1)
    );
    assert_eq!(MediaType::from_str("text/plain"), None);
}

#[test]
fn oci_manifest_invalid_json_errors() {
    assert!(ImageManifest::parse("not json").is_err());
}

// ── Layer compression detection ──────────────────────────────────────────────
// Upstream test: pkg/fanal/image/img_test.go (gzip vs tar detection)
#[test]
fn layer_detects_gzip_magic() {
    let gz = [0x1f, 0x8b, 0x08, 0x00];
    assert_eq!(detect_layer_compression(&gz), LayerCompression::Gzip);
}

#[test]
fn layer_detects_zstd_magic() {
    let zst = [0x28, 0xb5, 0x2f, 0xfd];
    assert_eq!(detect_layer_compression(&zst), LayerCompression::Zstd);
}

#[test]
fn layer_uncompressed_default() {
    let raw = [0x75, 0x73, 0x74, 0x61, 0x72]; // "ustar" tar magic offset 257 not at 0; still uncompressed
    assert_eq!(detect_layer_compression(&raw), LayerCompression::None);
}

// ── Target detection ─────────────────────────────────────────────────────────
// Upstream test: pkg/fanal/artifact/local/fs_test.go (kind detection)
#[test]
fn target_detect_image_tarball_by_extension() {
    assert_eq!(detect_target("image.tar"), TargetKind::ImageTar);
    assert_eq!(detect_target("image.tar.gz"), TargetKind::ImageTar);
}

#[test]
fn target_detect_filesystem_for_directory_like() {
    // Trivy treats a bare path that has no image-y extension as a filesystem.
    assert_eq!(detect_target("/home/user/project"), TargetKind::Filesystem);
}

#[test]
fn target_detect_sbom_by_extension() {
    assert_eq!(detect_target("sbom.cdx.json"), TargetKind::Sbom);
    assert_eq!(detect_target("sbom.spdx.json"), TargetKind::Sbom);
}

#[test]
fn target_detect_image_reference_by_form() {
    assert_eq!(detect_target("alpine:3.20"), TargetKind::ImageReference);
    assert_eq!(
        detect_target("docker.io/library/nginx:1.27"),
        TargetKind::ImageReference
    );
}

// ── Filesystem scanner end-to-end ────────────────────────────────────────────
// Upstream test: pkg/fanal/artifact/local/fs_test.go::TestArtifact_Inspect
#[test]
fn fs_scanner_finds_packages_in_temp_dir() {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    let tmp = std::env::temp_dir().join(format!("cave-scan-fs-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(tmp.join("lib/apk/db")).unwrap();
    let mut f = fs::File::create(tmp.join("lib/apk/db/installed")).unwrap();
    f.write_all(b"P:hello\nV:1.0\n\n").unwrap();

    fs::create_dir_all(tmp.join("app")).unwrap();
    let mut g = fs::File::create(tmp.join("app/Cargo.lock")).unwrap();
    g.write_all(b"version = 3\n\n[[package]]\nname = \"world\"\nversion = \"2.0\"\n")
        .unwrap();

    let scanner = FsScanner::new();
    let req = ScanRequest {
        target: ScanTarget::Filesystem(PathBuf::from(&tmp)),
    };
    let report = scanner.scan(&req).expect("fs scan succeeds");

    fs::remove_dir_all(&tmp).ok();

    let names: Vec<_> = report.packages.iter().map(|p| p.name.as_str()).collect();
    assert!(
        names.contains(&"hello"),
        "expected apk hello, got {:?}",
        names
    );
    assert!(
        names.contains(&"world"),
        "expected cargo world, got {:?}",
        names
    );
}

// ── Report aggregation ───────────────────────────────────────────────────────
// Upstream test: pkg/report/report_test.go (multi-scanner merge)
#[test]
fn report_aggregate_merges_two_scanners() {
    use cave_scan::analyzer::PackageInfo;
    let r1 = ScannerReport {
        scanner_name: "fs".into(),
        target: "/tmp/a".into(),
        packages: vec![PackageInfo {
            name: "foo".into(),
            version: "1.0".into(),
            ..Default::default()
        }],
    };
    let r2 = ScannerReport {
        scanner_name: "image".into(),
        target: "alpine:3.20".into(),
        packages: vec![PackageInfo {
            name: "bar".into(),
            version: "2.0".into(),
            ..Default::default()
        }],
    };
    let agg = aggregate(vec![r1, r2]);
    assert_eq!(agg.total_packages, 2);
    assert_eq!(agg.reports.len(), 2);
}

#[test]
fn report_aggregate_empty() {
    let agg = aggregate(vec![]);
    assert_eq!(agg.total_packages, 0);
    assert!(agg.reports.is_empty());
}
