// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for the inverted index and posting list data structures.

use std::str::FromStr;
use cave_search::index::{Index, PostingList};
use cave_search::tenant::TenantId;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

// --- PostingList tests ---

#[test]
fn posting_list_starts_empty() {
    let pl = PostingList::new();
    assert_eq!(pl.doc_freq(), 0);
    assert_eq!(pl.total_term_freq(), 0);
}

#[test]
fn posting_list_add_doc_increments_freq() {
    let mut pl = PostingList::new();
    pl.add_doc(1, 3);
    assert_eq!(pl.doc_freq(), 1);
    assert_eq!(pl.total_term_freq(), 3);
    assert_eq!(pl.get_doc_freq(1), 3);
}

#[test]
fn posting_list_add_multiple_docs() {
    let mut pl = PostingList::new();
    pl.add_doc(1, 2);
    pl.add_doc(5, 4);
    pl.add_doc(10, 1);
    assert_eq!(pl.doc_freq(), 3);
    assert_eq!(pl.total_term_freq(), 7);
}

#[test]
fn posting_list_remove_doc() {
    let mut pl = PostingList::new();
    pl.add_doc(1, 3);
    pl.add_doc(2, 5);
    pl.remove_doc(1);
    assert_eq!(pl.doc_freq(), 1);
    assert_eq!(pl.get_doc_freq(1), 0);
    assert_eq!(pl.get_doc_freq(2), 5);
}

#[test]
fn posting_list_iter_returns_all_entries() {
    let mut pl = PostingList::new();
    pl.add_doc(1, 2);
    pl.add_doc(3, 4);
    let entries: Vec<(u32, u32)> = pl.iter().collect();
    assert_eq!(entries.len(), 2);
    assert!(entries.contains(&(1, 2)));
    assert!(entries.contains(&(3, 4)));
}

#[test]
fn posting_list_merge_combines_entries() {
    let mut pl1 = PostingList::new();
    pl1.add_doc(1, 2);
    let mut pl2 = PostingList::new();
    pl2.add_doc(3, 5);
    let merged = PostingList::merge(vec![pl1, pl2]);
    assert_eq!(merged.doc_freq(), 2);
}

// --- Index tests ---

#[test]
fn index_new_has_correct_id() {
    let t = tenant();
    let idx = Index::new(&t, "my-index");
    assert_eq!(idx.id(), "my-index");
}

#[test]
fn index_add_and_retrieve_document() {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    idx.add_document(1, "the quick brown fox");
    // "quick" should appear in results for term "quick"
    let docs = idx.get_doc_ids_for_term("quick");
    assert!(docs.contains(&1));
}

#[test]
fn index_add_multiple_documents() {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    idx.add_document(1, "rust programming language");
    idx.add_document(2, "rust is great");
    idx.add_document(3, "python programming");
    let docs = idx.get_doc_ids_for_term("rust");
    assert!(docs.contains(&1));
    assert!(docs.contains(&2));
    assert!(!docs.contains(&3));
}

#[test]
fn index_delete_document_removes_from_postings() {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    idx.add_document(1, "hello world");
    idx.add_document(2, "hello cave");
    idx.delete_document(1);
    let docs = idx.get_doc_ids_for_term("hello");
    assert!(!docs.contains(&1));
    assert!(docs.contains(&2));
}

#[test]
fn index_doc_count_tracks_live_docs() {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    assert_eq!(idx.doc_count(), 0);
    idx.add_document(1, "foo bar");
    assert_eq!(idx.doc_count(), 1);
    idx.add_document(2, "baz qux");
    assert_eq!(idx.doc_count(), 2);
    idx.delete_document(1);
    assert_eq!(idx.doc_count(), 1);
}

#[test]
fn index_avg_doc_len_computes_correctly() {
    let t = tenant();
    let mut idx = Index::new(&t, "test");
    idx.add_document(1, "one two three"); // 3 tokens
    idx.add_document(2, "one");           // 1 token
    // avg = (3+1)/2 = 2.0
    let avg = idx.avg_doc_len();
    assert!((avg - 2.0).abs() < 1e-9, "expected avg 2.0, got {}", avg);
}

#[test]
fn index_term_not_present_returns_empty() {
    let t = tenant();
    let idx = Index::new(&t, "test");
    assert!(idx.get_doc_ids_for_term("nonexistent").is_empty());
}
