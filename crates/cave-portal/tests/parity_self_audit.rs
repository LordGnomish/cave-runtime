// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit — cave-portal's own `parity.manifest.toml`
//! must carry an honest measured `fill_ratio` against backstage v1.50.3,
//! a pinned `source_sha` for reproducibility, and per-axis mapping
//! blocks. The previous manifest declared `infra_only = true` with no
//! mapping inventory, which gated /admin/compliance parity_ratio at
//! 0.25 audit-fallback and prevented source="manifest" from being
//! reachable.
//!
//! This file is the RED signal for the close-out — it parses the
//! manifest as plain text (no new dependency) and asserts:
//!
//! 1. `[upstream] version` is the pinned `v1.50.3`.
//! 2. `[upstream] source_sha` exists (commit hash or tag).
//! 3. `[parity] fill_ratio` is present and `>= 0.65`.
//! 4. `[parity] last_audit` is the 2026-05-19 close-out.
//! 5. At least 60 `[[mapped]]` blocks exist (one per admin sub-page).
//! 6. At least 4 `[[surfaces]]` blocks exist (HTTP routes).
//! 7. At least 4 `[[tests]]` blocks exist (Backstage Cypress mappings).
//! 8. `[parity].infra_only` is `false` (Backstage IS the upstream now).

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
    let unquoted = stripped.trim_matches('"');
    Some(unquoted.to_string())
}

#[test]
fn upstream_version_is_pinned_v1_50_3() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("v1.50.3"),
        "manifest [upstream] version must pin backstage v1.50.3 (was {:?})",
        v
    );
}

#[test]
fn upstream_source_sha_is_present() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "manifest [upstream] source_sha must be set for reproducibility (got {:?})",
        sha
    );
}

#[test]
fn parity_fill_ratio_is_measured_and_at_least_0_65() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present after close-out")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.65,
        "Charter v2 floor: fill_ratio must be >= 0.65 (got {})",
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

#[test]
fn parity_last_audit_is_2026_05_19() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some("2026-05-19"),
        "[parity] last_audit must reflect the close-out date"
    );
}

#[test]
fn parity_infra_only_is_false() {
    let m = manifest_text();
    let v = extract_after(&m, "\ninfra_only ").or_else(|| extract_after(&m, "\ninfra_only="));
    assert_eq!(
        v.as_deref(),
        Some("false"),
        "after close-out cave-portal IS a parity surface vs backstage"
    );
}

#[test]
fn at_least_sixty_mapped_blocks() {
    let m = manifest_text();
    let n = m.matches("\n[[mapped]]").count();
    assert!(
        n >= 60,
        "expected >= 60 [[mapped]] blocks (one per admin sub-page); got {}",
        n
    );
}

#[test]
fn at_least_four_surface_blocks() {
    let m = manifest_text();
    let n = m.matches("\n[[surfaces]]").count();
    assert!(
        n >= 4,
        "expected >= 4 [[surfaces]] blocks (HTTP routes /admin/...); got {}",
        n
    );
}

#[test]
fn at_least_four_test_blocks() {
    let m = manifest_text();
    let n = m.matches("\n[[tests]]").count();
    assert!(
        n >= 4,
        "expected >= 4 [[tests]] blocks (upstream Cypress → local Rust mappings); got {}",
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
fn parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "[parity] parity_ratio_source must be \"manifest\" after close-out"
    );
}

#[test]
fn parity_report_md_exists_with_8_gate_stamp() {
    let p: std::path::PathBuf = [env!("CARGO_MANIFEST_DIR"), "PARITY_REPORT.md"]
        .iter()
        .collect();
    assert!(
        p.exists(),
        "PARITY_REPORT.md required for Charter v2 close-out"
    );
    let body = std::fs::read_to_string(&p).expect("read PARITY_REPORT.md");
    assert!(
        body.contains("Charter v2"),
        "report must mention Charter v2"
    );
    assert!(
        body.contains("8/8 PASS") || body.contains("8-gate"),
        "report must include 8/8 PASS or 8-gate stamp"
    );
}
