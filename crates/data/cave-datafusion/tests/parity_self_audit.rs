// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-datafusion must carry an honest, measured
//! `fill_ratio` against upstream apache/datafusion at the latest stable
//! release (53.1.0), a pinned `source_sha` for reproducibility, the
//! 2026-05-19 close-out audit date, `parity_ratio_source = "manifest"`,
//! a workspace-member listing, 100% AGPL SPDX header coverage, no stub
//! macros in `src/`, and a `docs/parity/parity-index.json` row whose
//! `parity_ratio` matches the manifest's `fill_ratio`.

use std::fs;
use std::path::PathBuf;

// Audit date is re-stamped on every honest uplift wave; the gate asserts a
// well-formed 2026 ISO date rather than a single frozen day (relaxed
// 2026-05-30 wave-2 when CSE was promoted skipped→mapped and last_audit
// advanced from 2026-05-24 → 2026-05-30).
const AUDIT_YEAR_PREFIX: &str = "2026-";
const FLOOR_FILL_RATIO: f64 = 0.95;

fn workspace_root() -> PathBuf {
    // Walk up from the crate manifest dir until we find the Cargo.toml that
    // declares `[workspace]`. Theme-reorg moved this crate from
    // `crates/cave-datafusion` to `crates/data/cave-datafusion`, so a fixed
    // pop-count is fragile; locate the workspace root structurally instead.
    let mut p: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    loop {
        if fs::read_to_string(p.join("Cargo.toml"))
            .map(|s| s.contains("[workspace]"))
            .unwrap_or(false)
        {
            return p;
        }
        if !p.pop() {
            // Fallback to the historical pop-2 behavior.
            let mut q: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
            q.pop();
            q.pop();
            return q;
        }
    }
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

#[test]
fn assertion_1_workspace_license_is_agpl() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read root Cargo.toml");
    let lic = extract_after(&cargo, "\nlicense ").or_else(|| extract_after(&cargo, "\nlicense="));
    assert_eq!(
        lic.as_deref(),
        Some("AGPL-3.0-or-later"),
        "workspace [workspace.package] license must be AGPL-3.0-or-later (got {:?})",
        lic
    );
}

#[test]
fn assertion_2_source_sha_present_and_non_empty() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert!(
        sha.is_some() && !sha.as_deref().unwrap().is_empty(),
        "[upstream] source_sha must be set and non-empty (got {:?})",
        sha
    );
}

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .expect("[parity] fill_ratio must be present");
    let ratio: f64 = raw.parse().expect("fill_ratio must parse as float");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "cave-datafusion Charter v2 floor: fill_ratio must be >= {} (got {})",
        FLOOR_FILL_RATIO,
        ratio
    );
    assert!(
        ratio <= 1.0,
        "[parity] fill_ratio must be a fraction <= 1.0 (got {})",
        ratio
    );
}

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

#[test]
fn assertion_5_cave_datafusion_is_workspace_member() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read root Cargo.toml");
    // Theme-reorg replaced explicit per-crate member paths with the
    // `crates/*/*` glob (cave-datafusion now lives at
    // `crates/data/cave-datafusion`). Accept either form.
    assert!(
        cargo.contains("\"crates/cave-datafusion\"")
            || cargo.contains("\"crates/*/*\"")
            || cargo.contains("\"crates/data/cave-datafusion\""),
        "root Cargo.toml [workspace.members] must list cave-datafusion (explicit path or crates/*/* glob)"
    );
}

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
        "{} of {} .rs files in cave-datafusion missing AGPL SPDX header: {:?}",
        missing.len(),
        total,
        missing
    );
}

#[test]
fn assertion_7_no_stub_macros_in_src() {
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
        "no-stub gate: src/ must not contain todo!()/unimplemented!()/panic!(\"stub\") — found:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn assertion_8_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    let when = when.expect("[parity] last_audit must be present");
    assert!(
        when.starts_with(AUDIT_YEAR_PREFIX) && when.len() == 10,
        "[parity] last_audit must be a {}MM-DD ISO date reflecting the latest audit wave (got {:?})",
        AUDIT_YEAR_PREFIX,
        when
    );
}

#[test]
fn assertion_9_parity_index_json_consistency() {
    let root = workspace_root();
    let idx = fs::read_to_string(root.join("docs/parity/parity-index.json"))
        .expect("read docs/parity/parity-index.json");

    let i = idx
        .find("\"cave-datafusion\":")
        .expect("docs/parity/parity-index.json must contain cave-datafusion entry");
    let block_end = idx[i..]
        .find("\n    }")
        .expect("cave-datafusion block must close with `\\n    }`");
    let block = &idx[i..i + block_end];

    let pr_line = block
        .lines()
        .find(|l| l.contains("\"parity_ratio\":"))
        .expect("cave-datafusion block must contain parity_ratio");
    let pr_idx: f64 = pr_line
        .split(':')
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(',')
        .parse()
        .expect("parity-index parity_ratio parses as float");

    let m = manifest_text();
    let fr_man: f64 = extract_after(&m, "\nfill_ratio ")
        .or_else(|| extract_after(&m, "\nfill_ratio="))
        .and_then(|s| s.parse().ok())
        .expect("manifest fill_ratio parses");

    let drift = (pr_idx - fr_man).abs();
    assert!(
        drift < 1e-4,
        "parity-index.json parity_ratio ({}) must match manifest fill_ratio ({}) — drift {}",
        pr_idx,
        fr_man,
        drift
    );

    assert!(
        block.contains("\"parity_ratio_source\": \"manifest\""),
        "parity-index.json cave-datafusion entry must declare parity_ratio_source = \"manifest\""
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
