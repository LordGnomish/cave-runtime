// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-edge-runtime must carry an honest, measured
//! `fill_ratio`/`honest_ratio` against its upstreams (KubeEdge v1.22.0 + K3s
//! edge mode), a pinned `source_sha`, a 2026 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage, no
//! stub macros in `src/`, count consistency, and the seven priority
//! subsystems reachable through the public API.
//!
//! 9 assertions — one per gate.

use std::fs;
use std::path::PathBuf;

const PINNED_VERSION: &str = "v1.22.0";
const FLOOR_FILL_RATIO: f64 = 0.55;
const FLOOR_HONEST_RATIO: f64 = 0.50;

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

// ─── Assertion 1: upstream pinned to KubeEdge v1.22.0 ───────────────────────

#[test]
fn assertion_1_upstream_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(PINNED_VERSION),
        "[upstream] version must pin KubeEdge {} (got {:?})",
        PINNED_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha present and matches version ────────────────────

#[test]
fn assertion_2_source_sha_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(
        sha.as_deref(),
        Some(PINNED_VERSION),
        "source_sha must match the pinned upstream version (got {:?})",
        sha
    );
}

// ─── Assertion 3: fill_ratio in [0.55, 1.0] ─────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        (FLOOR_FILL_RATIO..=1.0).contains(&ratio),
        "fill_ratio must be in [{}, 1.0] (got {})",
        FLOOR_FILL_RATIO,
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
        "parity_ratio_source must be \"manifest\" (got {:?})",
        v
    );
}

// ─── Assertion 5: last_audit is a 2026 close-out date ───────────────────────

#[test]
fn assertion_5_last_audit_is_2026() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    let when = when.expect("[parity] last_audit must be present");
    assert!(
        when.starts_with("2026-"),
        "[parity] last_audit must be a 2026 close-out date (got {:?})",
        when
    );
}

// ─── Assertion 6: counts sum to total + honest_ratio matches mapped/total ───

#[test]
fn assertion_6_counts_sum_and_honest_ratio_consistent() {
    let m = manifest_text();
    let read = |k: &str| -> Option<u64> {
        extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))?
            .parse()
            .ok()
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
        mapped >= 7,
        "floor: >= 7 mapped priority subsystems (got {})",
        mapped
    );

    let honest: f64 = extract_after(&m, "\nhonest_ratio ")
        .or_else(|| extract_after(&m, "\nhonest_ratio="))
        .expect("honest_ratio")
        .parse()
        .expect("honest_ratio float");
    let expected = mapped as f64 / total as f64;
    assert!(
        (honest - expected).abs() < 0.001,
        "honest_ratio ({}) must equal mapped/total ({:.4})",
        honest,
        expected
    );
    assert!(
        honest >= FLOOR_HONEST_RATIO,
        "honest_ratio floor {} (got {})",
        FLOOR_HONEST_RATIO,
        honest
    );
}

// ─── Assertion 7: every .rs file carries the AGPL SPDX header ───────────────

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
    assert!(total >= 8, "expected >= 8 .rs files; got {}", total);
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
        "no-stub gate failed in src/:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 9: the seven priority subsystems are reachable ───────────────

#[test]
fn assertion_9_seven_subsystems_exported() {
    use cave_edge_runtime::{
        ConnectionState, ConstrainedMode, DeviceTwin, EdgeAutonomy, EdgeHub, Edged, EventBus,
        MetaManager, ResourceBudget,
    };

    let _ = Edged::new("node");
    let _ = MetaManager::new();
    let _ = EventBus::new();
    let _ = EdgeHub::new();
    let _ = DeviceTwin::new();
    let a = EdgeAutonomy::new(0);
    assert_eq!(a.state(), ConnectionState::Connected);
    let _ = ConstrainedMode::new(ResourceBudget {
        total_mb: 256,
        reserved_mb: 56,
    });
}

// ─── helper ──────────────────────────────────────────────────────────────

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
