// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit — cave-flags's own `parity.manifest.toml` must
//! carry an honest measured `fill_ratio` against Unleash v5.0.0, a
//! pinned `source_sha` for reproducibility, the 2026-05-18 close-out
//! audit date, and structurally consistent bucket counts.
//!
//! Read-only audit on 2026-05-19 surfaced cave-flags as "kısmen": rich
//! mapping (15 tests, full engine + admin/client/frontend routes + Postgres
//! schema), but `[parity].ratio = 0.0`, no `source_sha`, no `parity_ratio_source`,
//! no `[[mapped]]` / `[[partial]]` / `[[unmapped]]` block breakdown — i.e. the
//! Charter v2 `parity_self_audit` test had never been written. This file is
//! the RED signal — every gate is asserted explicitly so a future drift in
//! any single field surfaces as a localised test failure rather than a silent
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
fn upstream_version_is_pinned_v5_0_0() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("v5.0.0"),
        "manifest [upstream] version must pin Unleash v5.0.0 (was {:?})",
        v
    );
}

#[test]
fn upstream_source_sha_is_present_and_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "manifest [upstream] source_sha must be set for reproducibility \
         (got {:?}). Charter v2 close-out: pin to the v5.0.0 tag tree-ish.",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some("v5.0.0"),
        "source_sha should match the pinned upstream version (got {:?})",
        sha
    );
}

#[test]
fn parity_fill_ratio_is_measured_and_at_least_0_65() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.95,
        "cave-flags parity-uplift floor: fill_ratio must be >= 0.95 \
         (got {}). Either improve coverage or document scope-cuts as [[skipped]].",
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

#[test]
fn parity_honest_ratio_is_present_and_le_fill_ratio() {
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
        "honest_ratio ({}) must be <= fill_ratio ({}): honest excludes \
         [[partial]] from the numerator",
        honest,
        fill
    );
    assert!(
        honest >= 0.5,
        "honest_ratio floor for cave-flags MVP: >= 0.5 (got {})",
        honest
    );
}

#[test]
fn parity_last_audit_is_current() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some("2026-05-19"),
        "[parity] last_audit must reflect the 2026-05-19 parity-uplift close-out"
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
        "parity_ratio_source must be \"manifest\" so cave-upstream parity-index \
         reads fill_ratio from this file rather than an external audit doc"
    );
}

#[test]
fn counts_sum_to_total_and_at_least_twenty_mapped() {
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
        "expected >= 20 mapped Unleash subsystems (got {})",
        mapped
    );
}

#[test]
fn no_stub_macros_in_src() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders = Vec::new();
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            if let Ok(s) = fs::read_to_string(p) {
                for (i, line) in s.lines().enumerate() {
                    // Strip line comments to skip the `// TODO:` style notes.
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
        "Charter v2 no-stub gate failed; remove unimplemented!/todo! macros: {:?}",
        offenders
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
        total >= 7,
        "expected >= 7 .rs files in cave-flags; got {}",
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
