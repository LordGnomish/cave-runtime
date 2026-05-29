// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit gates for cave-devlake.

use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // pop crates/ops/cave-devlake → workspace root
    p.pop(); p.pop(); p.pop();
    p
}

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

/// G2: every .rs file starts with SPDX header
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
    assert!(missing.is_empty(), "G2: missing SPDX headers in: {missing:?}");
}

/// G3: honest_ratio is truthful (non-negative, <= 1.0, present in manifest)
#[test]
fn g3_honest_ratio_present_and_valid() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(content.contains("honest_ratio"), "G3: honest_ratio missing");
    // Extract and validate value (strip inline TOML comments)
    for line in content.lines() {
        if line.trim().starts_with("honest_ratio") {
            let rhs = line.split('=').nth(1).expect("no value");
            // strip inline comment (everything after '#')
            let rhs_clean = rhs.split('#').next().unwrap_or(rhs).trim();
            let val: f64 = rhs_clean.parse().expect("honest_ratio not a float");
            assert!(val >= 0.0 && val <= 1.0, "G3: honest_ratio out of range: {val}");
        }
    }
}

/// G4: parity.manifest.toml exists with required fields
#[test]
fn g4_manifest_present_with_required_fields() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    for field in &["fill_ratio", "honest_ratio", "adr_justified_ratio", "mapped_count",
                   "skipped_count", "unmapped_count", "last_audit"] {
        assert!(content.contains(field), "G4: missing field {field}");
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
    assert!(stubs.is_empty(), "G5: stubs found: {stubs:?}");
}

/// G6: no backwards-compat shims (no #[deprecated] on pub items)
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
    assert!(shims.is_empty(), "G6: backcompat shims found: {shims:?}");
}

/// G7: upstream version is declared (we can't network-check in tests)
#[test]
fn g7_upstream_version_declared() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(
        content.contains("v0.21"),
        "G7: upstream version v0.21.x not declared in manifest"
    );
}

/// G8: 4-track present — backend (src/), route handler (routes.rs), parity comment notes portal/observability skips
#[test]
fn g8_four_track_backend_and_route_handler() {
    // Backend track: src/engine.rs exists with real logic
    let engine = crate_root().join("src").join("engine.rs");
    assert!(engine.exists(), "G8: src/engine.rs (backend) missing");
    let engine_content = fs::read_to_string(&engine).unwrap();
    assert!(
        engine_content.contains("pub fn dora_deployment_frequency_rating"),
        "G8: DORA engine function missing"
    );

    // Route track: routes.rs has real handlers
    let routes = crate_root().join("src").join("routes.rs");
    assert!(routes.exists(), "G8: src/routes.rs missing");
    let routes_content = fs::read_to_string(&routes).unwrap();
    assert!(
        routes_content.contains("/api/devlake/dora"),
        "G8: DORA route handler missing"
    );

    // Portal/observability tracks noted as parallel-track skips in manifest
    let manifest = fs::read_to_string(manifest_path()).expect("manifest missing");
    assert!(
        manifest.contains("parallel-track"),
        "G8: parallel-track scope-cut annotation missing in manifest"
    );
}

// ── Manifest counters consistent ──────────────────────────────────────────────

#[test]
fn manifest_counters_consistent() {
    let content = fs::read_to_string(manifest_path()).expect("manifest missing");
    let mapped: u32 = extract_u32(&content, "mapped_count");
    let partial: u32 = extract_u32(&content, "partial_count");
    let skipped: u32 = extract_u32(&content, "skipped_count");
    let unmapped: u32 = extract_u32(&content, "unmapped_count");
    let total: u32 = extract_u32(&content, "total");

    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "manifest counters inconsistent: {mapped}+{partial}+{skipped}+{unmapped} != {total}"
    );
    assert_eq!(unmapped, 0, "unmapped_count must be 0");
}

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
            assert!((val - 1.0).abs() < 0.001, "fill_ratio must be 1.0, got {val}");
            return;
        }
    }
    panic!("fill_ratio not found in manifest");
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
            let val = trimmed.split('=').nth(1).unwrap().trim().trim_matches('#');
            if let Ok(n) = val.trim().parse::<u32>() {
                return n;
            }
        }
    }
    panic!("key '{key}' not found in manifest");
}
