// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Detection engine — single-pass line scan with allowlist, keyword
//! pre-filter, regex match, optional entropy gate, and secret redaction.
//!
//! Mirrors `detect/detect.go::Detector.detectRule` upstream (`v8.29.1`).
//! Out-of-scope this MVP:
//! - Decoded-payload chains (base64 / gzip auto-decode)
//! - Multi-line "block" rules
//! - Stopword pruning

use std::path::{Path, PathBuf};

use crate::config::Allowlist;
use crate::finding::{Finding, redact};
use crate::rule::Rule;

/// Stateful scanner. Holds compiled rules + global allowlist.
///
/// Upstream type: `detect.Detector`.
#[derive(Debug)]
pub struct Detector {
    pub rules: Vec<Rule>,
    pub allowlist: Allowlist,
    /// If true, redact the `Match` field as well as `Secret`. Default
    /// `true` matches upstream `--redact=100` flag semantics.
    pub redact_match: bool,
}

impl Detector {
    /// Build from explicit rules + global allowlist.
    pub fn new(rules: Vec<Rule>, allowlist: Allowlist) -> Self {
        Self {
            rules,
            allowlist,
            redact_match: true,
        }
    }

    /// Build from the built-in rule pack and a default (empty) allowlist.
    pub fn with_builtins() -> Self {
        Self {
            rules: crate::rule::builtin_rules(),
            allowlist: Allowlist::default(),
            redact_match: true,
        }
    }

    /// Scan a string (single file or in-memory blob). `path` is the
    /// reporting filename; pass `""` for stdin.
    ///
    /// Two-phase loop mirrors upstream:
    /// 1. Path-level allowlist short-circuit.
    /// 2. Per-line, per-rule check with keyword pre-filter then regex.
    pub fn scan_str(&self, path: &str, content: &str) -> Vec<Finding> {
        if !path.is_empty() && self.allowlist.path_allowed(path) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (line_idx, line) in content.lines().enumerate() {
            for rule in &self.rules {
                self.check_rule_on_line(rule, path, line, line_idx, &mut out);
            }
        }
        out
    }

    /// Scan a file path on disk.
    pub fn scan_file(&self, path: &Path) -> std::io::Result<Vec<Finding>> {
        let content = std::fs::read_to_string(path)?;
        Ok(self.scan_str(&path.display().to_string(), &content))
    }

    /// Walk a working tree, scanning every regular UTF-8 file under
    /// `root` that survives path-allowlist filtering. Binary files and
    /// I/O errors are skipped silently — matches upstream `--no-banner`
    /// behaviour where unreadable files don't abort the run.
    pub fn scan_working_tree(&self, root: &Path) -> Vec<Finding> {
        let mut out = Vec::new();
        let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                let name = p.file_name().map(|s| s.to_string_lossy().to_string());
                // Skip VCS metadata (upstream defaults).
                if matches!(name.as_deref(), Some(".git" | ".svn" | ".hg")) {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(findings) = self.scan_file(&p) {
                    out.extend(findings);
                }
            }
        }
        out
    }

    fn check_rule_on_line(
        &self,
        rule: &Rule,
        path: &str,
        line: &str,
        line_idx: usize,
        out: &mut Vec<Finding>,
    ) {
        // Per-rule path scope.
        if let Some(re) = &rule.path
            && !path.is_empty()
            && !re.is_match(path)
        {
            return;
        }
        // Cheap keyword pre-filter.
        if !rule.keyword_matches(line) {
            return;
        }
        // Regex match.
        let Some(caps) = rule.regex.captures(line) else {
            return;
        };
        let full = caps.get(0).unwrap();
        let raw_secret_str: String = match rule.secret_group {
            Some(g) => caps
                .get(g)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| full.as_str().to_string()),
            None => full.as_str().to_string(),
        };
        // Per-rule + global secret allowlist.
        if rule.allowlist.secret_allowed(&raw_secret_str)
            || self.allowlist.secret_allowed(&raw_secret_str)
        {
            return;
        }
        // Optional entropy gate.
        let entropy = shannon_entropy(&raw_secret_str);
        if let Some(floor) = rule.entropy
            && (entropy as f64) < floor
        {
            return;
        }
        // Redact for the report.
        let secret_redacted = redact(&raw_secret_str);
        let match_text = if self.redact_match {
            redact(full.as_str())
        } else {
            full.as_str().to_string()
        };
        let mut f = Finding {
            description: rule.description.clone(),
            start_line: line_idx + 1,
            end_line: line_idx + 1,
            start_column: full.start() + 1,
            end_column: full.end() + 1,
            match_text,
            secret: secret_redacted,
            file: path.to_string(),
            symlink_file: String::new(),
            commit: String::new(),
            entropy,
            author: String::new(),
            email: String::new(),
            date: String::new(),
            message: String::new(),
            tags: Vec::new(),
            rule_id: rule.id.clone(),
            fingerprint: String::new(),
        };
        f.fingerprint = f.compute_fingerprint();
        out.push(f);
    }
}

