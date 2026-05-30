// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 8-gate self-audit for cave-kube-proxy (kubernetes/kubernetes
// pkg/proxy parity).
//
// Gates:
//   1. SPDX coverage 100% of src/*.rs
//   2. source_sha pinned (kubernetes v1.36.0 release tag)
//   3. last_audit = 2026-05-21
//   4. parity_ratio_source = "manifest"
//   5. fill_ratio >= 0.95
//   6. mapped + partial + skipped + unmapped == total
//   7. no unimplemented!() / todo!() in src/
//   8. PARITY_REPORT.md exists
//   9. Charter v2 composite — all of the above re-asserted

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
    assert_eq!(
        spdx,
        total,
        "SPDX-License-Identifier missing on {} files",
        total - spdx
    );
}

#[test]
fn gate_2_source_sha_pinned() {
    let m = read_manifest();
    assert!(
        m.contains("source_sha"),
        "manifest must declare source_sha pinning kubernetes/kubernetes release"
    );
    assert!(
        m.contains("v1.36.0"),
        "source_sha must pin kubernetes v1.36.0"
    );
}

#[test]
fn gate_3_last_audit_2026_05_30() {
    let m = read_manifest();
    assert!(
        has_kv(&m, "last_audit", "\"2026-05-30\""),
        "last_audit must be 2026-05-30 in [parity] block (bumped by honest_ratio uplift ray)"
    );
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
    let ratio =
        extract_float(&m, "fill_ratio").expect("manifest must declare fill_ratio = <0.0..1.0>");
    assert!(
        ratio >= 0.95,
        "fill_ratio = {} (need >= 0.95 — kube-proxy in-scope coverage)",
        ratio
    );
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
    assert!(
        offenders.is_empty(),
        "no stub macros allowed; offenders:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn gate_8_parity_report_exists() {
    let report = crate_root().join("PARITY_REPORT.md");
    assert!(report.exists(), "PARITY_REPORT.md must exist at crate root");
    let body = fs::read_to_string(&report).unwrap();
    assert!(
        body.contains("Charter v2"),
        "PARITY_REPORT must reference Charter v2"
    );
    assert!(
        body.contains("8/8 PASS") || body.contains("8-gate"),
        "PARITY_REPORT must summarise 8-gate result"
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
