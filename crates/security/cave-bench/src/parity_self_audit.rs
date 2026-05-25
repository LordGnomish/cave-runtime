// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-crate parity-self-audit helpers (also used by the integration test).
//!
//! Charter v2 gates G1–G8 against the `parity.manifest.toml` shipped here.
//! kube-bench + kubescape are dual-upstream, so G1 pin checks both source_shas.

use std::fs;
use std::path::PathBuf;

pub const KUBE_BENCH_VERSION: &str = "v0.15.5";
pub const KUBE_BENCH_SHA: &str = "13c5a2bed634b4f324ad54ba2942f4a77fc802e0";
pub const KUBESCAPE_VERSION: &str = "v4.0.8";
pub const KUBESCAPE_SHA: &str = "d7539c2264560a8685f59e89a731d6de833258a6";
pub const FLOOR_FILL_RATIO: f64 = 0.95;
pub const FLOOR_HONEST_RATIO: f64 = 0.50;
pub const TODAY: &str = "2026-05-24";

pub fn manifest_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"].iter().collect()
}

pub fn manifest_text() -> String {
    let p = manifest_path();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

/// Extract a top-level scalar value from a TOML body (very small subset).
pub fn extract_scalar(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim_start();
        if l.starts_with('#') {
            continue;
        }
        let Some(after) = l.strip_prefix(key) else { continue };
        let next = after.chars().next();
        if !matches!(next, Some(c) if c == ' ' || c == '\t' || c == '=') {
            continue;
        }
        let Some(eq) = after.find('=') else { continue };
        let rhs = after[eq + 1..].trim();
        return Some(unquote(rhs));
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.split('#').next().unwrap_or(s).trim();
    s.trim_matches('"').to_string()
}

pub fn count_tables(text: &str, header: &str) -> usize {
    let needle = format!("[[{header}]]");
    text.lines().filter(|l| l.trim_start() == needle).count()
}

// ─── Gates ──────────────────────────────────────────────────────────────────

pub fn gate_1_upstream_pinned(text: &str) -> Result<(), String> {
    if !text.contains(KUBE_BENCH_VERSION) {
        return Err(format!("manifest must mention kube-bench {KUBE_BENCH_VERSION}"));
    }
    if !text.contains(KUBE_BENCH_SHA) {
        return Err(format!("manifest must pin kube-bench source_sha {KUBE_BENCH_SHA}"));
    }
    if !text.contains(KUBESCAPE_VERSION) {
        return Err(format!("manifest must mention kubescape {KUBESCAPE_VERSION}"));
    }
    if !text.contains(KUBESCAPE_SHA) {
        return Err(format!("manifest must pin kubescape source_sha {KUBESCAPE_SHA}"));
    }
    Ok(())
}

pub fn gate_2_mapped_files_exist(text: &str) -> Result<(), String> {
    let mut in_mapped = false;
    let mut missing: Vec<String> = Vec::new();
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[[") {
            in_mapped = t == "[[mapped]]";
            continue;
        }
        if !in_mapped {
            continue;
        }
        if let Some(rest) = t.strip_prefix("local_files") {
            let rhs = rest.trim_start();
            let rhs = rhs.strip_prefix('=').unwrap_or(rhs).trim();
            let rhs = rhs.trim_start_matches('[').trim_end_matches(']');
            let entries: Vec<String> = rhs
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            for e in entries {
                let p = crate_dir.join(&e);
                if !p.exists() {
                    missing.push(e);
                }
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("mapped entries reference missing files: {missing:?}"))
    }
}

pub fn gate_3_partial_has_gap_reason(text: &str) -> Result<(), String> {
    let entries = collect_entries(text, "partial");
    for (i, body) in entries.iter().enumerate() {
        if !body.contains("reason ") && !body.contains("reason=") && !body.contains("gap ") {
            return Err(format!("partial #{i} lacks `reason` field"));
        }
    }
    Ok(())
}

pub fn gate_4_skipped_has_scope_cut(text: &str) -> Result<(), String> {
    let entries = collect_entries(text, "skipped");
    for (i, body) in entries.iter().enumerate() {
        if !body.contains("scope_cut_target") && !body.contains("scope_cut_category") && !body.contains("target") {
            return Err(format!("skipped #{i} lacks scope_cut_target / target"));
        }
        if !body.contains("reason") {
            return Err(format!("skipped #{i} lacks reason"));
        }
    }
    Ok(())
}

pub fn gate_5_unmapped_has_reason(text: &str) -> Result<(), String> {
    let entries = collect_entries(text, "unmapped");
    for (i, body) in entries.iter().enumerate() {
        if !body.contains("reason") {
            return Err(format!("unmapped #{i} lacks reason (must be honest gap)"));
        }
    }
    Ok(())
}

pub fn gate_6_fill_ratio(text: &str) -> Result<f64, String> {
    let m = count_tables(text, "mapped");
    let p = count_tables(text, "partial");
    let s = count_tables(text, "skipped");
    let u = count_tables(text, "unmapped");
    let total = m + p + s + u;
    if total == 0 {
        return Err("no subsystem entries in manifest".into());
    }
    let r = (m + p + s) as f64 / total as f64;
    if r < FLOOR_FILL_RATIO {
        return Err(format!("fill_ratio={r} < {FLOOR_FILL_RATIO}"));
    }
    Ok(r)
}

pub fn honest_ratio(text: &str) -> f64 {
    let m = count_tables(text, "mapped");
    let p = count_tables(text, "partial");
    let s = count_tables(text, "skipped");
    let u = count_tables(text, "unmapped");
    let total = m + p + s + u;
    if total == 0 {
        return 0.0;
    }
    m as f64 / total as f64
}

pub fn gate_7_spdx_coverage(root: &PathBuf) -> Result<usize, String> {
    let mut missing: Vec<String> = Vec::new();
    let mut total = 0;
    walk(root, &mut |p| {
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
    if missing.is_empty() {
        Ok(total)
    } else {
        Err(format!("{}/{} .rs files missing AGPL SPDX header: {missing:?}", missing.len(), total))
    }
}

pub fn gate_8_no_stub_macros(src: &PathBuf) -> Result<(), String> {
    let mut offenders: Vec<String> = Vec::new();
    walk(src, &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) {
            return;
        }
        if p.file_name().map(|n| n == "parity_self_audit.rs").unwrap_or(false) {
            return;
        }
        let Ok(text) = fs::read_to_string(p) else { return };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("todo!(")
                || trimmed.contains("unimplemented!(")
                || trimmed.contains("panic!(\"stub")
                || trimmed.contains("panic!(\"not impl")
                || trimmed.contains("panic!(\"not implemented")
            {
                offenders.push(format!("{}:{}: {}", p.display(), lineno + 1, line.trim()));
            }
        }
    });
    if offenders.is_empty() {
        Ok(())
    } else {
        Err(format!("stub macros found:\n{}", offenders.join("\n")))
    }
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn collect_entries(text: &str, header: &str) -> Vec<String> {
    let needle = format!("[[{header}]]");
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[[") {
            if let Some(body) = current.take() {
                out.push(body);
            }
            if t == needle {
                current = Some(String::new());
            }
            continue;
        }
        if let Some(b) = current.as_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    if let Some(body) = current.take() {
        out.push(body);
    }
    out
}

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        format!(
            r#"
[upstream]
version = "{kbv}"
source_sha = "{kbs}"
[[mapped]]
name = "x"
local_files = ["src/lib.rs"]
[[skipped]]
name = "y"
reason = "phase-2"
scope_cut_target = "cave-runtime"
[[unmapped]]
name = "z"
reason = "kernel only"
# mentions kubescape {ksv} ({kss})
"#,
            kbv = KUBE_BENCH_VERSION,
            kbs = KUBE_BENCH_SHA,
            ksv = KUBESCAPE_VERSION,
            kss = KUBESCAPE_SHA,
        )
    }

    #[test]
    fn test_extract_scalar() {
        let s = sample();
        assert_eq!(extract_scalar(&s, "version").as_deref(), Some(KUBE_BENCH_VERSION));
    }

    #[test]
    fn test_count_tables() {
        let s = sample();
        assert_eq!(count_tables(&s, "mapped"), 1);
        assert_eq!(count_tables(&s, "skipped"), 1);
        assert_eq!(count_tables(&s, "unmapped"), 1);
    }

    #[test]
    fn test_gate_1_pass_on_sample() {
        let s = sample();
        assert!(gate_1_upstream_pinned(&s).is_ok());
    }

    #[test]
    fn test_gate_4_skipped_must_have_target() {
        let bad = "[[skipped]]\nname = \"y\"\nreason = \"phase-2\"\n";
        assert!(gate_4_skipped_has_scope_cut(bad).is_err());
    }

    #[test]
    fn test_gate_5_unmapped_must_have_reason() {
        let bad = "[[unmapped]]\nname = \"z\"\n";
        assert!(gate_5_unmapped_has_reason(bad).is_err());
    }

    #[test]
    fn test_honest_ratio_on_sample() {
        let r = honest_ratio(&sample());
        assert!((r - 0.33333).abs() < 0.01);
    }
}
