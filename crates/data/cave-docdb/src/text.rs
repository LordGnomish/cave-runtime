// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Clean-room MongoDB-style text index and `$text` search.
//!
//! A text index covers one or more string fields of a collection. The `$text`
//! query operator runs a search string against the concatenated text of those
//! fields with MongoDB semantics:
//!   * the search string is split into terms (whitespace-separated);
//!   * bare terms are OR-combined — a document matches if it contains *any*;
//!   * a term prefixed with `-` is a negation — documents containing it are
//!     excluded even if they match a positive term;
//!   * a double-quoted run is a phrase — it must appear as a contiguous,
//!     case-insensitive substring;
//!   * matching is case-insensitive and diacritic-naive (ASCII fold only).
//!
//! This is a from-scratch implementation; no MongoDB/FerretDB code is copied.

use crate::bson::Document;
use std::collections::HashSet;

/// A parsed `$text` search string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextQuery {
    /// Positive single-word terms (OR-combined).
    pub terms: Vec<String>,
    /// Negated terms — their presence excludes a document.
    pub negations: Vec<String>,
    /// Quoted phrases that must appear contiguously.
    pub phrases: Vec<String>,
}

/// Tokenize a string into lowercase alphanumeric word tokens.
pub fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// Parse a `$text` `$search` string into its term / negation / phrase parts.
///
/// Quoted runs (`"..."`) become phrases; outside quotes, whitespace-separated
/// words are terms, and a leading `-` marks a negation.
pub fn parse_search(search: &str) -> TextQuery {
    let mut q = TextQuery::default();
    let mut chars = search.chars().peekable();
    let mut buf = String::new();

    // Flush the accumulated non-quoted word, classifying it.
    let flush = |buf: &mut String, q: &mut TextQuery| {
        if buf.is_empty() {
            return;
        }
        let word = std::mem::take(buf);
        if let Some(neg) = word.strip_prefix('-') {
            for t in tokenize(neg) {
                q.negations.push(t);
            }
        } else {
            for t in tokenize(&word) {
                q.terms.push(t);
            }
        }
    };

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                flush(&mut buf, &mut q);
                // Consume until the closing quote.
                let mut phrase = String::new();
                for pc in chars.by_ref() {
                    if pc == '"' {
                        break;
                    }
                    phrase.push(pc);
                }
                let phrase = phrase.trim().to_lowercase();
                if !phrase.is_empty() {
                    q.phrases.push(phrase);
                }
            }
            c if c.is_whitespace() => flush(&mut buf, &mut q),
            c => buf.push(c),
        }
    }
    flush(&mut buf, &mut q);
    q
}

/// Evaluate a parsed text query against a document's indexed text.
///
/// `full_text_lower` is the lowercase concatenation of the document's
/// text-indexed field values; `tokens` is its tokenized word set.
pub fn matches(query: &TextQuery, tokens: &HashSet<String>, full_text_lower: &str) -> bool {
    // A negated term anywhere disqualifies the document.
    if query.negations.iter().any(|n| tokens.contains(n)) {
        return false;
    }
    // Every phrase must appear contiguously.
    if !query.phrases.iter().all(|p| full_text_lower.contains(p)) {
        return false;
    }
    // If there are positive terms or phrases, require at least one positive hit.
    let has_positive = !query.terms.is_empty() || !query.phrases.is_empty();
    if !has_positive {
        // Negation-only query: matches anything that survived the exclusion.
        return true;
    }
    let term_hit = query.terms.iter().any(|t| tokens.contains(t));
    term_hit || !query.phrases.is_empty()
}

/// Collect the indexed text of `doc` over the given fields: a `(token_set,
/// space-joined lowercase text)` pair ready for [`matches`].
pub fn doc_text(doc: &Document, fields: &[String]) -> (HashSet<String>, String) {
    let mut tokens = HashSet::new();
    let mut parts: Vec<String> = Vec::new();
    for f in fields {
        if let Some(s) = doc.get(f).and_then(|v| v.as_str()) {
            let lower = s.to_lowercase();
            for t in tokenize(&lower) {
                tokens.insert(t);
            }
            parts.push(lower);
        }
    }
    (tokens, parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(words: &[&str]) -> HashSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn tokenize_lowercases_and_splits_punctuation() {
        assert_eq!(
            tokenize("Hello, WORLD! foo-bar"),
            vec!["hello", "world", "foo", "bar"]
        );
    }

    #[test]
    fn parse_separates_terms_negations_phrases() {
        let q = parse_search("coffee -decaf \"cold brew\"");
        assert_eq!(q.terms, vec!["coffee"]);
        assert_eq!(q.negations, vec!["decaf"]);
        assert_eq!(q.phrases, vec!["cold brew"]);
    }

    #[test]
    fn matches_any_positive_term() {
        let q = parse_search("coffee tea");
        assert!(matches(&q, &set(&["i", "love", "tea"]), "i love tea"));
        assert!(!matches(&q, &set(&["i", "love", "water"]), "i love water"));
    }

    #[test]
    fn negation_excludes_document() {
        let q = parse_search("coffee -decaf");
        assert!(matches(&q, &set(&["fresh", "coffee"]), "fresh coffee"));
        assert!(!matches(
            &q,
            &set(&["decaf", "coffee"]),
            "decaf coffee"
        ));
    }

    #[test]
    fn phrase_must_appear_contiguously() {
        let q = parse_search("\"cold brew\"");
        assert!(matches(&q, &set(&["cold", "brew"]), "iced cold brew please"));
        // tokens present but not contiguous -> no phrase match
        assert!(!matches(
            &q,
            &set(&["cold", "and", "brew"]),
            "cold and brew"
        ));
    }

    #[test]
    fn only_negations_matches_all_without_them() {
        let q = parse_search("-spam");
        assert!(matches(&q, &set(&["ham", "eggs"]), "ham eggs"));
        assert!(!matches(&q, &set(&["spam", "eggs"]), "spam eggs"));
    }

    #[test]
    fn doc_text_concatenates_indexed_fields() {
        let doc: Document = json!({"title": "Cold Brew", "body": "Best Coffee", "n": 3})
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let (tokens, full) = doc_text(&doc, &["title".to_string(), "body".to_string()]);
        assert!(tokens.contains("cold"));
        assert!(tokens.contains("coffee"));
        assert!(full.contains("cold brew"));
        // non-indexed / non-string field not included
        assert!(!full.contains('3'));
    }
}
