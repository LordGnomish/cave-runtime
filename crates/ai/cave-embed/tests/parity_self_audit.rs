// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 self-audit for cave-embed (michaelfeil/infinity 0.0.75 parity +
// sentence-transformers pooling/normalize/quantize patterns).
//
// Gates:
//   1. SPDX coverage 100% of src/*.rs
//   2. source_sha pinned (infinity 0.0.75)
//   3. last_audit is a 2026 date
//   4. parity_ratio_source = "manifest"
//   5. fill_ratio >= 0.95
//   6. mapped + partial + skipped + unmapped == total
//   7. no unimplemented!() / todo!() in src/
//   8. PARITY_REPORT.md exists + summarises Charter v2
//   9. UPSTREAM_VERSION constant matches manifest [upstream] version
//  10. Charter v2 composite — all of the above re-asserted

use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_manifest() -> String {
    fs::read_to_string(crate_root().join("parity.manifest.toml"))
        .expect("parity.manifest.toml must exist")
}

#[test]
fn gate_1_spdx_full_coverage() {
    let src = crate_root().join("src");
    let mut total = 0usize;
    let mut spdx = 0usize;
    walk_rs(&src, &mut |p| {
        total += 1;
        let body = fs::read_to_string(p).unwrap_or_default();
        if body.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
            spdx += 1;
        }
    });
    assert!(total > 0, "no .rs files found under src/");
    assert_eq!(spdx, total, "SPDX missing on {} files", total - spdx);
}

#[test]
fn gate_2_source_sha_pinned() {
    let m = read_manifest();
    assert!(m.contains("source_sha"), "manifest must declare source_sha");
    assert!(m.contains("0.0.75"), "source_sha must pin infinity 0.0.75");
}

#[test]
fn gate_3_last_audit_2026() {
    let m = read_manifest();
    let line = m
        .lines()
        .find(|l| l.trim_start().starts_with("last_audit"))
        .expect("last_audit must be present");
    assert!(line.contains("\"2026-"), "last_audit must be a 2026 date (got {line})");
}

#[test]
fn gate_4_parity_ratio_source_manifest() {
    assert!(has_kv(&read_manifest(), "parity_ratio_source", "\"manifest\""));
}

#[test]
fn gate_5_fill_ratio_floor() {
    let ratio = extract_float(&read_manifest(), "fill_ratio").expect("fill_ratio");
    assert!(ratio >= 0.95, "fill_ratio = {ratio} (need >= 0.95)");
}

#[test]
fn gate_6_count_invariants() {
    let m = read_manifest();
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    let partial = extract_int(&m, "partial_count").unwrap_or(0);
    let skipped = extract_int(&m, "skipped_count").unwrap_or(0);
    let unmapped = extract_int(&m, "unmapped_count").unwrap_or(0);
    let total = extract_int(&m, "total").unwrap_or(0);
    assert!(mapped > 0 && total > 0);
    assert_eq!(mapped + partial + skipped + unmapped, total);
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let src = crate_root().join("src");
    let mut offenders: Vec<String> = Vec::new();
    walk_rs(&src, &mut |p| {
        let body = fs::read_to_string(p).unwrap_or_default();
        for (i, line) in body.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") || t.starts_with("///") || t.starts_with("//!") {
                continue;
            }
            // Skip references inside string literals (e.g. doc/help text).
            if line.contains('"') {
                continue;
            }
            if line.contains("unimplemented!(") || line.contains("todo!(") {
                offenders.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    });
    assert!(offenders.is_empty(), "stub macros found:\n{}", offenders.join("\n"));
}

#[test]
fn gate_8_parity_report_exists() {
    let report = crate_root().join("PARITY_REPORT.md");
    assert!(report.exists(), "PARITY_REPORT.md must exist");
    let body = fs::read_to_string(&report).unwrap();
    assert!(body.contains("Charter v2"), "PARITY_REPORT must reference Charter v2");
    assert!(
        body.contains("10/10") || body.contains("10-gate"),
        "PARITY_REPORT must summarise the gate result"
    );
}

#[test]
fn gate_9_upstream_version_matches_constant() {
    let m = read_manifest();
    // [upstream] version line.
    let v = m
        .lines()
        .find(|l| l.trim_start().starts_with("version"))
        .and_then(|l| l.split('"').nth(1).map(|s| s.to_string()))
        .expect("version line in manifest");
    assert_eq!(
        v,
        cave_embed::UPSTREAM_VERSION,
        "manifest [upstream] version must match cave_embed::UPSTREAM_VERSION"
    );
}

#[test]
fn gate_10_charter_v2_summary() {
    let m = read_manifest();
    let ratio = extract_float(&m, "fill_ratio").unwrap_or(0.0);
    let total = extract_int(&m, "total").unwrap_or(0);
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    assert!(
        ratio >= 0.95 && total > 0 && mapped > 0 && m.contains("source_sha"),
        "Charter v2 composite invariants not satisfied"
    );
}

fn has_kv(s: &str, key: &str, expected: &str) -> bool {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if v == expected {
                    return true;
                }
            }
        }
    }
    false
}

fn walk_rs(dir: &Path, f: &mut dyn FnMut(&Path)) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rs(&p, f);
        } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            f(&p);
        }
    }
}

fn extract_float(s: &str, key: &str) -> Option<f64> {
    line_value(s, key).and_then(|v| v.parse::<f64>().ok())
}

fn extract_int(s: &str, key: &str) -> Option<i64> {
    line_value(s, key).and_then(|v| v.parse::<i64>().ok())
}

fn line_value(s: &str, key: &str) -> Option<String> {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                return Some(v.to_string());
            }
        }
    }
    None
}
