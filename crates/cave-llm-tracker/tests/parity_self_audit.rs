// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 self-audit for cave-llm-tracker — multi-source pin
//! (HuggingFace API + Ollama library + LMSys leaderboard CSV + GitHub
//! backend releases for vLLM / llama.cpp / MLX-LM).
//!
//! Nine assertions covering the eight Charter v2 gates plus one runtime
//! wiring check (Phase 0 mandate enforcement). A regression in any
//! single field surfaces as a localised failure rather than silent
//! audit-doc drift.

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
fn gate_1_upstream_version_pinned_to_2026_05_21() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some("2026-05-21"),
        "manifest [upstream] version must pin the multi-source snapshot date \
         2026-05-21 (was {:?}). Charter v2 always-latest gate.",
        v
    );
    assert_eq!(
        cave_llm_tracker::UPSTREAM_VERSION,
        "2026-05-21",
        "lib UPSTREAM_VERSION must match manifest pin"
    );
}

#[test]
fn gate_2_source_sha_is_inline_table_covering_all_four_sources() {
    let m = manifest_text();
    // The inline TOML table sits on one line: `source_sha = { ... }`.
    let line = m
        .lines()
        .find(|l| l.trim_start().starts_with("source_sha"))
        .expect("source_sha line missing");
    assert!(
        line.contains('{') && line.contains('}'),
        "source_sha must be an inline TOML table for multi-source pins: {:?}",
        line
    );
    for source in [
        "huggingface_api",
        "ollama_library",
        "lmsys_leaderboard",
        "vllm",
        "llama_cpp",
        "mlx_lm",
    ] {
        assert!(
            line.contains(source),
            "source_sha inline-table must pin `{}` (line: {:?})",
            source,
            line
        );
    }
}

#[test]
fn gate_3_fill_ratio_is_measured_and_at_least_floor() {
    let m = manifest_text();
    let raw = extract_after(&m, "\nfill_ratio ").or_else(|| extract_after(&m, "\nfill_ratio="));
    let ratio: f64 = raw
        .as_deref()
        .expect("[parity] fill_ratio must be present")
        .parse()
        .expect("fill_ratio must parse as float");
    assert!(
        ratio >= 0.95,
        "cave-llm-tracker parity floor: fill_ratio must be >= 0.95 (got {}). \
         Either improve coverage or document scope-cuts as [[skipped]].",
        ratio
    );
    assert!(ratio <= 1.0, "fill_ratio must be a fraction (got {})", ratio);
}

#[test]
fn gate_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let v = extract_after(&m, "\nparity_ratio_source ")
        .or_else(|| extract_after(&m, "\nparity_ratio_source="));
    assert_eq!(
        v.as_deref(),
        Some("manifest"),
        "parity_ratio_source must be \"manifest\" so the workspace parity-index \
         reads fill_ratio from this file rather than an external audit doc"
    );
}

#[test]
fn gate_5_last_audit_is_2026_05_21() {
    let m = manifest_text();
    let when = extract_after(&m, "\nlast_audit ").or_else(|| extract_after(&m, "\nlast_audit="));
    assert_eq!(
        when.as_deref(),
        Some("2026-05-21"),
        "[parity] last_audit must reflect the 2026-05-21 close-out"
    );
}

#[test]
fn gate_6_mapped_partial_skipped_unmapped_sum_to_total_with_floor() {
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
        mapped >= 11,
        "cave-llm-tracker MVP floor: >= 11 mapped subsystems (got {})",
        mapped
    );
    assert_eq!(
        unmapped, 0,
        "Charter v2 honest-fill: all subsystems must be classified (got {} unmapped)",
        unmapped
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
    assert!(
        offenders.is_empty(),
        "Charter v2 no-stub gate failed: {:?}",
        offenders
    );
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
        total >= 9,
        "expected >= 9 .rs files in cave-llm-tracker; got {}",
        total
    );
}

#[test]
fn gate_9_default_config_enforces_phase_0_and_wires_all_four_sources() {
    // Phase 0 mandate: auto_swap must be hard-wired off, and the four
    // sources must all be enabled out of the box. The deterministic
    // seed catalog must also contain >= 5 rows so `--mode report`
    // always emits a useful candidate list.
    let cfg = cave_llm_tracker::default_config();
    assert!(
        !cfg.selection.auto_swap,
        "Phase 0 mandate: auto_swap must be false in the default config"
    );
    let s = &cfg.sources;
    assert!(s.huggingface, "HuggingFace source must default on");
    assert!(s.ollama_library, "Ollama library source must default on");
    assert!(s.lmsys_leaderboard, "LMSys leaderboard source must default on");
    assert!(s.github_backend_releases, "GitHub backend source must default on");

    let seed = cave_llm_tracker::seed_catalog();
    assert!(
        seed.len() >= 5,
        "seed catalog must guarantee >= 5 candidates for offline reports; got {}",
        seed.len()
    );

    let prompts = cave_llm_tracker::cave_prompts();
    assert_eq!(prompts.len(), 5, "cave-specific bench is fixed at 5 prompts");
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
