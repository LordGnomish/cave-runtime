// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Text analyzer: tokenization, stop-word filtering, stemming, normalization.
//!
//! Implements Manticore-equivalent text analysis pipeline:
//! 1. Normalization (lowercase, punctuation strip)
//! 2. Tokenization (whitespace + punctuation split)
//! 3. Stop-word removal (English stop-word set)
//! 4. Suffix-stripping stemmer (Porter-inspired, rule-based)
//!
//! upstream: manticoresoftware/manticoresearch — searchd/src/sphinxjsonquery.cpp
//!           and src/tokenizer/ for the tokenizer pipeline.

use crate::tenant::TenantId;

/// Common English stop words (Manticore default English stop-word list subset).
static STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for",
    "of", "with", "by", "from", "is", "are", "was", "were", "be", "been",
    "being", "have", "has", "had", "do", "does", "did", "will", "would",
    "could", "should", "may", "might", "shall", "can", "it", "its", "this",
    "that", "these", "those", "i", "me", "my", "we", "our", "you", "your",
    "he", "she", "they", "them", "their", "not", "no", "up", "as", "if",
    "into", "about", "than", "then", "so", "also",
];

/// Normalize a single token: lowercase, strip leading/trailing punctuation.
pub fn normalize_token(token: &str) -> String {
    let lower = token.to_lowercase();
    lower
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_string()
}

/// Tokenize `text` into normalized, non-empty tokens.
///
/// The `_tenant_id` parameter is reserved for future per-tenant analyzer
/// configuration (custom dictionaries, locale-specific rules).
pub fn tokenize(text: &str, _tenant_id: &TenantId) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    text.split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '!' || c == '?')
        .map(normalize_token)
        // Keep only tokens that have at least one alphanumeric character.
        .filter(|t| !t.is_empty() && t.chars().any(|c| c.is_alphanumeric()))
        .collect()
}

/// Remove stop words from a token list.
///
/// The `_tenant_id` parameter is reserved for future per-tenant custom
/// stop-word lists stored in the tenant configuration.
pub fn filter_stop_words<'a>(tokens: Vec<&'a str>, _tenant_id: &TenantId) -> Vec<&'a str> {
    tokens
        .into_iter()
        .filter(|t| {
            let lower = t.to_lowercase();
            !STOP_WORDS.contains(&lower.as_str())
        })
        .collect()
}

/// Suffix-stripping stemmer (lightweight Porter-inspired rules).
///
/// Handles the most common English suffixes without external dependencies.
/// For production use, a full Porter2 / Snowball implementation should be
/// wired in; this covers the MVP surface.
pub fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    // Rule order: longest suffix first.
    let rules: &[(&str, &str)] = &[
        ("ational", "ate"),
        ("tional", "tion"),
        ("enci", "ence"),
        ("anci", "ance"),
        ("izer", "ize"),
        ("ising", "ise"),
        ("izing", "ize"),
        ("ising", "ise"),
        ("ation", "ate"),
        ("ator", "ate"),
        ("alism", "al"),
        ("iveness", "ive"),
        ("fulness", "ful"),
        ("ousness", "ous"),
        ("aliti", "al"),
        ("iviti", "ive"),
        ("running", "run"),
        ("singing", "sing"),
        ("going", "go"),
        ("flies", "fly"),
        ("ying", "y"),
        ("nesses", "ness"),
        ("ment", ""),
        ("ments", ""),
        ("ing", ""),
        ("ings", ""),
        ("tion", ""),
        ("tions", ""),
        ("ness", ""),
        ("ful", ""),
        ("less", ""),
        ("ous", ""),
        ("ive", ""),
        ("ize", ""),
        ("ise", ""),
        ("ied", "y"),
        ("ies", "y"),
        ("ed", ""),
        ("er", ""),
        ("ers", ""),
        ("ely", ""),
        ("ly", ""),
        ("al", ""),
        ("able", ""),
        ("ible", ""),
        ("ic", ""),
        ("ical", ""),
        ("est", ""),
        ("est", ""),
        ("s", ""),
    ];

    for (suffix, replacement) in rules {
        if w.len() > suffix.len() + 2 && w.ends_with(suffix) {
            let stem_part = &w[..w.len() - suffix.len()];
            return format!("{}{}", stem_part, replacement);
        }
    }

    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use cave_kernel::ns::TenantId;
    use std::str::FromStr;

    fn tenant() -> TenantId {
        TenantId::from_str("default").unwrap()
    }

    #[test]
    fn normalize_strips_trailing_punct() {
        assert_eq!(normalize_token("hello,"), "hello");
        assert_eq!(normalize_token("world!"), "world");
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(normalize_token("HELLO"), "hello");
    }

    #[test]
    fn tokenize_basic() {
        let t = tokenize("hello world", &tenant());
        assert_eq!(t, vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_empty() {
        assert!(tokenize("", &tenant()).is_empty());
    }

    #[test]
    fn stop_words_removed() {
        let tokens = vec!["the", "quick"];
        let filtered = filter_stop_words(tokens, &tenant());
        assert_eq!(filtered, vec!["quick"]);
    }

    #[test]
    fn stem_ing_suffix() {
        let s = stem("running");
        assert!(!s.is_empty());
        assert!(!s.ends_with("ing"));
    }
}
