// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for boolean query execution (AND/OR/NOT) and phrase query matching.

use std::str::FromStr;
use cave_search::query::{Query, BooleanQuery, BoolNode};
use cave_search::index::Index;
use cave_search::tenant::TenantId;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

fn build_index() -> Index {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    idx.add_document(1, "the quick brown fox jumps over the lazy dog");
    idx.add_document(2, "quick brown rabbit runs fast");
    idx.add_document(3, "lazy cat sleeps all day long");
    idx.add_document(4, "fox and rabbit are friends");
    idx
}

// ── Term query ─────────────────────────────────────────────────────────────────

#[test]
fn term_query_finds_matching_docs() {
    let idx = build_index();
    let q = Query::Term("fox".to_string());
    let results = q.execute(&idx);
    assert!(results.contains(&1), "doc 1 has 'fox'");
    assert!(results.contains(&4), "doc 4 has 'fox'");
    assert!(!results.contains(&2), "doc 2 has no 'fox'");
}

#[test]
fn term_query_missing_term_returns_empty() {
    let idx = build_index();
    let q = Query::Term("nonexistent".to_string());
    let results = q.execute(&idx);
    assert!(results.is_empty());
}

// ── Boolean AND (must) ─────────────────────────────────────────────────────────

#[test]
fn bool_and_query_intersects_results() {
    let idx = build_index();
    // "quick" AND "brown" — both in docs 1 and 2
    let q = BooleanQuery::and(vec![
        Query::Term("quick".to_string()),
        Query::Term("brown".to_string()),
    ]);
    let results = q.execute(&idx);
    assert!(results.contains(&1), "doc 1 has both quick and brown");
    assert!(results.contains(&2), "doc 2 has both quick and brown");
    assert!(!results.contains(&3), "doc 3 has neither");
    assert!(!results.contains(&4), "doc 4 has neither");
}

#[test]
fn bool_and_query_empty_intersection() {
    let idx = build_index();
    // "fox" AND "rabbit" — no doc has both
    let q = BooleanQuery::and(vec![
        Query::Term("fox".to_string()),
        Query::Term("rabbit".to_string()),
    ]);
    let results = q.execute(&idx);
    // Doc 4 has "fox" and "rabbit"
    assert!(results.contains(&4), "doc 4 has fox and rabbit");
}

// ── Boolean OR (should) ────────────────────────────────────────────────────────

#[test]
fn bool_or_query_unions_results() {
    let idx = build_index();
    // "fox" OR "rabbit" — docs 1, 2, 4
    let q = BooleanQuery::or(vec![
        Query::Term("fox".to_string()),
        Query::Term("rabbit".to_string()),
    ]);
    let results = q.execute(&idx);
    assert!(results.contains(&1), "doc 1 has fox");
    assert!(results.contains(&2), "doc 2 has rabbit");
    assert!(results.contains(&4), "doc 4 has both");
    assert!(!results.contains(&3), "doc 3 has neither");
}

// ── Boolean NOT (must_not) ─────────────────────────────────────────────────────

#[test]
fn bool_not_query_excludes_results() {
    let idx = build_index();
    // NOT "lazy" — should exclude docs 1 and 3
    let q = BooleanQuery::not(Query::Term("lazy".to_string()));
    let results = q.execute(&idx);
    assert!(!results.contains(&1), "doc 1 has lazy");
    assert!(!results.contains(&3), "doc 3 has lazy");
    assert!(results.contains(&2), "doc 2 has no lazy");
    assert!(results.contains(&4), "doc 4 has no lazy");
}

// ── Compound bool (must + must_not) ───────────────────────────────────────────

#[test]
fn compound_bool_must_and_must_not() {
    let idx = build_index();
    // "quick" AND NOT "brown"
    let q = Query::Bool(BoolNode {
        must: vec![Query::Term("quick".to_string())],
        should: vec![],
        must_not: vec![Query::Term("brown".to_string())],
    });
    let results = q.execute(&idx);
    // Docs with "quick": 1, 2. Both have "brown". So intersection should be empty.
    assert!(!results.contains(&1));
    assert!(!results.contains(&2));
}

// ── Phrase query ───────────────────────────────────────────────────────────────

#[test]
fn phrase_query_matches_exact_sequence() {
    let idx = build_index();
    // "quick brown" appears in docs 1 and 2
    let q = Query::Phrase(vec!["quick".to_string(), "brown".to_string()]);
    let results = q.execute(&idx);
    assert!(results.contains(&1), "doc 1 has 'quick brown'");
    assert!(results.contains(&2), "doc 2 has 'quick brown'");
    // doc 3: no quick or brown; doc 4: has neither quick nor brown in sequence
    assert!(!results.contains(&3));
}

#[test]
fn phrase_query_no_match_wrong_order() {
    let idx = build_index();
    // "brown quick" — inverted, not present in any doc
    let q = Query::Phrase(vec!["brown".to_string(), "quick".to_string()]);
    let results = q.execute(&idx);
    // Neither doc 1 nor 2 has "brown quick" in that order
    assert!(!results.contains(&1));
    assert!(!results.contains(&2));
}
