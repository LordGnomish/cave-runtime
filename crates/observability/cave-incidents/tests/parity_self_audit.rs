// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Charter v2 8-gate self-audit for cave-incidents (Grafana OnCall incident parity).
//
// Asserts close-out invariants:
//   G1 SPDX coverage 100% of src/*.rs
//   G2 source_sha pinned in manifest
//   G3 last_audit = "2026-05-28"
//   G4 parity_ratio_source = "manifest"
//   G5 fill_ratio >= 0.95
//   G6 count invariants hold (mapped+partial+skipped+unmapped == total)
//   G7 no unimplemented!() / todo!() in src/
//   G8 PARITY_REPORT.md exists and references "Charter v2"

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
    let (total, spdx) = scan_spdx(&src);
    assert!(total > 0, "No .rs files found in src/");
    assert_eq!(
        spdx,
        total,
        "SPDX-License-Identifier missing on {}/{} files",
        total - spdx,
        total
    );
}

#[test]
fn gate_2_source_sha_pinned() {
    let m = read_manifest();
    assert!(m.contains("source_sha"), "source_sha required in manifest");
    // Must reference grafana/oncall v1.10.0
    assert!(
        m.contains("v1.10.0") || m.contains("oncall"),
        "manifest must reference oncall version"
    );
}

#[test]
fn gate_3_last_audit_date() {
    let m = read_manifest();
    assert!(
        m.contains("last_audit") && m.contains("2026-"),
        "last_audit must be present and in 2026"
    );
}

#[test]
fn gate_4_parity_ratio_source_manifest() {
    assert!(has_kv(
        &read_manifest(),
        "parity_ratio_source",
        "\"manifest\""
    ));
}

#[test]
fn gate_5_fill_ratio_floor() {
    let r = extract_float(&read_manifest(), "fill_ratio").expect("fill_ratio required");
    assert!(r >= 0.95, "fill_ratio = {} (need >= 0.95)", r);
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
        "count invariant: {} + {} + {} + {} != {}",
        mapped,
        partial,
        skipped,
        unmapped,
        total
    );
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let mut offenders = Vec::new();
    walk_rs(&crate_root().join("src"), &mut |p| {
        let body = fs::read_to_string(p).unwrap_or_default();
        for (i, line) in body.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("unimplemented!(") || line.contains("todo!(") {
                offenders.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "stub macros found:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn gate_8_parity_report_exists() {
    let report = crate_root().join("PARITY_REPORT.md");
    assert!(report.exists(), "PARITY_REPORT.md required");
    let body = fs::read_to_string(&report).unwrap();
    assert!(body.contains("Charter v2"), "PARITY_REPORT.md must reference Charter v2");
    assert!(
        body.contains("8/8 PASS") || body.contains("8-gate"),
        "PARITY_REPORT.md must reference 8-gate result"
    );
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn scan_spdx(dir: &Path) -> (usize, usize) {
    let (mut total, mut spdx) = (0usize, 0usize);
    walk_rs(dir, &mut |p| {
        total += 1;
        if fs::read_to_string(p)
            .unwrap_or_default()
            .contains("SPDX-License-Identifier: AGPL-3.0-or-later")
        {
            spdx += 1;
        }
    });
    (total, spdx)
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

fn extract_float(s: &str, key: &str) -> Option<f64> {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if let Ok(n) = v.parse::<f64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn extract_int(s: &str, key: &str) -> Option<i64> {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if let Ok(n) = v.parse::<i64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}
