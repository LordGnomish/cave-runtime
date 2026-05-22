// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-karpenter must carry an honest,
//! measured `fill_ratio` against upstream kubernetes-sigs/karpenter
//! v1.4.0, a pinned `source_sha` for reproducibility, the
//! 2026-05-19 close-out audit date, `parity_ratio_source =
//! "manifest"`, 100% AGPL SPDX header coverage, no stub macros in
//! `src/`, and the 6-variant RequirementOperator + 2-variant
//! ScheduleOutcome surface reachable through the public API.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-19";
const FLOOR_FILL_RATIO: f64 = 0.40;
const PINNED_VERSION: &str = "v1.4.0";

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

// ─── Assertion 1: upstream pinned to v1.4.0 ─────────────────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin Karpenter {} — Charter v2 always-latest gate (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha matches version ────────────────────────────────

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

// ─── Assertion 3: fill_ratio >= 0.40 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-karpenter MVP floor: fill_ratio must be >= {} (got {}). \
         Document additional [[skipped]]/[[unmapped]] surfaces or improve coverage.",
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

// ─── Assertion 6: counts sum to total + >= 8 mapped ─────────────────────────

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
        mapped >= 8,
        "cave-karpenter MVP floor: >= 8 mapped Karpenter subsystems (got {})",
        mapped
    );
}

// ─── Assertion 7: AGPL SPDX header coverage 100% ────────────────────────────

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
        total >= 5,
        "expected >= 5 .rs files in cave-karpenter; got {}",
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

// ─── Assertion 9: 6-operator + ScheduleOutcome surface reachable ────────────

#[test]
fn assertion_9_crd_and_scheduler_surface_intact() {
    // The v1 RequirementOperator must expose all 6 upstream variants and
    // the scheduler's two-variant ScheduleOutcome must remain reachable
    // through the public API. A regression turns this assertion red
    // rather than only the consuming portal/cavectl call site.
    use cave_karpenter::models::{
        NodeClaim, NodeClass, NodePool, Requirement, RequirementOperator, Taint,
    };
    use cave_karpenter::scheduler::ScheduleOutcome;
    use cave_karpenter::{MODULE_NAME, Store, schedule_first_match};

    let _ops = [
        RequirementOperator::In,
        RequirementOperator::NotIn,
        RequirementOperator::Exists,
        RequirementOperator::DoesNotExist,
        RequirementOperator::Gt,
        RequirementOperator::Lt,
    ];

    let outcome = schedule_first_match(&[], &[]);
    assert!(
        matches!(outcome, ScheduleOutcome::NoMatch { .. }),
        "empty pools must produce NoMatch"
    );

    let _pool = NodePool::default();
    let _claim = NodeClaim::default();
    let _class = NodeClass::default();
    let _taint = Taint::default();
    let _ = Store::new();
    assert_eq!(MODULE_NAME, "cave-karpenter");

    // Touch Requirement struct constructively to keep it in the public API.
    let r = Requirement {
        key: "topology.kubernetes.io/zone".into(),
        operator: RequirementOperator::In,
        values: vec!["us-east-1a".into()],
        min_values: None,
    };
    assert_eq!(r.values.len(), 1);
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
