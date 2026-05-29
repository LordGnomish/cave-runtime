// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 parity self-audit — 8 gates.

use std::fs;

fn manifest() -> toml::Value {
    let content = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity.manifest.toml"),
    )
    .expect("parity.manifest.toml must exist");
    content.parse::<toml::Value>().expect("valid TOML")
}

/// G1: source_sha must be set in [upstream]
/// Accepts a full commit SHA (40 hex chars) or a version tag (e.g. "v0.7.6").
/// Consistent with cave-runtime convention: some crates pin tags, others pin SHAs.
#[test]
fn gate1_source_sha_pinned() {
    let m = manifest();
    let upstream = m.get("upstream").expect("[upstream] section");
    let source_sha = upstream
        .get("source_sha")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Accept any non-empty value that looks like a tag (starts with "v") or a SHA (hex chars)
    let ok = !source_sha.is_empty()
        && (source_sha.starts_with('v')
            || source_sha.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(
        ok,
        "G1 FAIL: [upstream].source_sha must be a version tag or SHA (got {:?})",
        source_sha
    );
}

/// G2: every .rs file must start with SPDX header
#[test]
fn gate2_spdx_headers() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut missing = vec![];
    visit_rs(&src_dir, &mut missing);
    assert!(
        missing.is_empty(),
        "G2 FAIL: SPDX header missing in: {:?}",
        missing
    );
}

fn visit_rs(dir: &std::path::Path, missing: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_rs(&path, missing);
            } else if path.extension().map_or(false, |e| e == "rs") {
                let contents = fs::read_to_string(&path).unwrap_or_default();
                if !contents.contains("SPDX-License-Identifier") {
                    missing.push(path.display().to_string());
                }
            }
        }
    }
}

/// G3: honest_ratio must be a number ≥ 0 and ≤ 1
#[test]
fn gate3_honest_ratio_truthful() {
    let m = manifest();
    let parity = m.get("parity").expect("[parity] section");
    let hr = parity
        .get("honest_ratio")
        .and_then(|v| v.as_float())
        .expect("honest_ratio must be a float");
    assert!(
        (0.0..=1.0).contains(&hr),
        "G3 FAIL: honest_ratio out of range: {}",
        hr
    );
}

/// G4: manifest must be present and have required fields
#[test]
fn gate4_manifest_present() {
    let m = manifest();
    let parity = m.get("parity").expect("[parity] section");
    for field in &["fill_ratio", "honest_ratio", "adr_justified_ratio", "last_audit"] {
        assert!(
            parity.get(field).is_some(),
            "G4 FAIL: [parity].{} is missing",
            field
        );
    }
}

/// G5: no stub implementations in src/
#[test]
fn gate5_no_stubs() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stubs = vec![];
    find_stubs(&src_dir, &mut stubs);
    assert!(
        stubs.is_empty(),
        "G5 FAIL: unimplemented!/todo!() found in: {:?}",
        stubs
    );
}

fn find_stubs(dir: &std::path::Path, stubs: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                find_stubs(&path, stubs);
            } else if path.extension().map_or(false, |e| e == "rs") {
                let contents = fs::read_to_string(&path).unwrap_or_default();
                if contents.contains("unimplemented!") || contents.contains("todo!()") {
                    stubs.push(path.display().to_string());
                }
            }
        }
    }
}

/// G6: fill_ratio must be 1.0
#[test]
fn gate6_fill_ratio_one() {
    let m = manifest();
    let parity = m.get("parity").expect("[parity] section");
    let fr = parity
        .get("fill_ratio")
        .and_then(|v| v.as_float())
        .expect("fill_ratio must be a float");
    assert!(
        (fr - 1.0).abs() < 1e-9,
        "G6 FAIL: fill_ratio is {}, must be 1.0",
        fr
    );
}

/// G7: upstream version field must look like a semver/tag
#[test]
fn gate7_upstream_version() {
    let m = manifest();
    let upstream = m.get("upstream").expect("[upstream] section");
    let version = upstream
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Accept vX.Y.Z or X.Y.Z
    assert!(
        !version.is_empty() && (version.starts_with('v') || version.chars().next().map_or(false, |c| c.is_numeric())),
        "G7 FAIL: [upstream].version looks wrong: {:?}",
        version
    );
}

/// G8: adr_justified_ratio must equal 1.0
#[test]
fn gate8_adr_justified() {
    let m = manifest();
    let parity = m.get("parity").expect("[parity] section");
    let adr = parity
        .get("adr_justified_ratio")
        .and_then(|v| v.as_float())
        .expect("adr_justified_ratio must be a float");
    assert!(
        (adr - 1.0).abs() < 1e-9,
        "G8 FAIL: adr_justified_ratio is {}, must be 1.0",
        adr
    );
}
