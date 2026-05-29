// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit for cave-search parity.manifest.toml.
//!
//! Every gate assertion is explicit so a future drift in any single field
//! surfaces as a localised test failure rather than a silent audit regression.

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

/// G1: source_sha / version pinned
#[test]
fn upstream_version_is_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert!(
        v.is_some() && !v.as_deref().unwrap().is_empty(),
        "[upstream] version must be set; got {:?}",
        v
    );
}

#[test]
fn upstream_source_sha_is_present() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "[upstream] source_sha must be set for reproducibility; got {:?}",
        sha
    );
}

/// G3: honest_ratio is truthful
#[test]
fn parity_fill_ratio_is_one() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as f64");
    assert!(
        (ratio - 1.0).abs() < 1e-9,
        "cave-search honest uplift: fill_ratio must be 1.0 (got {})",
        ratio
    );
}

#[test]
fn parity_honest_ratio_in_range() {
    let m = manifest_text();
    let fill: f64 = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .and_then(|s| s.parse().ok())
        .expect("fill_ratio parses");
    let honest: f64 = extract_after(&m, "\nhonest_ratio ")
        .or_else(|| extract_after(&m, "\nhonest_ratio="))
        .and_then(|s| s.parse().ok())
        .expect("honest_ratio parses");
    assert!(
        honest <= fill + 1e-6,
        "honest_ratio ({}) must be <= fill_ratio ({})",
        honest, fill
    );
    assert!(
        honest >= 0.3,
        "honest_ratio floor: >= 0.3 (got {}); if this is actually lower, update the floor",
        honest
    );
}

/// G4: manifest present and last_audit set
#[test]
fn parity_last_audit_is_2026() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert!(
        when.as_deref().map(|s| s.starts_with("2026-")).unwrap_or(false),
        "[parity] last_audit must start with 2026- (got {:?})",
        when
    );
}

#[test]
fn parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

/// Counts must sum to total; at least 8 mapped
#[test]
fn counts_sum_to_total_and_enough_mapped() {
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
    assert_eq!(unmapped, 0, "unmapped_count must be 0 (every surface accounted for)");
    assert!(
        mapped >= 8,
        "expected >= 8 mapped Manticore subsystems (got {})",
        mapped
    );
}

/// G5: No unimplemented!/todo!() macros in src/
#[test]
fn no_stub_macros_in_src() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders = Vec::new();
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            if let Ok(s) = fs::read_to_string(p) {
                for (i, line) in s.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if code.contains("unimplemented!(") || code.contains("todo!(") {
                        offenders.push(format!("{}:{}", p.display(), i + 1));
                    }
                }
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed; remove unimplemented!/todo! macros:\n{:?}",
        offenders
    );
}

/// G2: Every .rs file carries AGPL SPDX header
#[test]
fn every_rs_file_carries_agpl_spdx() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string()))
                .unwrap_or_default();
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
        total >= 6,
        "expected >= 6 .rs files in cave-search; got {}",
        total
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
