// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit — cave-net's own `parity.manifest.toml` must
//! carry an honest measured `fill_ratio` against cilium/cilium v1.19.3,
//! a pinned `source_sha` for reproducibility, the 2026-05-19 FINALIZE
//! audit date, and structurally consistent counts.
//!
//! The previous manifest already carried measured `fill_ratio = 0.9179`
//! (mapped 42 + skipped 81 + unmapped 11 = 134), but the `[upstream]`
//! block was missing `source_sha`. That gap was the last open item on
//! the Charter v2 8-gate close-out for cave-net. This file is the RED
//! signal — it parses the manifest as plain text (no new dependency)
//! and asserts every gate explicitly so a future drift in any single
//! field surfaces as a localised test failure rather than a silent
//! audit-doc regression.

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
fn upstream_version_is_pinned_v1_19_3() {
    let m = manifest_text();
    let v =
        extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("v1.19.3"),
        "manifest [upstream] version must pin cilium v1.19.3 (was {:?})",
        v
    );
}

#[test]
fn upstream_source_sha_is_present_and_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ")
        .or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "manifest [upstream] source_sha must be set for reproducibility \
         (got {:?}). Charter v2 close-out: pin to the v1.19.3 tag tree-ish.",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some("v1.19.3"),
        "source_sha should match the pinned upstream version \
         (got {:?})",
        sha
    );
}

#[test]
fn parity_fill_ratio_is_measured_and_at_least_0_9() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.9,
        "cave-net measured floor: fill_ratio must be >= 0.9 (got {})",
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

#[test]
fn parity_honest_ratio_matches_fill_ratio() {
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
        (fill - honest).abs() < 1e-6,
        "fill_ratio ({}) and honest_ratio ({}) must agree post-audit",
        fill,
        honest
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
        "cave-net IS a parity surface vs cilium — infra_only must be false"
    );
}

#[test]
fn at_least_forty_mapped_blocks() {
    let m = manifest_text();
    let n = m.matches("\n[[mapped]]").count();
    assert!(
        n >= 40,
        "expected >= 40 [[mapped]] blocks (cilium pkg ports); got {}",
        n
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
    assert!(total >= 100, "expected >= 100 .rs files in cave-net; got {}", total);
}

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Skip target/ and any hidden dirs.
            if p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
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
