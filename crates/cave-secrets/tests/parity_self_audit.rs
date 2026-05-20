// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit — cave-secrets' own `parity.manifest.toml`
//! must carry the canonical 2026-05-19 close-out fields:
//!
//!   1. upstream version == "v3.63.7"
//!   2. source_sha pinned to the v3.63.7 tag
//!   3. fill_ratio measured, >= 0.80 (cave-secrets Charter floor)
//!   4. parity_ratio_source == "manifest"
//!   5. last_audit == "2026-05-19"
//!   6. infra_only == false
//!   7. at least 5 [[files]] mapping blocks
//!   8. mapped + partial + skipped + unmapped == total
//!   9. every .rs file under crates/cave-secrets carries the AGPL SPDX header
//!
//! Parses the manifest as plain text (no new dependency).

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
fn upstream_version_is_pinned_v3_63_7() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ")
        .or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("v3.63.7"),
        "manifest [upstream] version must pin TruffleHog v3.63.7 (was {:?})",
        v
    );
}

#[test]
fn upstream_source_sha_is_present() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ")
        .or_else(|| extract_after(&m, "\nsource_sha="))
        .or_else(|| extract_after(&m, "\nsha "))
        .or_else(|| extract_after(&m, "\nsha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "manifest [upstream] source_sha must be set for reproducibility (got {:?})",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some("v3.63.7"),
        "source_sha should pin the v3.63.7 release tag (got {:?})",
        sha
    );
}

#[test]
fn parity_fill_ratio_is_measured_and_at_least_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.80,
        "cave-secrets measured floor: fill_ratio must be >= 0.80 (got {})",
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
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
    let when = extract_after(&m, "\nlast_audit ")
        .or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some("2026-05-19"),
        "[parity] last_audit must reflect the 2026-05-19 Charter v2 close-out"
    );
}

#[test]
fn parity_infra_only_is_false() {
    let m = manifest_text();
    let v = extract_after(&m, "\ninfra_only ")
        .or_else(|| extract_after(&m, "\ninfra_only="));
    assert_eq!(
        v.as_deref(),
        Some("false"),
        "cave-secrets IS a parity surface vs TruffleHog — infra_only must be false"
    );
}

#[test]
fn at_least_five_mapped_file_blocks() {
    let m = manifest_text();
    let blocks = m.matches("\n[[files]]").count();
    assert!(
        blocks >= 5,
        "expected >= 5 [[files]] blocks; got {}",
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
        total >= 3,
        "expected >= 3 .rs files in cave-secrets; got {}",
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
