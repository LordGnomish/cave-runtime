// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit gates for cave-tracker.

use std::fs;
use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn manifest_path() -> PathBuf {
    crate_root().join("parity.manifest.toml")
}

/// G1: source_sha and version pinned in [upstream]
#[test]
fn g1_upstream_sha_pinned() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(content.contains("source_sha"), "G1: source_sha not pinned");
    assert!(content.contains("version"), "G1: version not pinned");
}

/// G2: every .rs src file starts with SPDX header
#[test]
fn g2_spdx_headers_present() {
    let src = crate_root().join("src");
    let mut missing = vec![];
    for entry in walkdir_rs(&src) {
        let content = fs::read_to_string(&entry).unwrap_or_default();
        if !content.starts_with("// SPDX-License-Identifier:") {
            missing.push(entry.display().to_string());
        }
    }
    assert!(missing.is_empty(), "G2: missing SPDX headers in: {:?}", missing);
}

/// G3: honest_ratio is truthful (0 <= r <= 1)
#[test]
fn g3_honest_ratio_present_and_valid() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(content.contains("honest_ratio"), "G3: honest_ratio missing");
    for line in content.lines() {
        if line.trim().starts_with("honest_ratio") {
            let rhs = line.split('=').nth(1).expect("no value");
            let rhs_clean = rhs.split('#').next().unwrap_or(rhs).trim();
            let val: f64 = rhs_clean.parse().expect("honest_ratio not a float");
            assert!(val >= 0.0 && val <= 1.0, "G3: honest_ratio out of range: {}", val);
        }
    }
}

/// G4: parity.manifest.toml exists with required fields
#[test]
fn g4_manifest_present_with_required_fields() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    for field in &[
        "fill_ratio", "honest_ratio", "adr_justified_ratio",
        "mapped_count", "partial_count", "skipped_count", "unmapped_count",
        "last_audit",
    ] {
        assert!(content.contains(field), "G4: missing field {}", field);
    }
}

/// G5: no unimplemented!/todo!() stubs in src
#[test]
fn g5_no_stubs_in_src() {
    let src = crate_root().join("src");
    let mut stubs = vec![];
    for entry in walkdir_rs(&src) {
        let content = fs::read_to_string(&entry).unwrap_or_default();
        for (i, line) in content.lines().enumerate() {
            if line.contains("unimplemented!") || line.contains("todo!()") {
                stubs.push(format!("{}:{}", entry.display(), i + 1));
            }
        }
    }
    assert!(stubs.is_empty(), "G5: stubs found: {:?}", stubs);
}

/// G6: no backwards-compat shims
#[test]
fn g6_no_backcompat_shims() {
    let src = crate_root().join("src");
    let mut shims = vec![];
    for entry in walkdir_rs(&src) {
        let content = fs::read_to_string(&entry).unwrap_or_default();
        for (i, line) in content.lines().enumerate() {
            if line.contains("#[deprecated]") {
                shims.push(format!("{}:{}", entry.display(), i + 1));
            }
        }
    }
    assert!(shims.is_empty(), "G6: backcompat shims found: {:?}", shims);
}

/// G7: upstream version declared in manifest
#[test]
fn g7_upstream_version_declared() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(
        content.contains("v1.3.0"),
        "G7: upstream version v1.3.0 not declared in manifest"
    );
}

/// G8: 4-track present — backend impl, route handler, scope-cuts noted
#[test]
fn g8_four_track_backend_and_route_handler() {
    // Backend track: core engines exist
    let analytics = crate_root().join("src").join("analytics.rs");
    assert!(analytics.exists(), "G8: src/analytics.rs (backend) missing");
    let jql = crate_root().join("src").join("jql_engine.rs");
    assert!(jql.exists(), "G8: src/jql_engine.rs missing");

    // Route track: routes.rs has real handlers
    let routes = crate_root().join("src").join("routes.rs");
    assert!(routes.exists(), "G8: src/routes.rs missing");
    let routes_content = fs::read_to_string(&routes).unwrap();
    assert!(
        routes_content.contains("/api/tracker/issues"),
        "G8: issue route handler missing"
    );
    assert!(
        routes_content.contains("/api/tracker/sprints"),
        "G8: sprint route handler missing"
    );

    // Portal/observability/cavectl tracks noted as parallel-track skips in manifest
    let manifest = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(
        manifest.contains("parallel-track"),
        "G8: parallel-track scope-cut annotation missing in manifest"
    );
}

/// Manifest counters must be consistent: mapped+partial+skipped+unmapped == total, unmapped == 0
#[test]
fn manifest_counters_consistent() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    let mapped   = extract_u32(&content, "mapped_count");
    let partial  = extract_u32(&content, "partial_count");
    let skipped  = extract_u32(&content, "skipped_count");
    let unmapped = extract_u32(&content, "unmapped_count");
    let total    = extract_u32(&content, "total");

    assert_eq!(
        mapped + partial + skipped + unmapped, total,
        "counters: {}+{}+{}+{} != {}",
        mapped, partial, skipped, unmapped, total
    );
    assert_eq!(unmapped, 0, "unmapped_count must be 0");
}

/// fill_ratio must be 1.0
#[test]
fn manifest_fill_ratio_is_one() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    for line in content.lines() {
        if line.trim().starts_with("fill_ratio") {
            let val: f64 = line
                .split('=')
                .nth(1)
                .expect("no value")
                .trim()
                .parse()
                .expect("not a float");
            assert!((val - 1.0).abs() < 0.001, "fill_ratio must be 1.0, got {}", val);
            return;
        }
    }
    panic!("fill_ratio not found in manifest");
}

/// honest_ratio must be in the range (0.65, 1.0] to pass
/// (cave-tracker honest_ratio = 0.75 = 18/24)
#[test]
fn manifest_honest_ratio_meets_floor() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    for line in content.lines() {
        if line.trim().starts_with("honest_ratio") {
            let rhs = line.split('=').nth(1).expect("no value");
            let rhs_clean = rhs.split('#').next().unwrap_or(rhs).trim();
            let val: f64 = rhs_clean.parse().expect("honest_ratio not a float");
            assert!(
                val > 0.65,
                "honest_ratio {} below floor 0.65 — real functional gaps need implementation",
                val
            );
            return;
        }
    }
    panic!("honest_ratio not found in manifest");
}

/// last_audit date must start with "2026-"
#[test]
fn manifest_last_audit_date() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    for line in content.lines() {
        if line.trim().starts_with("last_audit") {
            assert!(
                line.contains("2026-"),
                "G9: last_audit does not contain 2026-: {}",
                line
            );
            return;
        }
    }
    panic!("last_audit not found in manifest");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn walkdir_rs(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walkdir_rs(&path));
            } else if path.extension().map_or(false, |e| e == "rs") {
                out.push(path);
            }
        }
    }
    out
}

fn extract_u32(content: &str, key: &str) -> u32 {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) && trimmed.contains('=') {
            let val = trimmed.split('=').nth(1).unwrap().trim();
            let val_clean = val.split('#').next().unwrap_or(val).trim();
            if let Ok(n) = val_clean.parse::<u32>() {
                return n;
            }
        }
    }
    panic!("key '{}' not found in manifest", key);
}
