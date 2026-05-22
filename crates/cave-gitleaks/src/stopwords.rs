// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Stopwords — anti-false-positive post-match filter.
//!
//! Mirrors `config/gitleaks.toml [stopwords]` upstream (`v8.29.1`). The
//! upstream stoplist is a flat array of strings; any rule match whose
//! captured value (or match text) case-insensitively contains a stopword
//! is dropped. Used heavily to prune `generic-api-key` false positives
//! ("EXAMPLE", "PLACEHOLDER", "FAKE", "test", etc.).

use crate::finding::Finding;

/// Default stopword pack — cherry-picked from upstream
/// `config/gitleaks.toml` for high-signal noise pruning.
pub fn default_stopwords() -> Vec<String> {
    [
        "example",
        "placeholder",
        "fake",
        "dummy",
        "sample",
        "test123",
        "test1234",
        "changeme",
        "12345",
        "secret",
        "yoursecret",
        "yourtoken",
        "your_api_key",
        "yourapikey",
        "todo",
        "xxxxxxxx",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Drop any finding whose `match_text` (case-insensitive) contains any
/// stopword. Mirrors upstream `detect.Detector.applyStopwords` behaviour.
pub fn filter_with_stopwords(findings: Vec<Finding>, stopwords: &[String]) -> Vec<Finding> {
    if stopwords.is_empty() {
        return findings;
    }
    let lowered: Vec<String> = stopwords.iter().map(|s| s.to_ascii_lowercase()).collect();
    findings
        .into_iter()
        .filter(|f| {
            let m_lower = f.match_text.to_ascii_lowercase();
            let s_lower = f.secret.to_ascii_lowercase();
            !lowered
                .iter()
                .any(|stop| m_lower.contains(stop) || s_lower.contains(stop))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_finding(match_text: &str, secret: &str) -> Finding {
        Finding {
            description: String::new(),
            start_line: 1,
            end_line: 1,
            start_column: 1,
            end_column: 1,
            match_text: match_text.into(),
            secret: secret.into(),
            file: String::new(),
            symlink_file: String::new(),
            commit: String::new(),
            entropy: 0.0,
            author: String::new(),
            email: String::new(),
            date: String::new(),
            message: String::new(),
            tags: vec![],
            rule_id: String::new(),
            fingerprint: String::new(),
        }
    }

    #[test]
    fn default_pack_includes_high_signal_terms() {
        let pack = default_stopwords();
        assert!(pack.iter().any(|s| s == "example"));
        assert!(pack.iter().any(|s| s == "changeme"));
    }

    #[test]
    fn empty_stopwords_passes_everything_through() {
        let f = mk_finding("api_key=abc", "abc");
        let kept = filter_with_stopwords(vec![f], &[]);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn stopword_drops_matching_finding() {
        let f = mk_finding("api_key=DUMMY_TOKEN_123", "DUMMY_TOKEN_123");
        let kept = filter_with_stopwords(vec![f], &["dummy".to_string()]);
        assert!(kept.is_empty());
    }
}
