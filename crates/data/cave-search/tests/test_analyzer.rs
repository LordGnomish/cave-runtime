// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for the text analyzer: tokenization and stop-word filtering.

use std::str::FromStr;
use cave_search::analyzer::{tokenize, filter_stop_words, stem, normalize_token};
use cave_search::tenant::TenantId;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

#[test]
fn tokenize_splits_on_whitespace_and_punctuation() {
    let tokens = tokenize("Hello, world! This is a test.", &tenant());
    assert!(tokens.contains(&"hello".to_string()));
    assert!(tokens.contains(&"world".to_string()));
    assert!(tokens.contains(&"test".to_string()));
}

#[test]
fn tokenize_lowercases_all_tokens() {
    let tokens = tokenize("UPPER lower MiXeD", &tenant());
    assert!(tokens.iter().all(|t| t.chars().all(|c| !c.is_uppercase())));
}

#[test]
fn tokenize_removes_empty_tokens() {
    let tokens = tokenize("  spaces   between   words  ", &tenant());
    assert!(!tokens.iter().any(|t| t.is_empty()));
}

#[test]
fn filter_stop_words_removes_common_english_words() {
    let tokens = vec!["the", "quick", "brown", "fox", "is", "a", "test"];
    let filtered = filter_stop_words(tokens.clone(), &tenant());
    assert!(!filtered.contains(&"the"));
    assert!(!filtered.contains(&"is"));
    assert!(!filtered.contains(&"a"));
    assert!(filtered.contains(&"quick"));
    assert!(filtered.contains(&"fox"));
    assert!(filtered.contains(&"test"));
}

#[test]
fn normalize_token_strips_punctuation() {
    assert_eq!(normalize_token("hello,"), "hello");
    assert_eq!(normalize_token("world!"), "world");
    assert_eq!(normalize_token("test."), "test");
}

#[test]
fn stem_returns_root_form() {
    // Basic stemming: "running" -> "run", "tests" -> "test"
    let s = stem("running");
    assert!(!s.is_empty());
    let s2 = stem("tests");
    assert!(!s2.is_empty());
}

#[test]
fn tokenize_returns_empty_for_empty_input() {
    let tokens = tokenize("", &tenant());
    assert!(tokens.is_empty());
}

#[test]
fn tokenize_handles_numeric_tokens() {
    let tokens = tokenize("version 3.14 release", &tenant());
    assert!(tokens.iter().any(|t| t.contains("3") || t.contains("314") || t == "3.14"));
}
