// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Copy-paste detector (CPD) — parity with
//! `sonar-scanner-engine/src/main/java/org/sonar/scanner/cpd/CpdExecutor.java`
//! and `org.sonar.duplications.block.BlockChunker` (SonarQube v10.4.1).
//!
//! Block-based duplicate detection: tokenize each file, slide a window
//! of `block_size` tokens, fingerprint each window with a rolling hash,
//! group identical fingerprints across files. Reports back to
//! Sonar-style `Duplication` records (file, start_line, end_line).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicationOccurrence {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DuplicationGroup {
    pub fingerprint: u64,
    pub block_size: usize,
    pub occurrences: Vec<DuplicationOccurrence>,
}

#[derive(Debug, Clone)]
pub struct CpdConfig {
    pub block_size: usize,
    pub min_occurrences: usize,
}

impl Default for CpdConfig {
    fn default() -> Self {
        // Sonar default block sizes (per language) bucket around 10 tokens.
        Self {
            block_size: 10,
            min_occurrences: 2,
        }
    }
}

/// Tokenize source, normalising identifiers (replaced with `ID`) and
/// numeric / string literals (replaced with `LIT`) so renaming doesn't
/// defeat detection — same approach Sonar's `Tokenizer` takes.
fn tokenize(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let mut iter = line.chars().peekable();
        let mut buf = String::new();
        while let Some(&ch) = iter.peek() {
            if ch.is_whitespace() {
                if !buf.is_empty() {
                    out.push((i + 1, normalize(&buf)));
                    buf.clear();
                }
                iter.next();
            } else if ch.is_alphanumeric() || ch == '_' {
                buf.push(ch);
                iter.next();
            } else if ch == '"' || ch == '\'' {
                if !buf.is_empty() {
                    out.push((i + 1, normalize(&buf)));
                    buf.clear();
                }
                let quote = ch;
                iter.next();
                while let Some(c) = iter.next() {
                    if c == quote {
                        break;
                    }
                }
                out.push((i + 1, "LIT".into()));
            } else {
                if !buf.is_empty() {
                    out.push((i + 1, normalize(&buf)));
                    buf.clear();
                }
                out.push((i + 1, ch.to_string()));
                iter.next();
            }
        }
        if !buf.is_empty() {
            out.push((i + 1, normalize(&buf)));
        }
    }
    out
}

fn normalize(tok: &str) -> String {
    if tok.chars().all(|c| c.is_ascii_digit()) {
        return "LIT".into();
    }
    const KEYWORDS: &[&str] = &[
        "if", "else", "while", "for", "return", "fn", "function", "def", "class",
        "struct", "enum", "let", "var", "const", "true", "false", "null", "None",
        "import", "from", "as", "match", "switch", "case", "break", "continue",
    ];
    if KEYWORDS.contains(&tok) {
        return tok.to_string();
    }
    if tok.chars().any(|c| c.is_ascii_digit()) && tok.chars().all(|c| c.is_alphanumeric()) {
        // identifier with digit — still an identifier
        return "ID".into();
    }
    if tok.chars().all(|c| c.is_alphabetic() || c == '_') {
        // pure identifier
        return "ID".into();
    }
    tok.to_string()
}

/// FNV-1a 64-bit — stable enough for hash buckets without needing a heavy crate.
fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn block_fingerprint(tokens: &[(usize, String)], start: usize, size: usize) -> u64 {
    let mut s = String::new();
    for i in 0..size {
        s.push_str(&tokens[start + i].1);
        s.push('|');
    }
    fnv1a64(&s)
}

/// Scan a multi-file corpus and return duplication groups.
pub fn detect(files: &[(String, String)], cfg: &CpdConfig) -> Vec<DuplicationGroup> {
    let mut by_fp: HashMap<u64, Vec<DuplicationOccurrence>> = HashMap::new();
    for (path, src) in files {
        let toks = tokenize(src);
        if toks.len() < cfg.block_size {
            continue;
        }
        for i in 0..=toks.len() - cfg.block_size {
            let fp = block_fingerprint(&toks, i, cfg.block_size);
            let start_line = toks[i].0;
            let end_line = toks[i + cfg.block_size - 1].0;
            by_fp.entry(fp).or_default().push(DuplicationOccurrence {
                file: path.clone(),
                start_line,
                end_line,
            });
        }
    }
    let mut groups: Vec<DuplicationGroup> = by_fp
        .into_iter()
        .filter(|(_, v)| v.len() >= cfg.min_occurrences)
        .map(|(fp, occs)| DuplicationGroup {
            fingerprint: fp,
            block_size: cfg.block_size,
            occurrences: occs,
        })
        .collect();
    groups.sort_by_key(|g| g.fingerprint);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_blocks_across_files_detected() {
        let src_a = "fn alpha() { return compute_value(x, y, z); }\n";
        let src_b = "fn beta() { return compute_value(x, y, z); }\n";
        let groups = detect(
            &[
                ("a.rs".into(), src_a.into()),
                ("b.rs".into(), src_b.into()),
            ],
            &CpdConfig {
                block_size: 6,
                min_occurrences: 2,
            },
        );
        assert!(!groups.is_empty(), "identical blocks must be detected");
    }

    #[test]
    fn distinct_blocks_no_match() {
        let src_a = "fn alpha() { return x + y; }\n";
        let src_b = "fn beta() { while_loop_until_done(); }\n";
        let groups = detect(
            &[
                ("a.rs".into(), src_a.into()),
                ("b.rs".into(), src_b.into()),
            ],
            &CpdConfig {
                block_size: 8,
                min_occurrences: 2,
            },
        );
        assert!(groups.is_empty());
    }

    #[test]
    fn short_file_under_block_size_skipped() {
        let src = "x\n";
        let groups = detect(
            &[("a.rs".into(), src.into())],
            &CpdConfig {
                block_size: 10,
                min_occurrences: 2,
            },
        );
        assert!(groups.is_empty());
    }

    #[test]
    fn tokenize_strips_string_literal() {
        let toks = tokenize(r#"let x = "secret";"#);
        let lits: Vec<&String> = toks.iter().map(|(_, t)| t).filter(|t| t == &"LIT").collect();
        assert_eq!(lits.len(), 1);
    }

    #[test]
    fn tokenize_strips_number_literal() {
        let toks = tokenize("let x = 12345;");
        let lits: Vec<&String> = toks.iter().map(|(_, t)| t).filter(|t| t == &"LIT").collect();
        assert_eq!(lits.len(), 1);
    }

    #[test]
    fn keyword_preserved_after_normalize() {
        assert_eq!(normalize("return"), "return");
        assert_eq!(normalize("foo"), "ID");
        assert_eq!(normalize("42"), "LIT");
    }

    #[test]
    fn rename_does_not_defeat_match() {
        let src_a = "fn alpha() { return compute(x, y); }\n";
        let src_b = "fn renamed_func() { return compute(z, w); }\n";
        let groups = detect(
            &[
                ("a.rs".into(), src_a.into()),
                ("b.rs".into(), src_b.into()),
            ],
            &CpdConfig {
                block_size: 6,
                min_occurrences: 2,
            },
        );
        assert!(
            !groups.is_empty(),
            "renamed identifiers should still be detected via normalization"
        );
    }

    #[test]
    fn min_occurrences_filter_applies() {
        let src_a = "let x = y;\n";
        let groups = detect(
            &[("a.rs".into(), src_a.into())],
            &CpdConfig {
                block_size: 3,
                min_occurrences: 2,
            },
        );
        // Only one file → no duplication can exist.
        assert!(groups.is_empty());
    }
}
