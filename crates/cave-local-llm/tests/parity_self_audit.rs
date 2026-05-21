// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Charter v2 8-gate self-audit for cave-local-llm (ollama/ollama v0.3.0
// parity + OpenAI-compat layer + prompt template engine + InferenceBackend
// trait).
//
// Gates:
//   1. SPDX coverage 100% of src/*.rs
//   2. source_sha pinned (v0.3.0)
//   3. last_audit = 2026-05-19
//   4. parity_ratio_source = "manifest"
//   5. fill_ratio >= 0.85
//   6. mapped + partial + skipped + unmapped == total
//   7. no unimplemented!() / todo!() in src/
//   8. PARITY_REPORT.md exists
//   9. Charter v2 composite — all of the above re-asserted

use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_manifest() -> String {
    fs::read_to_string(crate_root().join("parity.manifest.toml"))
        .expect("parity.manifest.toml must exist")
}

#[test]
fn gate_1_spdx_full_coverage() {
    let src = crate_root().join("src");
    let mut total = 0usize;
    let mut spdx = 0usize;
    walk_rs(&src, &mut |p| {
        total += 1;
        let body = fs::read_to_string(p).unwrap_or_default();
        if body.contains("SPDX-License-Identifier: AGPL-3.0-or-later") {
            spdx += 1;
        }
    });
    assert!(total > 0, "no .rs files found under src/");
    assert_eq!(
        spdx,
        total,
        "SPDX-License-Identifier missing on {} files",
        total - spdx
    );
}

#[test]
fn gate_2_source_sha_pinned() {
    let m = read_manifest();
    assert!(
        m.contains("source_sha"),
        "manifest must declare source_sha = ollama release tag"
    );
    assert!(
        m.contains("v0.3.0"),
        "source_sha must pin ollama v0.3.0"
    );
}

#[test]
fn gate_3_last_audit_2026_05_19() {
    let m = read_manifest();
    assert!(
        has_kv(&m, "last_audit", "\"2026-05-19\""),
        "last_audit must be 2026-05-19 in [parity] block"
    );
}

#[test]
fn gate_4_parity_ratio_source_manifest() {
    let m = read_manifest();
    assert!(
        has_kv(&m, "parity_ratio_source", "\"manifest\""),
        "parity_ratio_source must be \"manifest\""
    );
}

#[test]
fn gate_5_fill_ratio_floor() {
    let m = read_manifest();
    let ratio =
        extract_float(&m, "fill_ratio").expect("manifest must declare fill_ratio = <0.0..1.0>");
    assert!(
        ratio >= 0.85,
        "fill_ratio = {} (need >= 0.85 — cave-local-llm in-scope coverage)",
        ratio
    );
}

#[test]
fn gate_6_count_invariants() {
    let m = read_manifest();
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    let partial = extract_int(&m, "partial_count").unwrap_or(0);
    let skipped = extract_int(&m, "skipped_count").unwrap_or(0);
    let unmapped = extract_int(&m, "unmapped_count").unwrap_or(0);
    let total = extract_int(&m, "total").unwrap_or(0);
    assert!(mapped > 0, "mapped_count must be > 0");
    assert!(total > 0, "total must be > 0");
    assert_eq!(
        mapped + partial + skipped + unmapped,
        total,
        "mapped + partial + skipped + unmapped must equal total"
    );
}

