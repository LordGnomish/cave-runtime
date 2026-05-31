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
    let _ = s;
    Vec::new() // stub
}

/// Parse a `$text` `$search` string into its term / negation / phrase parts.
pub fn parse_search(search: &str) -> TextQuery {
    let _ = search;
    TextQuery::default() // stub
}

/// Evaluate a parsed text query against a document's indexed text.
///
/// `full_text` is the concatenation (space-joined) of the document's
/// text-indexed field values; `tokens` is its tokenized word set.
pub fn matches(query: &TextQuery, tokens: &HashSet<String>, full_text_lower: &str) -> bool {
    let _ = (query, tokens, full_text_lower);
    false // stub
}

/// Collect the indexed text of `doc` over the given fields: a `(token_set,
/// space-joined lowercase text)` pair ready for [`matches`].
pub fn doc_text(doc: &Document, fields: &[String]) -> (HashSet<String>, String) {
    let _ = (doc, fields);
    (HashSet::new(), String::new()) // stub
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
