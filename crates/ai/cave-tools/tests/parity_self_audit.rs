// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit for cave-tools — pinned to MCP spec 2025-11-25.
//!
//! Nine assertions, one per close-out gate. A regression in any single
//! field surfaces as a localised failure rather than silent audit drift.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

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
    Some(comment_split.trim_matches('"').to_string())
}

#[test]
fn gate_1_upstream_version_pinned_2025_11_25() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("2025-11-25"),
        "manifest [upstream] version must pin MCP spec revision 2025-11-25 (was {:?})",
        v
    );
    assert_eq!(
        cave_tools::UPSTREAM_VERSION,
        "2025-11-25",
        "lib UPSTREAM_VERSION must match the manifest pin"
    );
    assert_eq!(cave_tools::MCP_PROTOCOL_VERSION, "2025-11-25");
}

#[test]
fn gate_2_source_sha_present_and_matches_version() {
    let m = manifest_text();
    let sha = extract_after(&m, "\nsource_sha ").or_else(|| extract_after(&m, "\nsource_sha="));
    assert_eq!(
        sha.as_deref(),
        Some("2025-11-25"),
        "source_sha must match the pinned upstream revision (got {:?})",
        sha
    );
}

#[test]
fn gate_3_fill_ratio_measured_and_at_least_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        (0.85..=1.0).contains(&ratio),
        "cave-tools floor: fill_ratio must be in [0.85, 1.0] (got {}). Convert a partial \
         to mapped to raise it — never reclassify a real gap as skipped.",
        ratio
    );
}

#[test]
fn gate_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(v.as_deref(), Some("manifest"));
}

#[test]
fn gate_5_last_audit_is_2026() {
    // Relaxed to a year prefix: the worktree timestamp can differ from the
    // branch close-out date, so we pin the year not the exact day.
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert!(
        when.as_deref().map(|s| s.starts_with("2026-")).unwrap_or(false),
        "[parity] last_audit must be a 2026 ISO date (got {:?})",
        when
    );
}

#[test]
fn gate_6_counts_sum_to_total_and_honest_ratio_consistent() {
    let m = manifest_text();
    let read = |k: &str| -> u64 {
        extract_after(&m, &format!("\n{} ", k))
            .or_else(|| extract_after(&m, &format!("\n{}=", k)))
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("missing/!parse count {k}"))
    };
    let mapped = read("mapped_count");
    let partial = read("partial_count");
    let skipped = read("skipped_count");
    let unmapped = read("unmapped_count");
    let total = read("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped must equal total"
    );
    assert!(mapped >= 20, "MVP floor: >= 20 mapped subsystems (got {mapped})");

    // honest_ratio must equal mapped / in-scope (mapped+partial+unmapped),
    // i.e. ADR-justified skips are excluded from the denominator.
    let in_scope = (mapped + partial + unmapped) as f64;
    let expected = mapped as f64 / in_scope;
    let honest: f64 = extract_after(&m, "\nhonest_ratio ")
        .or_else(|| extract_after(&m, "\nhonest_ratio="))
        .and_then(|s| s.parse().ok())
        .expect("honest_ratio present");
    assert!(
        (honest - expected).abs() < 1e-6,
        "honest_ratio {honest} must equal mapped/in-scope {expected} ({mapped}/{in_scope})"
    );
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src"].iter().collect();
    let mut offenders = Vec::new();
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false)
            && let Ok(s) = fs::read_to_string(p)
        {
            for (i, line) in s.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("unimplemented!(") || code.contains("todo!(") {
                    offenders.push(format!("{}:{}", p.display(), i + 1));
                }
            }
        }
    });
    assert!(offenders.is_empty(), "no-stub gate failed: {:?}", offenders);
}

#[test]
fn gate_8_every_rs_file_carries_agpl_spdx() {
    let root: PathBuf = [env!("CARGO_MANIFEST_DIR")].iter().collect();
    let mut missing = Vec::new();
    let mut total = 0usize;
    walk(&root, &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p)
                .ok()
                .and_then(|s| s.lines().next().map(str::to_string))
                .unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    assert!(missing.is_empty(), "missing AGPL SPDX: {:?}", missing);
    assert!(total >= 10, "expected >= 10 .rs files, got {total}");
}

#[test]
fn gate_9_register_builtins_wires_the_eight_named_tools() {
    use cave_tools::builtin::{
        BuiltinConfig, Calendar, FileSandbox, Mailbox, WebResult, WebSearchProvider,
    };
    use cave_tools::tool::ToolRegistry;

    struct NoWeb;
    impl WebSearchProvider for NoWeb {
        fn search(&self, _q: &str, _n: usize) -> Vec<WebResult> {
            Vec::new()
        }
    }

    let tmp = std::env::temp_dir();
    let cfg = BuiltinConfig {
        file_sandbox: Arc::new(FileSandbox::new(&tmp)),
        web: Arc::new(NoWeb),
        calendar: Arc::new(Calendar::new()),
        mailbox: Arc::new(Mailbox::new()),
    };
    let mut reg = ToolRegistry::new();
    cave_tools::builtin::register_builtins(&mut reg, &cfg);
    assert_eq!(reg.len(), cave_tools::BUILTIN_TOOL_NAMES.len());
    for name in cave_tools::BUILTIN_TOOL_NAMES {
        assert!(reg.get(name).is_some(), "missing built-in tool: {name}");
    }
}

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let skip = p
                .file_name()
                .map(|n| n.to_string_lossy().starts_with('.') || n == "target")
                .unwrap_or(false);
            if !skip {
                walk(&p, cb);
            }
        } else {
            cb(&p);
        }
    }
}
