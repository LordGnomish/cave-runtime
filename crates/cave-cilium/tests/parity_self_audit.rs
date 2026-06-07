// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter self-audit — cave-cilium's `parity.manifest.toml` must carry an
//! honest, measured `fill_ratio` against cilium/cilium v1.19.4, a pinned
//! `source_sha`, the audit date, and structurally consistent counts.
//!
//! This is a fresh control-plane port: the floor is deliberately honest
//! (>= 0.5), NOT the 0.95 of the mature cave-net datapath crate. The four
//! `[[unmapped]]` rows and three `[[partial]]` rows are real.

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
    let stripped = line.trim().trim_start_matches('=').trim();
    let comment_split = stripped.split('#').next().unwrap_or(stripped).trim();
    Some(comment_split.trim_matches('"').to_string())
}

fn read_f64(m: &str, key: &str) -> f64 {
    extract_after(m, &format!("\n{key} "))
        .or_else(|| extract_after(m, &format!("\n{key}=")))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("{key} must parse as float"))
}

fn read_u64(m: &str, key: &str) -> u64 {
    extract_after(m, &format!("\n{key} "))
        .or_else(|| extract_after(m, &format!("\n{key}=")))
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| panic!("{key} must parse as int"))
}

#[test]
fn upstream_version_is_pinned_v1_19_4() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(v.as_deref(), Some("v1.19.4"), "pin cilium v1.19.4");
}

#[test]
fn source_sha_present_and_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(sha.as_deref(), Some("v1.19.4"), "source_sha pins the tag");
}

#[test]
fn fill_ratio_is_measured_and_honest() {
    let m = manifest_text();
    let ratio = read_f64(&m, "fill_ratio");
    assert!(ratio >= 0.5, "fresh-port honest floor 0.5 (got {ratio})");
    assert!(ratio <= 1.0, "fill_ratio is a fraction (got {ratio})");
}

#[test]
fn ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(v.as_deref(), Some("manifest"));
}

#[test]
fn honest_ratio_matches_fill_ratio() {
    let m = manifest_text();
    assert!(
        (read_f64(&m, "fill_ratio") - read_f64(&m, "honest_ratio")).abs() < 1e-6,
        "fill_ratio and honest_ratio must agree post-audit"
    );
}

#[test]
fn fill_ratio_matches_counts() {
    let m = manifest_text();
    let mapped = read_u64(&m, "mapped_count") as f64;
    let skipped = read_u64(&m, "skipped_count") as f64;
    let total = read_u64(&m, "total") as f64;
    let expected = (mapped + skipped) / total;
    assert!(
        (read_f64(&m, "fill_ratio") - expected).abs() < 1e-3,
        "fill_ratio must equal (mapped+skipped)/total = {expected:.4}"
    );
}

#[test]
fn last_audit_is_2026_06_07() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(when.as_deref(), Some("2026-06-07"));
}

#[test]
fn infra_only_is_false() {
    let m = manifest_text();
    let v = extract_after(&m, "\ninfra_only ").or_else(|| extract_after(&m, "\ninfra_only="));
    assert_eq!(v.as_deref(), Some("false"));
}

#[test]
fn counts_sum_to_total_and_match_blocks() {
    let m = manifest_text();
    let mapped = read_u64(&m, "mapped_count");
    let partial = read_u64(&m, "partial_count");
    let skipped = read_u64(&m, "skipped_count");
    let unmapped = read_u64(&m, "unmapped_count");
    let total = read_u64(&m, "total");
    assert_eq!(mapped + partial + skipped + unmapped, total, "counts sum to total");

    // Scalar counts must match the actual block counts.
    assert_eq!(m.matches("\n[[mapped]]").count() as u64, mapped);
    assert_eq!(m.matches("\n[[partial]]").count() as u64, partial);
    assert_eq!(m.matches("\n[[skipped]]").count() as u64, skipped);
    assert_eq!(m.matches("\n[[unmapped]]").count() as u64, unmapped);
}

#[test]
fn at_least_fifteen_mapped_blocks() {
    let m = manifest_text();
    let n = m.matches("\n[[mapped]]").count();
    assert!(n >= 15, "expected >= 15 mapped pkg ports; got {n}");
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
    assert!(missing.is_empty(), "missing AGPL SPDX header: {missing:?}");
    assert!(total >= 6, "expected >= 6 .rs files; got {total}");
}

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name()
                .map(|n| n.to_string_lossy().starts_with('.') || n == "target")
                .unwrap_or(false)
            {
                continue;
            }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}
