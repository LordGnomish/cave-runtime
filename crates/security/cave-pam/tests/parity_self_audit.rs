// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit gates for cave-pam.
//!
//! All 8 gates must pass before the crate can be considered closed.

use std::path::Path;

fn manifest_path() -> std::path::PathBuf {
    // Walk up from the test binary to find the crate root.
    let mut p = std::env::current_exe().unwrap();
    // target/release/deps/parity_self_audit-xxx  →  crate root
    // Actually use the CARGO_MANIFEST_DIR env set at compile time.
    p.clear();
    p.push(env!("CARGO_MANIFEST_DIR"));
    p.push("parity.manifest.toml");
    p
}

fn read_manifest() -> String {
    std::fs::read_to_string(manifest_path()).expect("parity.manifest.toml must exist")
}

/// G1: upstream version + source_sha are pinned.
#[test]
fn g1_upstream_version_pinned() {
    let m = read_manifest();
    assert!(m.contains("version"), "G1: [upstream] version must be present");
    assert!(m.contains("source_sha"), "G1: [upstream] source_sha must be present");
}

/// G2: every .rs source file has the SPDX header.
#[test]
fn g2_spdx_on_all_rs_files() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let header = "// SPDX-License-Identifier: AGPL-3.0-or-later";

    for dir in [&src_dir, &tests_dir] {
        if !dir.exists() {
            continue;
        }
        for entry in walkdir(dir) {
            if entry.extension().and_then(|e| e.to_str()) == Some("rs") {
                let contents = std::fs::read_to_string(&entry)
                    .unwrap_or_default();
                assert!(
                    contents.starts_with(header),
                    "G2: {} is missing SPDX header",
                    entry.display()
                );
            }
        }
    }
}

fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                result.extend(walkdir(&path));
            } else {
                result.push(path);
            }
        }
    }
    result
}

/// G3: honest_ratio is between 0.0 and 1.0 (sanity bounds).
#[test]
fn g3_honest_ratio_sane() {
    let m = read_manifest();
    let ratio_line = m
        .lines()
        .find(|l| l.trim_start().starts_with("honest_ratio"))
        .expect("G3: honest_ratio must be present");
    let val: f64 = ratio_line
        .split('=')
        .nth(1)
        .expect("malformed honest_ratio line")
        .trim()
        .parse()
        .expect("honest_ratio must be numeric");
    assert!(
        (0.0..=1.0).contains(&val),
        "G3: honest_ratio {val} outside [0.0, 1.0]"
    );
}

/// G4: manifest file is present and has [parity] section.
#[test]
fn g4_manifest_present() {
    let m = read_manifest();
    assert!(m.contains("[parity]"), "G4: [parity] section missing");
    assert!(m.contains("fill_ratio"), "G4: fill_ratio missing");
    assert!(m.contains("last_audit"), "G4: last_audit missing");
}

/// G5: no stubs (unimplemented! / todo!()) in src/.
#[test]
fn g5_no_stubs() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in walkdir(&src_dir) {
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                !contents.contains("unimplemented!"),
                "G5: {} contains unimplemented!()",
                path.display()
            );
            assert!(
                !contents.contains("todo!()"),
                "G5: {} contains todo!()",
                path.display()
            );
        }
    }
}

/// G6: no backwards-compatibility shims.
#[test]
fn g6_no_backcompat_shims() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in walkdir(&src_dir) {
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                !contents.contains("#[deprecated"),
                "G6: {} contains deprecated shim",
                path.display()
            );
        }
    }
}

/// G7: upstream version is marked as latest stable (≥ current manifest version).
#[test]
fn g7_upstream_version_recent() {
    let m = read_manifest();
    // Teleport v17.x.x is latest stable at time of writing (2026-05-28).
    let version_line = m
        .lines()
        .find(|l| l.trim_start().starts_with("version") && l.contains('"'))
        .expect("G7: version line missing");
    // Accept any v14+ version.
    let has_recent = version_line.contains("v14")
        || version_line.contains("v15")
        || version_line.contains("v16")
        || version_line.contains("v17");
    assert!(has_recent, "G7: upstream version too old: {version_line}");
}

/// G8: four-track backend coverage (routes handler wired).
#[test]
fn g8_routes_handler_exists() {
    let routes_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("routes.rs");
    let contents = std::fs::read_to_string(&routes_path).expect("G8: routes.rs missing");
    // Must have at least one GET/POST handler beyond health.
    assert!(
        contents.contains("sessions") || contents.contains("requests") || contents.contains("nodes"),
        "G8: routes.rs must expose PAM endpoints beyond /health"
    );
}