/// Shannon entropy of a byte string, in bits per symbol. Pure function;
/// `detect/utils.go::shannonEntropy` ported verbatim.
pub fn shannon_entropy(s: &str) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let mut freq = [0u32; 256];
    for b in s.bytes() {
        freq[b as usize] += 1;
    }
    let len = s.len() as f32;
    let mut h = 0.0_f32;
    for &c in freq.iter() {
        if c > 0 {
            let p = c as f32 / len;
            h -= p * p.log2();
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_finds_nothing() {
        let d = Detector::with_builtins();
        assert!(d.scan_str("f", "").is_empty());
    }

    #[test]
    fn aws_key_is_detected_and_redacted() {
        let d = Detector::with_builtins();
        let content = "let aws = \"AKIAIOSFODNN7EXAMPLE\";";
        let findings = d.scan_str("src/main.rs", content);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, "aws-access-token");
        assert!(f.secret.starts_with("AKIA"));
        // After the 4-char prefix only asterisks survive — payload never leaks.
        assert!(f.secret[4..].chars().all(|c| c == '*'));
        assert_eq!(f.start_line, 1);
        assert!(f.start_column > 0);
    }

    #[test]
    fn multiple_rules_on_one_line_each_emit_a_finding() {
        let d = Detector::with_builtins();
        // github-pat = `ghp_` + 36 alnum: 26 letters + 10 digits = 36 exactly.
        let content = "AKIAIOSFODNN7EXAMPLE ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let ids: Vec<String> = d
            .scan_str("f", content)
            .into_iter()
            .map(|f| f.rule_id)
            .collect();
        assert!(ids.contains(&"aws-access-token".to_string()));
        assert!(ids.contains(&"github-pat".to_string()));
    }

    #[test]
    fn path_allowlist_skips_files_globally() {
        let mut d = Detector::with_builtins();
        d.allowlist = Allowlist {
            description: String::new(),
            paths: vec![regex::Regex::new("testdata/.*").unwrap()],
            regexes: vec![],
            commits: vec![],
        };
        let content = "AKIAIOSFODNN7EXAMPLE";
        assert!(d.scan_str("testdata/dump.txt", content).is_empty());
        assert_eq!(d.scan_str("src/main.rs", content).len(), 1);
    }

    #[test]
    fn secret_allowlist_skips_known_dummy_values() {
        let mut d = Detector::with_builtins();
        d.allowlist = Allowlist {
            description: String::new(),
            paths: vec![],
            regexes: vec![regex::Regex::new("EXAMPLE").unwrap()],
            commits: vec![],
        };
        assert!(d.scan_str("f", "AKIAIOSFODNN7EXAMPLE").is_empty());
    }

    #[test]
    fn keyword_prefilter_short_circuits_unrelated_lines() {
        // Build a stripped detector with only github-pat — a line not
        // containing the keyword must early-out before regex.
        let only_gh: Vec<_> = crate::rule::builtin_rules()
            .into_iter()
            .filter(|r| r.id == "github-pat")
            .collect();
        let d = Detector::new(only_gh, Allowlist::default());
        assert!(d.scan_str("f", "lorem ipsum dolor sit amet").is_empty());
    }

    #[test]
    fn entropy_gate_blocks_low_entropy_generic_match() {
        // generic-api-key has entropy floor 3.5 on group 2.
        let d = Detector::with_builtins();
        // Repeating chars → low entropy.
        let low = "api_key = \"aaaaaaaaaaaaaaaaaaaa\"";
        let any_generic = d
            .scan_str("f", low)
            .into_iter()
            .any(|f| f.rule_id == "generic-api-key");
        assert!(!any_generic, "low-entropy match should be gated out");
    }

    #[test]
    fn entropy_gate_admits_high_entropy_generic_match() {
        let d = Detector::with_builtins();
        let high = "api_key = \"x9F2pQ7kL1aN8sB4mZ3vR6tY0wH5cE\"";
        let any_generic = d
            .scan_str("f", high)
            .into_iter()
            .any(|f| f.rule_id == "generic-api-key");
        assert!(any_generic);
    }

    #[test]
    fn shannon_entropy_known_values() {
        assert_eq!(shannon_entropy(""), 0.0);
        assert_eq!(shannon_entropy("a"), 0.0);
        // "ab" → uniform 2-symbol distribution → 1.0 bit per symbol.
        assert!((shannon_entropy("ab") - 1.0).abs() < 1e-5);
    }

    #[test]
    fn scan_working_tree_finds_secret_in_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("leak.txt");
        std::fs::write(&p, "AKIAIOSFODNN7EXAMPLE\n").unwrap();
        let d = Detector::with_builtins();
        let findings = d.scan_working_tree(tmp.path());
        assert!(findings.iter().any(|f| f.rule_id == "aws-access-token"));
    }
}
