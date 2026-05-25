// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 8-gate self-audit for cave-sandbox.
// Triumvirate deep-port: gVisor + kata-containers + firecracker.

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
    assert!(total > 0, "no .rs files under src/");
    assert_eq!(
        spdx, total,
        "SPDX missing on {} files",
        total - spdx
    );
}

#[test]
fn gate_2_source_sha_pinned_three_upstreams() {
    let m = read_manifest();
    // gVisor pin:
    assert!(
        m.contains("d8751e5ab6770060517e3cd00617820b6b8663a6"),
        "gVisor source_sha must be pinned"
    );
    // kata pin:
    assert!(
        m.contains("cec98e0d976bbf4cae016298ffea269f57294264"),
        "kata source_sha must be pinned"
    );
    // firecracker pin:
    assert!(
        m.contains("f82c0bd0f0a74015642a0d452880f3ad10147b14"),
        "firecracker source_sha must be pinned"
    );
}

#[test]
fn gate_3_last_audit_today() {
    let m = read_manifest();
    assert!(
        has_kv(&m, "last_audit", "\"2026-05-24\""),
        "last_audit must be 2026-05-24 (re-stamped by ADR-RUNTIME-SANDBOX-NO-FFI-001)"
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
fn gate_5_fill_ratio_floor_0_95() {
    let m = read_manifest();
    let ratio = extract_float(&m, "fill_ratio")
        .expect("manifest must declare fill_ratio");
    assert!(
        ratio >= 0.95,
        "fill_ratio = {} (need >= 0.95 — Charter v2 deep-port floor)",
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
    assert!(mapped >= 40, "mapped_count must be >= 40 (got {})", mapped);
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
        let mut in_raw = false;
        for (i, line) in body.lines().enumerate() {
            // crude raw-string-aware skipper: toggle on a line that contains `r#"`
            // (good enough for our hand-rolled sources; no nested raw strings).
            if line.contains("r#\"") {
                in_raw = !in_raw;
            }
            if in_raw {
                continue;
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for needle in ["unimplemented!(", "todo!(", "panic!("] {
                if line.contains(needle) {
                    offenders.push(format!("{}:{} [{}]", p.display(), i + 1, needle));
                }
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
    // observability config check (G6 supplemental).
    let obs = crate_root().join("observability.toml");
    assert!(obs.exists(), "observability.toml must exist at crate root");
    let obody = fs::read_to_string(&obs).unwrap();
    assert!(obody.matches("[[panel]]").count() >= 8, "must have >= 8 panels");
    assert!(obody.matches("[[alert]]").count() >= 5, "must have >= 5 alerts");
}

#[test]
fn gate_9_charter_v2_composite() {
    let m = read_manifest();
    let ratio = extract_float(&m, "fill_ratio").unwrap_or(0.0);
    let total = extract_int(&m, "total").unwrap_or(0);
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    assert!(
        ratio >= 0.95
            && total > 0
            && mapped >= 40
            && m.contains("d8751e5ab6770060517e3cd00617820b6b8663a6")
            && m.contains("cec98e0d976bbf4cae016298ffea269f57294264")
            && m.contains("f82c0bd0f0a74015642a0d452880f3ad10147b14"),
        "Charter v2 composite invariants not satisfied"
    );
}

// ── helpers ────────────────────────────────────────────────────────────────

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
