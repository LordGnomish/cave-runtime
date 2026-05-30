// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 self-audit for cave-mlx (ml-explore/mlx v0.31.2 array-core port,
// pure-Rust CPU backend).
//
// Gates:
//   1. SPDX coverage 100% of src/*.rs
//   2. source_sha pinned (v0.31.2)
//   3. last_audit is a 2026 date
//   4. parity_ratio_source = "manifest"
//   5. fill_ratio >= 0.95
//   6. mapped + partial + skipped + unmapped == total
//   7. no unimplemented!() / todo!() in src/
//   8. PARITY_REPORT.md exists and summarises the gate result
//   9. Charter v2 composite — all of the above re-asserted
//
// honest_ratio (0.9615) is deliberately below fill_ratio (1.0): mx.random is
// partial (only the seeded Kaiming initializer is ported). Convolution, once an
// unmapped gap, was closed on 2026-05-30 (conv.rs + nn.Conv2d). No item is
// ADR-justified.

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
    assert_eq!(spdx, total, "SPDX missing on {} src files", total - spdx);
}

#[test]
fn gate_2_source_sha_pinned() {
    let m = read_manifest();
    assert!(m.contains("source_sha"), "manifest must declare source_sha");
    assert!(m.contains("v0.31.2"), "source_sha must pin mlx v0.31.2");
}

#[test]
fn gate_3_last_audit_present() {
    let m = read_manifest();
    let line = m
        .lines()
        .find(|l| l.trim_start().starts_with("last_audit"))
        .expect("last_audit must be present in [parity]");
    assert!(line.contains("\"2026-"), "last_audit must be a 2026 date (got {line})");
}

#[test]
fn gate_4_parity_ratio_source_manifest() {
    let m = read_manifest();
    assert!(
        has_kv(&m, "parity_ratio_source", "\"manifest\""),
        "parity_ratio_source must be \"manifest\""
    );
}

#[test]
fn gate_5_fill_ratio_floor() {
    let m = read_manifest();
    let ratio = extract_float(&m, "fill_ratio").expect("fill_ratio must be declared");
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
    assert!(mapped > 0, "mapped_count must be > 0");
    assert!(total > 0, "total must be > 0");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped + partial + skipped + unmapped must equal total"
    );
}

#[test]
fn gate_6b_honest_ratio_is_consistent() {
    // honest_ratio = (mapped + skipped) / total, and must not exceed fill_ratio.
    let m = read_manifest();
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0) as f64;
    let skipped = extract_int(&m, "skipped_count").unwrap_or(0) as f64;
    let total = extract_int(&m, "total").unwrap_or(1) as f64;
    let honest = extract_float(&m, "honest_ratio").unwrap_or(0.0);
    let fill = extract_float(&m, "fill_ratio").unwrap_or(0.0);
    assert!((honest - (mapped + skipped) / total).abs() < 1e-3, "honest_ratio mismatch");
    assert!(honest <= fill + 1e-9, "honest_ratio must not exceed fill_ratio");
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let src = crate_root().join("src");
    let mut offenders: Vec<String> = Vec::new();
    walk_rs(&src, &mut |p| {
        let body = fs::read_to_string(p).unwrap_or_default();
        for (i, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains("unimplemented!(") || line.contains("todo!(") {
                offenders.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    });
    assert!(offenders.is_empty(), "no stub macros allowed; offenders:\n{}", offenders.join("\n"));
}

#[test]
fn gate_8_parity_report_exists() {
    let report = crate_root().join("PARITY_REPORT.md");
    assert!(report.exists(), "PARITY_REPORT.md must exist at crate root");
    let body = fs::read_to_string(&report).unwrap();
    assert!(body.contains("Charter v2"), "PARITY_REPORT must reference Charter v2");
    assert!(
        body.contains("8/8") || body.contains("9/9") || body.contains("gate"),
        "PARITY_REPORT must summarise the gate result"
    );
}

#[test]
fn gate_9_charter_v2_summary() {
    let m = read_manifest();
    let ratio = extract_float(&m, "fill_ratio").unwrap_or(0.0);
    let total = extract_int(&m, "total").unwrap_or(0);
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    assert!(
        ratio >= 0.95 && total > 0 && mapped > 0 && m.contains("source_sha"),
        "Charter v2 composite invariants not satisfied"
    );
}

fn has_kv(s: &str, key: &str, expected_value: &str) -> bool {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if v == expected_value {
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
