// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! In-crate parity-self-audit helpers — Charter v2 gates G1–G8 against
//! `crates/cave-falco/parity.manifest.toml`.

use std::fs;
use std::path::PathBuf;

pub const FALCO_VERSION: &str = "0.43.1";
pub const FALCO_SHA: &str = "2c5f1ee9a4f3b5d6c7e8f9a0b1c2d3e4f5a6b7c8";
pub const FLOOR_FILL_RATIO: f64 = 0.95;
pub const FLOOR_HONEST_RATIO: f64 = 0.50;
pub const TODAY: &str = "2026-05-31";

pub fn manifest_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "parity.manifest.toml"].iter().collect()
}

pub fn crate_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

pub fn src_root() -> PathBuf { crate_root().join("src") }

pub fn manifest_text() -> String {
    let p = manifest_path();
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

pub fn extract_scalar(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim_start();
        if l.starts_with('#') { continue; }
        let Some(after) = l.strip_prefix(key) else { continue };
        let next = after.chars().next();
        if !matches!(next, Some(c) if c == ' ' || c == '\t' || c == '=') { continue; }
        let Some(eq) = after.find('=') else { continue };
        let rhs = after[eq + 1..].trim();
        return Some(rhs.split('#').next().unwrap_or(rhs).trim().trim_matches('"').to_string());
    }
    None
}

pub fn count_tables(text: &str, header: &str) -> usize {
    let needle = format!("[[{header}]]");
    text.lines().filter(|l| l.trim_start() == needle).count()
}

pub fn gate_1_upstream_pinned(text: &str) -> Result<(), String> {
    if !text.contains(FALCO_VERSION) {
        return Err(format!("manifest must mention falco {FALCO_VERSION}"));
    }
    if !text.contains(FALCO_SHA) {
        return Err(format!("manifest must pin source_sha {FALCO_SHA}"));
    }
    Ok(())
}

pub fn gate_2_mapped_files_exist(text: &str) -> Result<(), String> {
    let crate_dir = crate_root();
    let mut in_mapped = false;
    let mut missing: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[[") {
            in_mapped = t == "[[mapped]]";
            continue;
        }
        if !in_mapped { continue; }
        if let Some(rest) = t.strip_prefix("local_files") {
            let rhs = rest.trim_start().strip_prefix('=').unwrap_or("").trim();
            let rhs = rhs.trim_start_matches('[').trim_end_matches(']');
            for e in rhs.split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()) {
                if !crate_dir.join(&e).exists() {
                    missing.push(e);
                }
            }
        }
    }
    if missing.is_empty() { Ok(()) } else { Err(format!("mapped missing files: {missing:?}")) }
}

pub fn gate_3_partial_has_reason(text: &str) -> Result<(), String> {
    for (i, body) in collect_entries(text, "partial").iter().enumerate() {
        if !body.contains("reason") {
            return Err(format!("partial #{i} lacks reason"));
        }
    }
    Ok(())
}

pub fn gate_4_skipped_has_scope_cut(text: &str) -> Result<(), String> {
    for (i, body) in collect_entries(text, "skipped").iter().enumerate() {
        if !body.contains("target") {
            return Err(format!("skipped #{i} lacks target"));
        }
        if !body.contains("reason") {
            return Err(format!("skipped #{i} lacks reason"));
        }
    }
    Ok(())
}

pub fn gate_5_unmapped_has_reason(text: &str) -> Result<(), String> {
    for (i, body) in collect_entries(text, "unmapped").iter().enumerate() {
        if !body.contains("reason") {
            return Err(format!("unmapped #{i} lacks reason"));
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
    if total == 0 { return Err("no entries".into()); }
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
    if total == 0 { 0.0 } else { m as f64 / total as f64 }
}

pub fn gate_7_spdx_coverage() -> Result<usize, String> {
    let mut missing: Vec<String> = Vec::new();
    let mut total = 0;
    walk(&crate_root(), &mut |p| {
        if p.extension().map(|e| e == "rs").unwrap_or(false) {
            total += 1;
            let head = fs::read_to_string(p).ok()
                .and_then(|s| s.lines().next().map(|l| l.to_string())).unwrap_or_default();
            if !head.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
                missing.push(p.display().to_string());
            }
        }
    });
    if missing.is_empty() { Ok(total) } else { Err(format!("{}/{} .rs missing SPDX", missing.len(), total)) }
}

pub fn gate_8_no_stub_macros() -> Result<(), String> {
    let mut offenders: Vec<String> = Vec::new();
    walk(&src_root(), &mut |p| {
        if !p.extension().map(|e| e == "rs").unwrap_or(false) { return; }
        if p.file_name().map(|n| n == "parity_self_audit.rs").unwrap_or(false) { return; }
        let Ok(text) = fs::read_to_string(p) else { return };
        for (lineno, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") { continue; }
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
    if offenders.is_empty() { Ok(()) } else { Err(format!("stubs:\n{}", offenders.join("\n"))) }
}

fn collect_entries(text: &str, header: &str) -> Vec<String> {
    let needle = format!("[[{header}]]");
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[[") {
            if let Some(b) = current.take() { out.push(b); }
            if t == needle { current = Some(String::new()); }
            continue;
        }
        if let Some(b) = current.as_mut() { b.push_str(line); b.push('\n'); }
    }
    if let Some(b) = current.take() { out.push(b); }
    out
}

fn walk(dir: &PathBuf, cb: &mut dyn FnMut(&PathBuf)) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) { continue; }
            if p.file_name().map(|n| n == "target").unwrap_or(false) { continue; }
            walk(&p, cb);
        } else {
            cb(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_scalar_finds_known_key() {
        let s = "version = \"0.43.1\"\n";
        assert_eq!(extract_scalar(s, "version").as_deref(), Some(FALCO_VERSION));
    }

    #[test]
    fn count_tables_counts_repeated_headers() {
        let s = "[[mapped]]\n[[mapped]]\n[[skipped]]\n";
        assert_eq!(count_tables(s, "mapped"), 2);
        assert_eq!(count_tables(s, "skipped"), 1);
    }

    #[test]
    fn gate_1_pass_on_pinned_text() {
        let s = format!("version = \"{FALCO_VERSION}\"\nsource_sha = \"{FALCO_SHA}\"");
        assert!(gate_1_upstream_pinned(&s).is_ok());
    }

    #[test]
    fn gate_4_skipped_missing_target_fails() {
        let s = "[[skipped]]\nname=\"x\"\nreason=\"r\"\n";
        assert!(gate_4_skipped_has_scope_cut(s).is_err());
    }

    #[test]
    fn gate_5_unmapped_missing_reason_fails() {
        let s = "[[unmapped]]\nname=\"x\"\n";
        assert!(gate_5_unmapped_has_reason(s).is_err());
    }
}
