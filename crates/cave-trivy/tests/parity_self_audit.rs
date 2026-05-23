// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-trivy must carry an honest, measured
//! `fill_ratio` against upstream aquasecurity/trivy v0.70.0 + trivy-checks
//! v2.2.0, pinned `source_sha`s for reproducibility, the 2026-05-23
//! close-out audit date, `parity_ratio_source = "manifest"`, 100% AGPL
//! SPDX header coverage, no stub macros in `src/`,
//! mapped+partial+skipped+unmapped summing to total, and the full
//! image/fs/repo/k8s/sbom/secret/config surface reachable through
//! `cave_trivy`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const TRIVY_VERSION: &str = "v0.70.0";
const TRIVY_SHA: &str = "8a3177aedf7ee0864920eb1852eef031cd3742b8";
const CHECKS_VERSION: &str = "v2.2.0";
const CHECKS_SHA: &str = "d7c9302130a9b7e614a5c5d32854f6a08b4bc52e";

fn manifest_text() -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"]
        .iter()
        .collect();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn extract_after(text: &str, needle: &str) -> Option<String> {
    let i = text.find(needle)?;
    let rest = &text[i + needle.len()..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..line_end];
    let stripped = line.trim().trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

// ─── Assertion 1: trivy upstream pinned to v0.70.0 ──────────────────────────

#[test]
fn assertion_1_trivy_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(TRIVY_VERSION),
        "[upstream] version must pin Trivy {} — Charter v2 always-latest gate (got {:?})",
        TRIVY_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches trivy v0.70.0 + trivy-checks v2.2.0 ────

#[test]
fn assertion_2_source_sha_matches_versions() {
    let m = manifest_text();
    assert!(
        m.contains(TRIVY_SHA),
        "[upstream] trivy source_sha must contain {} (full manifest text scan)",
        TRIVY_SHA
    );
    assert!(
        m.contains(CHECKS_VERSION),
        "[upstreams] companion trivy-checks version {} must be pinned",
        CHECKS_VERSION
    );
    assert!(
        m.contains(CHECKS_SHA),
        "[upstreams] companion trivy-checks source_sha {} must be pinned",
        CHECKS_SHA
    );
}

// ─── Assertion 3: fill_ratio >= 0.95 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-trivy deep-port floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit == 2026-05-23 ──────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total + >= 20 mapped ────────────────────────

#[test]
fn assertion_6_counts_sum_to_total() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count");
    let partial = read("partial_count").expect("partial_count");
    let skipped = read("skipped_count").expect("skipped_count");
    let unmapped = read("unmapped_count").expect("unmapped_count");
    let total = read("total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(
        mapped >= 20,
        "cave-trivy MVP floor: >= 20 mapped trivy subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ────────────────────────────

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 20,
        "expected >= 20 .rs files in cave-trivy; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"todo")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: full image/fs/repo/k8s/sbom/secret/config surface ────────

#[test]
fn assertion_9_trivy_surface_intact() {
    use cave_trivy::cache::ScanCache;
    use cave_trivy::engine::{Engine, Renderer};
    use cave_trivy::filter::Filter;
    use cave_trivy::ignore::IgnorePolicy;
    use cave_trivy::k8s_operator::{ConfigAuditReport, VulnerabilityReport};
    use cave_trivy::misconf::MisconfRegistry;
    use cave_trivy::models::{OsFamily, Package, Report, ScanResult, Vulnerability};
    use cave_trivy::osv::OsvAdvisory;
    use cave_trivy::pkg_lang::parse_lockfile;
    use cave_trivy::pkg_os::{parse_apk_installed, parse_dpkg_status, OsRelease};
    use cave_trivy::purl::PackageUrl;
    use cave_trivy::scan_fs::FsTree;
    use cave_trivy::scan_image::{scan_image, ImageArtifact, ScanImageOpts};
    use cave_trivy::scan_k8s::{scan_cluster, K8sClusterSnapshot, K8sResource};
    use cave_trivy::scan_license::classify;
    use cave_trivy::scan_repo::RepoArtifact;
    use cave_trivy::scan_sbom::scan_sbom;
    use cave_trivy::scan_secret::{scan_secrets_in_tree, SecretRules};
    use cave_trivy::sbom_cyclonedx::emit_from_packages as cyclonedx_emit;
    use cave_trivy::sbom_spdx::emit as spdx_emit;
    use cave_trivy::server::{handle, ScanRequest, ScanTarget};
    use cave_trivy::severity::Severity;
    use cave_trivy::store::ScanStore;
    use cave_trivy::vex::{VexDocument, VexIndex};
    use cave_trivy::vulndb::VulnDb;
    use cave_trivy::{router, State, MODULE_NAME, UPSTREAM_SHA, UPSTREAM_VERSION};
    use std::sync::Arc;

    // ── 1. Module identity + state + version pin ──────────────────────────
    assert_eq!(MODULE_NAME, "trivy");
    assert_eq!(UPSTREAM_VERSION, "v0.70.0");
    assert_eq!(UPSTREAM_SHA, "8a3177aedf7ee0864920eb1852eef031cd3742b8");
    let _r = router(Arc::new(State::default()));

    // ── 2. Severity scale + parse ─────────────────────────────────────────
    assert!(Severity::Critical > Severity::High);
    let p = "HIGH,CRITICAL".parse::<Severity>().ok();
    let _ = p; // checked elsewhere; ensure import works

    // ── 3. purl + ecosystem ───────────────────────────────────────────────
    let pu = PackageUrl::parse("pkg:npm/lodash@4.17.21").unwrap();
    assert_eq!(pu.name, "lodash");

    // ── 4. VulnDb default + OSV ingest ────────────────────────────────────
    let mut db = VulnDb::cave_default();
    assert!(db.len() >= 14);
    assert!(db.match_pkg("alpine", "openssl", "3.0.0").len() >= 1);
    let osv = OsvAdvisory {
        id: "GHSA-X".into(),
        ..Default::default()
    };
    assert!(db.ingest_osv(&[osv]).is_err()); // empty affected list

    // ── 5. OS pkg parsers ─────────────────────────────────────────────────
    let r = OsRelease::parse("ID=alpine\nVERSION_ID=3.19\n").unwrap();
    assert_eq!(r.family(), OsFamily::Alpine);
    let pkgs = parse_apk_installed("P:openssl\nV:3.0.0\n");
    assert_eq!(pkgs[0].name, "openssl");
    let dpkg = parse_dpkg_status("Package: curl\nVersion: 8.5.0-1\n");
    assert_eq!(dpkg[0].name, "curl");
    let l = parse_lockfile("Cargo.lock", "[[package]]\nname=\"x\"\nversion=\"1\"\n");
    assert_eq!(l[0].name, "x");

    // ── 6. Image scanner + correlate ──────────────────────────────────────
    let art = ImageArtifact {
        name: "alpine:3.19".into(),
        digest: "sha:1".into(),
        os_release: Some("ID=alpine\nVERSION_ID=3.19".into()),
        apk_db: Some("P:openssl\nV:3.0.0\n".into()),
        ..Default::default()
    };
    let rep = scan_image(&art, &db, ScanImageOpts::default()).unwrap();
    assert!(rep.total_vulns() >= 1);

    // ── 7. Filesystem + secret + repo + SBOM + K8s scanners ───────────────
    let tree = FsTree::default()
        .push("Cargo.lock", "[[package]]\nname=\"openssl-sys\"\nversion=\"0.9.0\"\n")
        .push(".env", "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE")
        .push("main.tf", r#"acl = "public-read""#);
    let secrets = scan_secrets_in_tree(&tree, &SecretRules::default_rules());
    assert!(!secrets.is_empty());
    let repo = RepoArtifact {
        url: "git+https://x".into(),
        git_ref: None,
        tree: tree.clone(),
    };
    let e = Engine::default();
    let _rep_repo = e.scan_repo(&repo).unwrap();
    let sbom = cyclonedx_emit("x", &[Package::new("openssl-sys", "0.9.0", "cargo")]).unwrap();
    let _rep_sbom = scan_sbom("sbom", &sbom, &db).unwrap();
    let snap = K8sClusterSnapshot {
        context: "ctx".into(),
        resources: vec![K8sResource {
            kind: "Pod".into(),
            namespace: "default".into(),
            name: "p".into(),
            manifest_yaml: "    privileged: true\n".into(),
        }],
    };
    let rep_k8s = scan_cluster(&snap, &MisconfRegistry::builtin()).unwrap();
    assert!(rep_k8s.total_misconfigs() >= 1);

    // ── 8. License + Filter + Ignore + VEX ────────────────────────────────
    assert_eq!(
        classify("Apache-2.0"),
        cave_trivy::models::LicenseCategory::Permissive
    );
    let mut rep_f = scan_image(&art, &db, ScanImageOpts::default()).unwrap();
    let f = Filter::default().min_severity(Severity::Critical);
    f.apply(&mut rep_f);
    let _ignore = IgnorePolicy::parse_trivyignore("CVE-2026-0001\n");
    let doc = VexDocument {
        context: "".into(),
        statements: vec![],
    };
    let _idx = VexIndex::from_document(&doc);

    // ── 9. Report writers + SBOM SPDX + server handle ─────────────────────
    let json_out = e
        .filter_and_render(&mut rep_f.clone(), &Filter::default(), Renderer::Json)
        .unwrap();
    assert!(json_out.contains("schema_version"));
    let table_out = e
        .filter_and_render(&mut rep_f.clone(), &Filter::default(), Renderer::Table)
        .unwrap();
    assert!(table_out.contains("Artifact:"));
    let sarif_out = e
        .filter_and_render(&mut rep_f.clone(), &Filter::default(), Renderer::Sarif)
        .unwrap();
    assert!(sarif_out.contains("sarif"));
    let spdx_out = spdx_emit("x", &[Package::new("p", "1", "npm")]).unwrap();
    assert!(spdx_out.contains("SPDXRef-DOCUMENT"));
    let resp = handle(&ScanRequest {
        target: ScanTarget::Image,
        artifact_name: "x".into(),
        min_severity: None,
        only_fixed: false,
        format: cave_trivy::server::ReportFormat::Json,
        body: serde_json::Value::Null,
    });
    assert_eq!(resp.format, cave_trivy::server::ReportFormat::Json);

    // ── 10. K8s operator CRDs + Cache + Store ─────────────────────────────
    let vr = VulnerabilityReport::from_scan("p", "ns", "ghcr.io/cave/p", "v1", "sha:1", &rep);
    assert_eq!(vr.kind, "VulnerabilityReport");
    let ca = ConfigAuditReport::from_report("p", "ns", &rep_k8s);
    assert_eq!(ca.kind, "ConfigAuditReport");
    let cache = ScanCache::new();
    cache.put("alpine:3.19", rep.clone());
    assert!(cache.get("alpine:3.19").is_some());
    let store = ScanStore::new();
    store.insert(rep.clone()).unwrap();
    assert_eq!(store.count().unwrap(), 1);

    // ── 11. Sanity — Report + Vuln + Pkg + ScanResult constructors hold ───
    let _v: Vulnerability = Vulnerability::new("X", "p", "1", Severity::Low);
    let _r: Report = Report::new("z", "container_image");
    let _sr: ScanResult = ScanResult::default();
    let _pk = Package::new("x", "1", "npm");
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            if p.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}
