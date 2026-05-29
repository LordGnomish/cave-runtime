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
//! upstream: manticoresoftware/manticoresearch v25.8.2 — src/tokenizer/

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
        // Keep only tokens with at least one alphanumeric character.
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
pub fn stem(word: &str) -> String {
    let w = word.to_lowercase();
    // Rule order: longest suffix first to avoid partial matches.
    let rules: &[(&str, &str)] = &[
        ("ational", "ate"),
        ("tional", "tion"),
        ("enci", "ence"),
        ("anci", "ance"),
        ("izer", "ize"),
        ("ising", "ise"),
        ("izing", "ize"),
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
        ("ments", ""),
        ("ment", ""),
        ("ings", ""),
        ("ing", ""),
        ("tions", ""),
        ("tion", ""),
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
