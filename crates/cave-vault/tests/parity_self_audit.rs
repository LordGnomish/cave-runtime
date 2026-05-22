// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit for cave-vault — pinned to openbao/openbao v2.5.3.

use std::fs;
use std::path::PathBuf;

fn manifest_text() -> String {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"]
        .iter()
        .collect();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn extract_after(text: &str, needle: &str) -> Option<String> {
    let i = text.find(needle)?;
    let rest = &text[i + needle.len()..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..line_end];
    let trimmed = line.trim();
    let stripped = trimmed.trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

#[test]
fn gate_1_upstream_version_pinned_v2_5_3() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("v2.5.3"),
        "manifest [upstream] version must pin OpenBao v2.5.3 (was {:?}).",
        v
    );
}

#[test]
fn gate_2_source_sha_present_and_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "manifest [upstream] source_sha must be set (got {:?})",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some("v2.5.3"),
        "source_sha must match the pinned upstream version (got {:?})",
        sha
    );
}

#[test]
fn gate_3_fill_ratio_is_measured_and_at_least_0_95() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.95,
        "cave-vault Charter v2 floor: fill_ratio must be >= 0.95 (got {}).",
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

#[test]
fn gate_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\""
    );
}

#[test]
fn gate_5_last_audit_is_2026_05_19() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some("2026-05-19"),
        "[parity] last_audit must reflect the 2026-05-19 wave-3 close-out"
    );
}

#[test]
fn gate_6_mapped_partial_skipped_unmapped_sum_to_total_with_floor() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count");
    let partial = read("partial_count").expect("partial_count");
    let skipped = read("skipped_count").expect("skipped_count");
    let unmapped = read("unmapped_count").expect("unmapped_count");
    let total = read("total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(
        mapped >= 20,
        "cave-vault wave-3 floor: >= 20 mapped OpenBao subsystems (got {})",
        mapped
    );
    assert_eq!(
        unmapped, 0,
        "wave-3 close-out: zero unmapped — every gap is either mapped or scope-cut"
    );
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders = Vec::new();
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false)
            && let Ok(s) = fs::read_to_string(p)
        {
            for (i, line) in s.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("unimplemented!(") || code.contains("todo!(") {
                    offenders.push(format!("{}:{}", p.display(), i + 1));
                }
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed: {:?}",
        offenders
    );
}

#[test]
fn gate_8_every_rs_file_carries_agpl_spdx() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let body = fs::read_to_string(p).unwrap_or_default();
            let head: String = body.lines().take(5).collect::<Vec<_>>().join("\n");
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
    assert!(
        total >= 30,
        "expected >= 30 .rs files in cave-vault; got {}",
        total
    );
}

#[test]
fn gate_9_parity_report_exists_with_charter_v2_stamp() {
    let report: PathBuf = [env!("CARGO_MANIFEST_DIR"), "PARITY_REPORT.md"]
        .iter()
        .collect();
    assert!(
        report.exists(),
        "PARITY_REPORT.md required at {}",
        report.display()
    );
    let body = fs::read_to_string(&report).expect("read PARITY_REPORT.md");
    assert!(
        body.contains("Charter v2"),
        "PARITY_REPORT.md must reference Charter v2"
    );
    assert!(
        body.contains("8/8 PASS") || body.contains("8-gate"),
        "PARITY_REPORT.md must record 8-gate close-out"
    );
    assert!(
        body.contains("v2.5.3"),
        "PARITY_REPORT.md must pin OpenBao v2.5.3"
    );
}

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            if p.file_name().map(|n| n == "target").unwrap_or(false) {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}