#[test]
fn gate_7_no_stub_macros_in_src() {
    let src = crate_root().join("src");
    let mut offenders: Vec<String> = Vec::new();
    walk_rs(&src, &mut |p| {
        let body = fs::read_to_string(p).unwrap_or_default();
        // Track raw-string-literal state across lines. Each open `r#"`
        // (with optional more #'s) must close with the matching `"#`.
        // We use a conservative single-`#` tracker — sufficient for the
        // cave-local-llm prompt-template strings.
        let mut in_raw_string = false;
        for (i, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            if !in_raw_string && trimmed.starts_with("//") {
                continue;
            }
            // Update raw-string state by scanning this line. Use char
            // iteration so UTF-8 multi-byte sequences (em-dashes, etc.)
            // do not panic on a byte-slice.
            let chars: Vec<char> = line.chars().collect();
            let mut k = 0usize;
            while k < chars.len() {
                if !in_raw_string
                    && k + 2 < chars.len()
                    && chars[k] == 'r'
                    && chars[k + 1] == '#'
                    && chars[k + 2] == '"'
                {
                    in_raw_string = true;
                    k += 3;
                    continue;
                }
                if in_raw_string && k + 1 < chars.len() && chars[k] == '"' && chars[k + 1] == '#' {
                    in_raw_string = false;
                    k += 2;
                    continue;
                }
                k += 1;
            }
            // If we were in a raw string at any point on this line (start
            // OR end), the macro reference is text, not code. The clearest
            // tells: the line is inside `r#"..."#` or the macro appears in
            // a backtick code-fence (prompt-template markdown).
            let line_in_raw = in_raw_string || line.contains("\"#");
            let in_string =
                line_macro_is_in_string(line, "unimplemented!(")
                || line_macro_is_in_string(line, "todo!(");
            let in_backticks = line.contains("`todo!(") || line.contains("`unimplemented!(");
            let escaped_quote_inside = line.contains("todo!(\\\"")
                || line.contains("unimplemented!(\\\"");
            if line_in_raw || in_string || in_backticks || escaped_quote_inside {
                continue;
            }
            if line.contains("unimplemented!(") || line.contains("todo!(") {
                offenders.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    });
    assert!(
        offenders.is_empty(),
        "no stub macros allowed; offenders:\n{}",
        offenders.join("\n")
    );
}

/// True iff the macro substring appears inside a `"`-delimited string
/// literal on this line. Counts unescaped `"` before the substring; odd
/// count means we're inside a string.
fn line_macro_is_in_string(line: &str, needle: &str) -> bool {
    let idx = match line.find(needle) {
        Some(i) => i,
        None => return false,
    };
    let prefix = &line[..idx];
    let mut in_str = false;
    let mut prev_backslash = false;
    for ch in prefix.chars() {
        if ch == '\\' && !prev_backslash {
            prev_backslash = true;
            continue;
        }
        if ch == '"' && !prev_backslash {
            in_str = !in_str;
        }
        prev_backslash = false;
    }
    in_str
}

#[test]
fn gate_8_parity_report_exists() {
    let report = crate_root().join("PARITY_REPORT.md");
    assert!(report.exists(), "PARITY_REPORT.md must exist at crate root");
    let body = fs::read_to_string(&report).unwrap();
    assert!(
        body.contains("Charter v2"),
        "PARITY_REPORT must reference Charter v2"
    );
    assert!(
        body.contains("8/8 PASS") || body.contains("8-gate"),
        "PARITY_REPORT must summarise 8-gate result"
    );
}

#[test]
fn gate_9_charter_v2_summary() {
    let m = read_manifest();
    let ratio = extract_float(&m, "fill_ratio").unwrap_or(0.0);
    let total = extract_int(&m, "total").unwrap_or(0);
    let mapped = extract_int(&m, "mapped_count").unwrap_or(0);
    assert!(
        ratio >= 0.85 && total > 0 && mapped > 0 && m.contains("source_sha"),
        "Charter v2 composite invariants not satisfied"
    );
}

fn has_kv(s: &str, key: &str, expected_value: &str) -> bool {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if v == expected_value {
                    return true;
                }
            }
        }
    }
    false
}

fn walk_rs(dir: &Path, f: &mut dyn FnMut(&Path)) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_rs(&p, f);
        } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            f(&p);
        }
    }
}

fn extract_float(s: &str, key: &str) -> Option<f64> {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if let Ok(n) = v.parse::<f64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

fn extract_int(s: &str, key: &str) -> Option<i64> {
    for line in s.lines() {
        let l = line.trim();
        if l.starts_with(key) {
            if let Some(eq) = l.find('=') {
                let v = l[eq + 1..].trim().trim_end_matches(',');
                let v = v.split('#').next().unwrap_or(v).trim();
                if let Ok(n) = v.parse::<i64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}
