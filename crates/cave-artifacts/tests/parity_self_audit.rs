// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit — cave-artifacts' own `parity.manifest.toml`
//! must carry the canonical 2026-05-19 close-out fields:
//!
//!   1. upstream version == "3.49.0" (pulpcore)
//!   2. source_sha inline-table present (pulpcore + harbor + nexus pins)
//!   3. fill_ratio measured, >= 0.80 (cave-artifacts Charter floor)
//!   4. parity_ratio_source == "manifest"
//!   5. last_audit == "2026-05-19"
//!   6. infra_only == false
//!   7. at least 15 [[files]] mapping blocks
//!   8. mapped + partial + skipped + unmapped == total
//!   9. every .rs file under crates/cave-artifacts carries the AGPL SPDX header
//!
//! cave-artifacts is multi-upstream (Pulp + Harbor + Nexus). The primary
//! [upstream] block pins pulpcore; the source_sha inline-table records
//! all three.

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
fn upstream_version_is_pinned_3_49_0() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("3.49.0"),
        "manifest [upstream] version must pin pulpcore 3.49.0 (was {:?})",
        v
    );
}

#[test]
fn upstream_source_sha_is_inline_table_with_three_upstreams() {
    let m = manifest_text();
    let line = m
        .lines()
        .find(|l| l.trim_start().starts_with("source_sha"))
        .unwrap_or("");
    assert!(
        line.contains("pulpcore") && line.contains("harbor") && line.contains("nexus"),
        "[upstream] source_sha must be an inline-table pinning pulpcore + harbor + nexus (got {:?})",
        line
    );
    assert!(
        line.contains("3.49.0") && line.contains("v2.10.0") && line.contains("3.69.0"),
        "[upstream] source_sha must pin the explicit version tags (got {:?})",
        line
    );
}

#[test]
fn parity_fill_ratio_is_measured_and_at_least_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.80,
        "cave-artifacts measured floor: fill_ratio must be >= 0.80 (got {})",
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

#[test]
fn parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let src = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        src.as_deref(),
        Some("manifest"),
        "[parity] parity_ratio_source must be \"manifest\" (got {:?})",
        src
    );
}

#[test]
fn parity_last_audit_is_2026_05_19() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some("2026-05-19"),
        "[parity] last_audit must reflect the 2026-05-19 Charter v2 close-out"
    );
}

#[test]
fn parity_infra_only_is_false() {
    let m = manifest_text();
    let v = extract_after(&m, "\ninfra_only ").or_else(|| extract_after(&m, "\ninfra_only="));
    assert_eq!(
        v.as_deref(),
        Some("false"),
        "cave-artifacts IS a parity surface vs Pulp+Harbor+Nexus — infra_only must be false"
    );
}

#[test]
fn at_least_fifteen_mapped_file_blocks() {
    let m = manifest_text();
    let blocks = m.matches("\n[[files]]").count();
    assert!(
        blocks >= 15,
        "expected >= 15 [[files]] blocks; got {}",
        blocks
    );
}

#[test]
fn counts_sum_to_total() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        let s = extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?;
        s.parse().ok()
    };
    let mapped = read("mapped_count").expect("mapped_count must be present");
    let partial = read("partial_count").expect("partial_count must be present");
    let skipped = read("skipped_count").expect("skipped_count must be present");
    let unmapped = read("unmapped_count").expect("unmapped_count must be present");
    let total = read("total").expect("total must be present");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total \
         (got {} + {} + {} + {} = {}, expected {})",
        mapped,
        partial,
        skipped,
        unmapped,
        mapped + partial + skipped + unmapped,
        total
    );
}

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
        total >= 40,
        "expected >= 40 .rs files in cave-artifacts; got {}",
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
