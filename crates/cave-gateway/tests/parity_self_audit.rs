// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Charter v2 self-audit — cave-gateway must carry an honest, measured
//! `fill_ratio` against upstream Kong 3.9.1 + Gravitee 4.11.7, pinned
//! `source_sha`s for reproducibility, the 2026-05-23 close-out audit date,
//! `parity_ratio_source = "manifest"`, 100% AGPL SPDX header coverage,
//! no stub macros in `src/`, mapped+partial+skipped+unmapped summing to
//! total, and the Kong admin / proxy / plugin chain + Gravitee management
//! surfaces reachable through `cave_gateway`.
//!
//! 9 assertions — one per gate of the close-out checklist.

use std::fs;
use std::path::{Path, PathBuf};

const TODAY: &str = "2026-05-23";
const FLOOR_FILL_RATIO: f64 = 0.95;
const KONG_VERSION: &str = "3.9.1";
const KONG_SHA: &str = "b724fc7154de3a9971e33490097d5ea2c1bae93b";
const GRAVITEE_VERSION: &str = "4.11.7";
const GRAVITEE_SHA: &str = "33ac33b9f4e061e024eaff6efd457fa10bf998e8";

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

fn extract_f64(text: &str, key: &str) -> Option<f64> {
    extract_after(text, &format!("\n{} ", key))
        .or_else(|| extract_after(text, &format!("\n{}=", key)))
        .and_then(|s| s.parse::<f64>().ok())
}

fn extract_u64(text: &str, key: &str) -> Option<u64> {
    extract_after(text, &format!("\n{} ", key))
        .or_else(|| extract_after(text, &format!("\n{}=", key)))
        .and_then(|s| s.parse::<u64>().ok())
}

// ─── Assertion 1: Kong upstream pinned to 3.9.1 (always-latest gate) ────────

#[test]
fn assertion_1_kong_version_pinned() {
    let m = manifest_text();
    let v = extract_after(&m, "\nversion ").or_else(|| extract_after(&m, "\nversion="));
    assert_eq!(
        v.as_deref(),
        Some(KONG_VERSION),
        "[upstream] version must pin Kong {} — Charter v2 always-latest gate (got {:?})",
        KONG_VERSION,
        v
    );
}

// ─── Assertion 2: source_sha pins for both Kong + Gravitee ──────────────────

#[test]
fn assertion_2_source_sha_matches_versions() {
    let m = manifest_text();
    assert!(
        m.contains(KONG_SHA),
        "[upstream] Kong source_sha must contain {} (full manifest text scan)",
        KONG_SHA
    );
    assert!(
        m.contains(GRAVITEE_VERSION),
        "[[upstreams]] companion Gravitee version {} must be pinned",
        GRAVITEE_VERSION
    );
    assert!(
        m.contains(GRAVITEE_SHA),
        "[[upstreams]] companion Gravitee source_sha {} must be pinned",
        GRAVITEE_SHA
    );
}

// ─── Assertion 3: fill_ratio >= 0.95 ────────────────────────────────────────

#[test]
fn assertion_3_fill_ratio_meets_floor() {
    let m = manifest_text();
    let ratio = extract_f64(&m, "fill_ratio").expect("[parity].fill_ratio must be present");
    assert!(
        ratio >= FLOOR_FILL_RATIO,
        "fill_ratio = {} must be >= {} (Charter v2 ≥0.95 close floor)",
        ratio,
        FLOOR_FILL_RATIO
    );
}

// ─── Assertion 4: parity_ratio_source = "manifest" ──────────────────────────

#[test]
fn assertion_4_parity_ratio_source_is_manifest() {
    let m = manifest_text();
    let src = extract_after(&m, "parity_ratio_source ")
        .or_else(|| extract_after(&m, "parity_ratio_source="));
    assert_eq!(
        src.as_deref(),
        Some("manifest"),
        "[parity].parity_ratio_source must be \"manifest\" (got {:?})",
        src
    );
}

// ─── Assertion 5: last_audit = today ────────────────────────────────────────

