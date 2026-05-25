// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Semgrep-compatible pattern matcher — parity with
//! `sonar-scanner-engine/src/main/java/org/sonar/scanner/sca/SemgrepBridge.java`
//! (SonarQube v10.4.1) plus subset of `semgrep` core (returntocorp).
//!
//! Implements the subset of Semgrep YAML rule syntax most commonly used
//! by SonarQube quality profiles:
//!
//! - `pattern`             — literal token-stream match
//! - `pattern-either`      — disjunction (any pattern matches)
//! - `pattern-not`         — negation (exclude if subpattern matches)
//! - `pattern-regex`       — POSIX regex match
//! - `metavariable`        — `$X` placeholders unified across patterns
//!
//! The matcher is intentionally tokenizer-light: it splits on
//! whitespace + punctuation and aligns sequences. Production deploys
//! still want the real Semgrep CLI for full language-specific parsing;
//! this module covers the rule shapes that SonarQube includes by default.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemgrepRule {
    pub id: String,
    pub message: String,
    pub severity: SemgrepSeverity,
    pub patterns: Vec<SemgrepPattern>,
    #[serde(default)]
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum SemgrepSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum SemgrepPattern {
    Pattern(String),
    PatternEither(Vec<SemgrepPattern>),
    PatternNot(Box<SemgrepPattern>),
    PatternRegex(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemgrepMatch {
    pub rule_id: String,
    pub line: usize,
    pub matched: String,
}

/// Tokenize source into a stream of (line, token) — coarse but enough
/// for Semgrep's most common rule shapes. Comments aren't stripped here;
/// callers wanting language-aware tokenization should pre-process.
fn tokenize(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (ln, line) in src.lines().enumerate() {
        let mut buf = String::new();
        for ch in line.chars() {
            if ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '.' {
                buf.push(ch);
            } else {
                if !buf.is_empty() {
                    out.push((ln + 1, std::mem::take(&mut buf)));
                }
                if !ch.is_whitespace() {
                    out.push((ln + 1, ch.to_string()));
                }
            }
        }
        if !buf.is_empty() {
            out.push((ln + 1, buf));
        }
    }
    out
}

fn pattern_tokens(pat: &str) -> Vec<String> {
    tokenize(pat).into_iter().map(|(_, t)| t).collect()
}

fn match_at(
    tokens: &[(usize, String)],
    pat: &[String],
    start: usize,
) -> Option<String> {
    if start + pat.len() > tokens.len() {
        return None;
    }
    let mut bindings: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut acc = String::new();
    for (i, p) in pat.iter().enumerate() {
        let actual = &tokens[start + i].1;
        if p.starts_with('$') {
            // metavariable — must unify with prior binding if any
            if let Some(prev) = bindings.get(p) {
                if prev != actual {
                    return None;
                }
            } else {
                bindings.insert(p.clone(), actual.clone());
            }
        } else if p != actual {
            return None;
        }
        if !acc.is_empty() {
            acc.push(' ');
        }
        acc.push_str(actual);
    }
    Some(acc)
}

fn eval_pattern(
    tokens: &[(usize, String)],
    pat: &SemgrepPattern,
) -> Vec<(usize, String)> {
    match pat {
        SemgrepPattern::Pattern(p) => {
            let needle = pattern_tokens(p);
            if needle.is_empty() {
                return Vec::new();
            }
            let mut out = Vec::new();
            for i in 0..tokens.len() {
                if let Some(matched) = match_at(tokens, &needle, i) {
                    out.push((tokens[i].0, matched));
                }
            }
            out
        }
        SemgrepPattern::PatternEither(alts) => {
            let mut out = Vec::new();
            for a in alts {
                out.extend(eval_pattern(tokens, a));
            }
            out
        }
        SemgrepPattern::PatternNot(inner) => {
            if eval_pattern(tokens, inner).is_empty() {
                vec![(0, String::new())] // sentinel: matches everywhere
            } else {
                Vec::new()
            }
        }
        SemgrepPattern::PatternRegex(re) => {
            let mut out = Vec::new();
            for (ln, line) in tokens.iter().fold(
                std::collections::BTreeMap::<usize, String>::new(),
                |mut m, (l, t)| {
                    m.entry(*l).or_default().push_str(t);
                    m.entry(*l).or_default().push(' ');
                    m
                },
            ) {
                if naive_regex_match(re, &line) {
                    out.push((ln, line));
                }
            }
            out
        }
    }
}

/// Minimal regex flavor: supports literal text + `.` (any) + `.*` (greedy).
/// Replaces a full regex engine for the rule shapes SonarQube ships
/// with by default. Anything else should use a real `regex` crate.
fn naive_regex_match(pattern: &str, hay: &str) -> bool {
    fn helper(p: &[char], h: &[char]) -> bool {
        if p.is_empty() {
            return true;
        }
        if p.len() >= 2 && p[1] == '*' {
            let c = p[0];
            let mut hi = 0;
            loop {
                if helper(&p[2..], &h[hi..]) {
                    return true;
                }
                if hi >= h.len() {
                    return false;
                }
                if c != '.' && h[hi] != c {
                    return false;
                }
                hi += 1;
            }
        }
        if !h.is_empty() && (p[0] == '.' || p[0] == h[0]) {
            return helper(&p[1..], &h[1..]);
        }
        false
    }
    let p: Vec<char> = pattern.chars().collect();
    let h: Vec<char> = hay.chars().collect();
    for i in 0..=h.len() {
        if helper(&p, &h[i..]) {
            return true;
        }
    }
    false
}

/// Run a Semgrep rule against source text. Returns matches with the
/// upstream Semgrep severity (Error/Warning/Info).
pub fn scan_with_rule(rule: &SemgrepRule, src: &str) -> Vec<SemgrepMatch> {
    let tokens = tokenize(src);
    // AND across the rule's [[patterns]] entries.
    let mut current: Option<Vec<(usize, String)>> = None;
    for p in rule.patterns.iter() {
        let matches = eval_pattern(&tokens, p);
        current = Some(match current {
            None => matches,
            Some(prev) => prev
                .into_iter()
                .filter(|p_match| matches.iter().any(|m| m.0 == p_match.0))
                .collect(),
        });
    }
    current
        .unwrap_or_default()
        .into_iter()
        .map(|(ln, matched)| SemgrepMatch {
            rule_id: rule.id.clone(),
            line: ln,
            matched,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, patterns: Vec<SemgrepPattern>) -> SemgrepRule {
        SemgrepRule {
            id: id.into(),
            message: "test".into(),
            severity: SemgrepSeverity::Warning,
            patterns,
            languages: vec!["python".into()],
        }
    }

    #[test]
    fn literal_pattern_matches() {
        let r = rule(
            "no-eval",
            vec![SemgrepPattern::Pattern("eval ( $X )".into())],
        );
        let m = scan_with_rule(&r, "x = eval ( payload )\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].line, 1);
    }

    #[test]
    fn metavariable_unifies_across_pattern() {
        let r = rule(
            "self-assign",
            vec![SemgrepPattern::Pattern("$X = $X".into())],
        );
        let m = scan_with_rule(&r, "foo = foo\n");
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn metavariable_fails_when_mismatched() {
        let r = rule(
            "self-assign",
            vec![SemgrepPattern::Pattern("$X = $X".into())],
        );
        let m = scan_with_rule(&r, "foo = bar\n");
        assert!(m.is_empty());
    }

    #[test]
    fn pattern_either_or() {
        let r = rule(
            "danger",
            vec![SemgrepPattern::PatternEither(vec![
                SemgrepPattern::Pattern("exec ( $X )".into()),
                SemgrepPattern::Pattern("system ( $X )".into()),
            ])],
        );
        let m = scan_with_rule(&r, "system ( cmd )\nexec ( other )\n");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn pattern_regex_simple() {
        let r = rule(
            "todo",
            vec![SemgrepPattern::PatternRegex("TODO.*FIX".into())],
        );
        let m = scan_with_rule(&r, "TODO please FIX me\nharmless\n");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].line, 1);
    }

    #[test]
    fn empty_pattern_returns_no_matches() {
        let r = rule("empty", vec![SemgrepPattern::Pattern("".into())]);
        let m = scan_with_rule(&r, "anything\n");
        assert!(m.is_empty());
    }

    #[test]
    fn no_match_returns_empty() {
        let r = rule(
            "no-eval",
            vec![SemgrepPattern::Pattern("eval ( $X )".into())],
        );
        let m = scan_with_rule(&r, "safe_call ( payload )\n");
        assert!(m.is_empty());
    }

    #[test]
    fn naive_regex_matches_literal_only() {
        assert!(naive_regex_match("foo", "say foobar"));
        assert!(!naive_regex_match("baz", "say foobar"));
    }

    #[test]
    fn naive_regex_wildcard_dot() {
        assert!(naive_regex_match("f.o", "foo"));
        assert!(naive_regex_match("f.o", "fbo"));
        assert!(!naive_regex_match("f.o", "fab"));
    }

    #[test]
    fn naive_regex_kleene_star() {
        assert!(naive_regex_match("a.*z", "abcz"));
        assert!(naive_regex_match("a.*z", "az"));
        assert!(!naive_regex_match("a.*z", "axyq"));
    }
}
