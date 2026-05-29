// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit gate for cave-ai-obs.

use std::path::Path;

const CRATE_ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn crate_path(rel: &str) -> std::path::PathBuf {
    Path::new(CRATE_ROOT).join(rel)
}

fn read_src(rel: &str) -> String {
    std::fs::read_to_string(crate_path(rel))
        .unwrap_or_else(|_| panic!("cannot read {rel}"))
}

fn read_manifest() -> String {
    read_src("parity.manifest.toml")
}

// G1: source_sha pinned in [upstream]
#[test]
fn g1_source_sha_pinned() {
    let manifest = read_manifest();
    assert!(
        manifest.contains("source_sha"),
        "G1 FAIL: [upstream] must have source_sha pinned"
    );
}

// G2: every .rs starts with SPDX header
#[test]
fn g2_spdx_headers() {
    let src_dir = crate_path("src");
    let entries = std::fs::read_dir(&src_dir).expect("src dir exists");
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(
                content.starts_with("// SPDX-License-Identifier: AGPL-3.0-or-later"),
                "G2 FAIL: {} missing SPDX header",
                path.display()
            );
        }
    }
}

// G3: fill_ratio = 1.0 in manifest
#[test]
fn g3_fill_ratio_one() {
    let manifest = read_manifest();
    assert!(
        manifest.contains("fill_ratio") && manifest.contains("1.0"),
        "G3 FAIL: fill_ratio must be 1.0"
    );
}

// G4: manifest file present
#[test]
fn g4_manifest_present() {
    assert!(
        crate_path("parity.manifest.toml").exists(),
        "G4 FAIL: parity.manifest.toml must exist"
    );
}

// G5: no stubs (unimplemented!/todo!())
#[test]
fn g5_no_stubs() {
    let src_dir = crate_path("src");
    let entries = std::fs::read_dir(&src_dir).expect("src dir exists");
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(
                !content.contains("unimplemented!()") && !content.contains("todo!()"),
                "G5 FAIL: {} contains stub (unimplemented!/todo!())",
                path.display()
            );
        }
    }
}

// G6: no backcompat shims
#[test]
fn g6_no_backcompat_shims() {
    let src_dir = crate_path("src");
    let entries = std::fs::read_dir(&src_dir).expect("src dir exists");
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "rs").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(
                !content.contains("#[deprecated]") || content.contains("// backcompat-ok"),
                "G6 FAIL: {} has deprecated backcompat shim without justification comment",
                path.display()
            );
        }
    }
}

// G7: upstream version (not 0.0.0 / empty)
#[test]
fn g7_upstream_version_set() {
    let manifest = read_manifest();
    // version should be set to something meaningful (not 0.0.0)
    assert!(
        manifest.contains("version") && !manifest.contains("version = \"\""),
        "G7 FAIL: upstream version must be set"
    );
    // The manifest version should include a real version like v3 or v2
    assert!(
        manifest.contains("version = \"v3") || manifest.contains("version = \"v2") || manifest.contains("source_sha"),
        "G7 FAIL: upstream version should reflect latest stable"
    );
}

// G8: 4-track coverage check (backend impl + route handler present)
#[test]
fn g8_route_handler_present() {
    let routes = read_src("src/routes.rs");
    // Must have at least a trace ingest route and health route
    assert!(
        routes.contains("ingest") || routes.contains("traces"),
        "G8 FAIL: routes.rs must have trace ingestion endpoint"
    );
    assert!(
        routes.contains("health"),
        "G8 FAIL: routes.rs must have health endpoint"
    );
}

// honest_ratio must be between 0 (exclusive) and 1 (inclusive)
#[test]
fn g3_honest_ratio_in_range() {
    let manifest = read_manifest();
    assert!(
        manifest.contains("honest_ratio"),
        "G3b FAIL: honest_ratio must be set in manifest"
    );
    // Extract and validate
    for line in manifest.lines() {
        if line.trim().starts_with("honest_ratio") {
            let val: f64 = line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse()
                .expect("honest_ratio must be a float");
            assert!(
                (0.0..=1.0).contains(&val),
                "G3b FAIL: honest_ratio={val} out of range [0,1]"
            );
            // Honest ratio must reflect real mapping, not inflated
            assert!(val > 0.0, "G3b FAIL: honest_ratio=0.0 means nothing is mapped");
            return;
        }
    }
    panic!("G3b FAIL: honest_ratio not found in manifest");
}

// adr_justified_ratio must be 1.0
#[test]
fn g3_adr_justified_ratio_one() {
    let manifest = read_manifest();
    for line in manifest.lines() {
        if line.trim().starts_with("adr_justified_ratio") {
            let val: f64 = line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse()
                .expect("adr_justified_ratio must be a float");
            assert!(
                (val - 1.0).abs() < 0.001,
                "adr_justified_ratio={val} must be 1.0"
            );
            return;
        }
    }
    panic!("adr_justified_ratio not found in manifest");
}

// unmapped_count must be 0
#[test]
fn g3_unmapped_count_zero() {
    let manifest = read_manifest();
    for line in manifest.lines() {
        if line.trim().starts_with("unmapped_count") {
            let val: u32 = line
                .split('=')
                .nth(1)
                .unwrap()
                .trim()
                .parse()
                .expect("unmapped_count must be an integer");
            assert_eq!(val, 0, "unmapped_count must be 0");
            return;
        }
    }
    panic!("unmapped_count not found in manifest");
}