#[test]
fn assertion_5_last_audit_is_today() {
    let m = manifest_text();
    let when = extract_after(&m, "last_audit ").or_else(|| extract_after(&m, "last_audit="));
    assert_eq!(
        when.as_deref(),
        Some(TODAY),
        "[parity].last_audit must equal {} (got {:?})",
        TODAY,
        when
    );
}

// ─── Assertion 6: counts sum to total + ≥ 15 mapped ─────────────────────────

#[test]
fn assertion_6_counts_sum_to_total() {
    let m = manifest_text();
    let mapped = extract_u64(&m, "mapped_count").expect("mapped_count");
    let partial = extract_u64(&m, "partial_count").expect("partial_count");
    let skipped = extract_u64(&m, "skipped_count").expect("skipped_count");
    let unmapped = extract_u64(&m, "unmapped_count").expect("unmapped_count");
    let total = extract_u64(&m, "total").expect("total");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped+partial+skipped+unmapped ({}+{}+{}+{} = {}) must equal total {}",
        mapped,
        partial,
        skipped,
        unmapped,
        mapped + partial + skipped + unmapped,
        total
    );
    assert!(
        mapped >= 15,
        "mapped_count = {} must be >= 15 (Charter v2 honest-mapped floor)",
        mapped
    );
}

// ─── Assertion 7: 100% AGPL SPDX header coverage in src/ + tests/ ───────────

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let p = entry.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn assertion_7_agpl_spdx_header_coverage() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect_rs(&root.join("src"), &mut files);
    collect_rs(&root.join("tests"), &mut files);
    let needle = "SPDX-License-Identifier: AGPL-3.0-or-later";
    let mut missing = Vec::new();
    for f in &files {
        let head = fs::read_to_string(f).unwrap_or_default();
        let head = head.lines().take(5).collect::<Vec<_>>().join("\n");
        if !head.contains(needle) {
            missing.push(f.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "{}/{} files missing AGPL SPDX header:\n  {}",
        missing.len(),
        files.len(),
        missing.join("\n  ")
    );
    assert!(!files.is_empty(), "expected to scan some .rs files");
}

// ─── Assertion 8: no stub macros in src/ ────────────────────────────────────

#[test]
fn assertion_8_no_stub_macros_in_src() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    let needles = ["todo!(", "unimplemented!(", "panic!(\"stub", "panic!(\"todo"];
    let mut hits = Vec::new();
    for f in &files {
        let text = fs::read_to_string(f).unwrap_or_default();
        // Skip occurrences inside line comments or string contexts is approximate;
        // good enough for a charter gate that bans literal source occurrences.
        for (n, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }
            for needle in needles {
                if line.contains(needle) {
                    hits.push(format!("{}:{}", f.display(), n + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "stub macros found in src/ — Charter v2 gate forbids them:\n  {}",
        hits.join("\n  ")
    );
}

// ─── Assertion 9: Kong + Gravitee surface integrity ─────────────────────────

#[test]
fn assertion_9_gateway_surface_intact() {
    // Compile-time re-export check — these must exist on the crate root
    // (or one level in) so cave-runtime can integrate without spelunking.
    use cave_gateway as gw;

    // Build the gateway state — proves the public construction surface works.
    let state = gw::GatewayState::new();

    // Kong admin / proxy routers reachable through the crate root.
    let _admin: axum::Router = gw::admin_router(state.clone());
    let _proxy: axum::Router = gw::proxy_router(state.clone());
    let _grav: axum::Router = gw::gravitee_router(state);

    // Plugin pipeline type reachable.
    let plugins_mod: &str = std::any::type_name::<gw::plugins::PluginCtx>();
    assert!(
        plugins_mod.contains("PluginCtx"),
        "plugins::PluginCtx must be reachable (got {})",
        plugins_mod
    );

    // Gravitee management types reachable (ApiDef + Application + Subscription).
    let api_def: &str = std::any::type_name::<gw::gravitee::apis::ApiDef>();
    assert!(
        api_def.contains("ApiDef"),
        "gravitee::apis::ApiDef must be reachable (got {})",
        api_def
    );
    let sub: &str = std::any::type_name::<gw::gravitee::apis::Subscription>();
    assert!(
        sub.contains("Subscription"),
        "gravitee::apis::Subscription must be reachable (got {})",
        sub
    );
}
