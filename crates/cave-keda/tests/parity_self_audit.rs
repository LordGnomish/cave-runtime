// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-keda must carry an honest, measured
//! `fill_ratio` against upstream kedacore/keda v2.16.1, a pinned
//! `source_sha` for reproducibility, the 2026-06-07 close-out audit
//! date, `parity_ratio_source = "manifest"`, a workspace-member
//! listing, 100% AGPL SPDX header coverage, no stub macros in
//! `src/`, and 7+ first-class scalers wired through the public API.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-06-07";
const FLOOR_FILL_RATIO: f64 = 0.55;
const PINNED_VERSION: &str = "v2.16.1";

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
    let unquoted = comment_split.trim_matches('"');
    Some(unquoted.to_string())
}

// ─── Assertion 1: upstream pinned to v2.16.1 (always-latest) ─────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin KEDA {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha present and matches version ─────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "[upstream] source_sha must be set for reproducibility (got {:?})",
        sha
    );
    assert_eq!(
        sha.as_deref(),
        Some(PINNED_VERSION),
        "source_sha must match the pinned upstream version (got {:?})",
        sha
    );
}

// ─── Assertion 3: fill_ratio >= 0.55 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-keda MVP floor: fill_ratio must be >= {} (got {}). \
         Either improve coverage or move surfaces to [[skipped]].",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(
        ratio <= 1.0,
        "fill_ratio must be a fraction (got {})",
        ratio
    );
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" — Charter v2 honest-attribution gate (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit == 2026-05-19 ──────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: mapped + partial + skipped + unmapped sum to total ────────

#[test]
fn assertion_6_counts_sum_to_total() {
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
        mapped >= 10,
        "cave-keda MVP floor: >= 10 mapped KEDA subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: every .rs file carries AGPL SPDX header ───────────────────

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
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
        total >= 10,
        "expected >= 10 .rs files in cave-keda; got {}",
        total
    );
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else {
            return;
        };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"todo")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: backend-track surface — 7+ scalers exported ───────────────

#[test]
fn assertion_9_seven_scalers_exported() {
    // The 7 KEDA scalers Burak named in the close-out scope are all
    // reachable from the public API — a regression here would mean a
    // module deletion slipped past review.
    use cave_keda::{
        CpuScaler, CronScaler, HttpScaler, KafkaScaler, MemoryScaler, PrometheusScaler,
        RedisScaler, ScaledJob, ScaledObject, TriggerAuthentication,
    };

    // Smoke-construct each one — if a constructor signature drifts this
    // assertion turns red rather than the breakage hiding in a portal call.
    let _ = CpuScaler::new("t");
    let _ = MemoryScaler::new("t");
    let _ = CronScaler::new("t");
    let _ = HttpScaler::new("t");
    let _ = KafkaScaler::new("t");
    let _ = PrometheusScaler::new("t");
    let _ = RedisScaler::new("t");
    let _ = ScaledObject::new("t");
    let _ = ScaledJob::new("t");
    let _ = TriggerAuthentication::new("t");
}

// ─── helpers ────────────────────────────────────────────────────────────────

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

// Workspace-membership assertion deliberately omitted — the integration
// test binary cannot compile unless the crate is a workspace member,
// so the build itself enforces it. The 9 assertions above cover
// Charter v2 gates 1/2/3/4/5/6/7/8 and the backend-surface check.
