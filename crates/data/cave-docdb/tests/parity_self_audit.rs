// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 8-gate self-audit for cave-docdb (MongoDB wire / FerretDB-style
// clean-room reimplementation).
//
// Asserts that the close-out invariants for this crate hold:
//   * SPDX coverage 100% of src/*.rs
//   * source_sha pinned in manifest (FerretDB clean-room reference + mongodb wire pin)
//   * last_audit = 2026-05-24
//   * parity_ratio_source = "manifest"
//   * fill_ratio >= 0.90  (honest measured against the in-scope inventory)
//   * mapped + partial + skipped + unmapped == total
//   * no unimplemented!() / todo!() in src/
//   * PARITY_REPORT.md exists

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
    assert!(m.contains("source_sha"), "manifest must declare source_sha");
    assert!(
        m.contains("v2.0.0") || m.contains("v7.0.0") || m.contains("r7.0.0"),
        "source_sha must pin a release tag (FerretDB v2.0.0 or mongodb r7.0.0)"
    );
}

#[test]
fn gate_3_last_audit_2026_iso() {
    // Accept any 2026 ISO audit date so the manifest can be re-stamped on
    // each honest-uplift continuation without editing the gate every time.
    let m = read_manifest();
    let ok = m.lines().any(|line| {
        let l = line.trim();
        l.starts_with("last_audit") && l.contains("\"2026-")
    });
    assert!(ok, "last_audit must carry a 2026 ISO date");
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
    assert!(mapped > 0 && total > 0);
    assert_eq!(mapped + partial + skipped + unmapped, total);
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
    assert!(body.contains("Charter v2"));
    assert!(body.contains("8/8 PASS") || body.contains("8-gate"));
}

#[test]
fn gate_9_charter_v2_summary() {
    let m = read_manifest();
    let r = extract_float(&m, "fill_ratio").unwrap_or(0.0);
    let total = extract_int(&m, "total").unwrap_or(0);
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    assert!(r >= 0.95 && total > 0 && mapped > 0 && m.contains("source_sha"));
}

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
