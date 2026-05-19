// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-streams must carry an honest, measured
//! `fill_ratio` against both upstream Apache Kafka 4.2.0 and Apache
//! Pulsar v4.2.0 (unified Rust crate exposing both wire protocols, per
//! ADR-RUNTIME-STREAMING-CONSOLIDATION-001), a pinned `source_sha` per
//! upstream for reproducibility, the 2026-05-19 close-out audit date,
//! `parity_ratio_source = "manifest"`, a workspace-member listing,
//! 100% AGPL SPDX header coverage, no stub macros in `src/`, and a
//! `docs/parity/parity-index.json` row whose `parity_ratio` matches
//! the manifest's `fill_ratio`.
//!
//! 9 assertions — RED until the 2026-05-19 close-out commit fills
//! the four missing manifest fields (`source_sha`, `parity_ratio_source`,
//! `last_audit`).

use std::fs;
use std::path::PathBuf;

const TODAY: &str = "2026-05-19";
const FLOOR_FILL_RATIO: f64 = 0.0;

fn workspace_root() -> PathBuf {
    let mut p: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    // CARGO_MANIFEST_DIR -> crates/cave-streams, parent twice = repo root
    p.pop();
    p.pop();
    p
}

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

// ─── Assertion 1: workspace license is AGPL-3.0-or-later ─────────────────────

#[test]
fn assertion_1_workspace_license_is_agpl() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml"))
        .expect("read root Cargo.toml");
    let lic = extract_after(&cargo, "\nlicense ")
        .or_else(|| extract_after(&cargo, "\nlicense="));
    assert_eq!(
        lic.as_deref(),
        Some("AGPL-3.0-or-later"),
        "workspace [workspace.package] license must be AGPL-3.0-or-later (got {:?})",
        lic
    );
}

// ─── Assertion 2: source_sha is present and non-empty ────────────────────────

#[test]
fn assertion_2_source_sha_present_and_non_empty() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ")
        .or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "[parity] source_sha must be set and non-empty (got {:?})",
        sha
    );
    // The unified crate has two upstreams — the value must reference both.
    let raw = sha.unwrap();
    assert!(
        raw.contains("kafka") && raw.contains("pulsar"),
        "cave-streams ports BOTH Kafka and Pulsar — source_sha must reference both upstreams (got {:?})",
        raw
    );
}

// ─── Assertion 3: fill_ratio is positive and a valid fraction ────────────────

#[test]
fn assertion_3_fill_ratio_is_positive_fraction() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio > FLOOR_FILL_RATIO,
        "[parity] fill_ratio must be > {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(
        ratio <= 1.0,
        "[parity] fill_ratio must be a fraction <= 1.0 (got {})",
        ratio
    );
}

// ─── Assertion 4: parity_ratio_source = "manifest" ───────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "[parity] parity_ratio_source must be \"manifest\" — Charter v2 honest-attribution gate (got {:?})",
        v
    );
}

// ─── Assertion 5: cave-streams is a workspace member ─────────────────────────

#[test]
fn assertion_5_cave_streams_is_workspace_member() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml"))
        .expect("read root Cargo.toml");
    assert!(
        cargo.contains("\"crates/cave-streams\""),
        "root Cargo.toml [workspace.members] must list \"crates/cave-streams\""
    );
}

// ─── Assertion 6: every src/ + tests/ .rs file carries AGPL SPDX ─────────────

#[test]
fn assertion_6_agpl_spdx_header_coverage() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .map(|s| s.lines().take(3).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(
        missing.is_empty(),
        "{} of {} .rs files in cave-streams missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
}

// ─── Assertion 7: no stub macros in src/ ─────────────────────────────────────

#[test]
fn assertion_7_no_stub_macros_in_src() {
    let src: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders: Vec<String> = Vec::new();
    walk(&src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else { return };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip comments/doc-comments — Burak's spec explicitly allows
            // `// TODO(S2→S1)`-style commentary lines.
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
        "no-stub gate: src/ must not contain todo!()/unimplemented!()/panic!(\"stub\") — found:\n{}",
        offenders.join("\n")
    );
}

// ─── Assertion 8: last_audit == 2026-05-19 (always-latest) ───────────────────

#[test]
fn assertion_8_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ")
        .or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity] last_audit must reflect the {} Charter v2 close-out (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 9: parity-index.json consistency ──────────────────────────────

#[test]
fn assertion_9_parity_index_json_consistency() {
    let root = workspace_root();
    let idx = fs::read_to_string(root.join("docs/parity/parity-index.json"))
        .expect("read docs/parity/parity-index.json");

    // Locate the "cave-streams": { ... } block.
    let i = idx
        .find("\"cave-streams\":")
        .expect("docs/parity/parity-index.json must contain cave-streams entry");
    let block_end = idx[i..]
        .find("\n    }")
        .expect("cave-streams block must close with `\\n    }`");
    let block = &idx[i..i + block_end];

    // Extract parity_ratio from parity-index.
    let pr_line = block
        .lines()
        .find(|l| l.contains("\"parity_ratio\":"))
        .expect("cave-streams block must contain parity_ratio");
    let pr_idx: f64 = pr_line
        .split(':')
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(',')
        .parse()
        .expect("parity-index parity_ratio parses as float");

    // Extract fill_ratio from manifest.
    let m = manifest_text();
    let fr_man: f64 = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .and_then(|s| s.parse().ok())
        .expect("manifest fill_ratio parses");

    // Tolerance — parity-index drives off manifest at audit time.
    let drift = (pr_idx - fr_man).abs();
    assert!(
        drift < 1e-4,
        "parity-index.json parity_ratio ({}) must match manifest fill_ratio ({}) — drift {}",
        pr_idx,
        fr_man,
        drift
    );

    // Also assert parity_ratio_source = "manifest" in the index.
    assert!(
        block.contains("\"parity_ratio_source\": \"manifest\""),
        "parity-index.json cave-streams entry must declare parity_ratio_source = \"manifest\""
    );
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
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
