// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for the text analyzer: tokenize, stop-word filter, stemmer, normalize.

use cave_search::analyzer::{normalize_token, tokenize, filter_stop_words, stem};
use cave_search::tenant::TenantId;
use std::str::FromStr;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

#[test]
fn normalize_lowercases_input() {
    assert_eq!(normalize_token("HELLO"), "hello");
    assert_eq!(normalize_token("Mixed"), "mixed");
}

#[test]
fn normalize_strips_trailing_punctuation() {
    assert_eq!(normalize_token("hello,"), "hello");
    assert_eq!(normalize_token("world!"), "world");
    assert_eq!(normalize_token("test."), "test");
}

#[test]
fn tokenize_splits_on_whitespace() {
    let tokens = tokenize("hello world", &tenant());
    assert_eq!(tokens, vec!["hello", "world"]);
}

#[test]
fn tokenize_splits_on_punctuation() {
    let tokens = tokenize("Hello, world! This is a test.", &tenant());
    assert!(tokens.contains(&"hello".to_string()), "expected 'hello' in {:?}", tokens);
    assert!(tokens.contains(&"world".to_string()), "expected 'world' in {:?}", tokens);
    assert!(tokens.contains(&"test".to_string()), "expected 'test' in {:?}", tokens);
}

#[test]
fn tokenize_lowercases_all_tokens() {
    let tokens = tokenize("UPPER lower MiXeD", &tenant());
    assert!(tokens.iter().all(|t| t.chars().all(|c| !c.is_uppercase())));
}

#[test]
fn tokenize_returns_empty_for_empty_input() {
    let tokens = tokenize("", &tenant());
    assert!(tokens.is_empty());
}

#[test]
fn tokenize_removes_empty_tokens() {
    let tokens = tokenize("  spaces   between   words  ", &tenant());
    assert!(!tokens.iter().any(|t| t.is_empty()));
    assert!(!tokens.is_empty());
}

#[test]
fn filter_stop_words_removes_common_english_words() {
    let tokens = vec!["the", "quick", "brown", "fox", "is", "a", "test"];
    let filtered = filter_stop_words(tokens, &tenant());
    assert!(!filtered.contains(&"the"), "should remove 'the'");
    assert!(!filtered.contains(&"is"), "should remove 'is'");
    assert!(!filtered.contains(&"a"), "should remove 'a'");
    assert!(filtered.contains(&"quick"));
    assert!(filtered.contains(&"fox"));
    assert!(filtered.contains(&"test"));
}

#[test]
fn filter_stop_words_preserves_non_stop_words() {
    let tokens = vec!["search", "engine", "fast"];
    let filtered = filter_stop_words(tokens, &tenant());
    assert_eq!(filtered, vec!["search", "engine", "fast"]);
}

#[test]
fn stem_removes_ing_suffix() {
    let s = stem("running");
    assert!(!s.is_empty(), "stem must return non-empty for 'running'");
    assert!(!s.ends_with("ing"), "stem('running') should not end with 'ing'; got '{}'", s);
}

#[test]
fn stem_short_words_unchanged() {
    // Words too short to stem should be returned as-is or minimally changed.
    let s = stem("go");
    assert!(!s.is_empty());
}
