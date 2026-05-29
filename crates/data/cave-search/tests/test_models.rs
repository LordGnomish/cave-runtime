// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for IndexSchema, Document, SearchRequest/Response, highlight_field, paginate.

use std::str::FromStr;
use cave_search::models::{
    Document, FieldType, FieldMapping, IndexSchema,
    SearchRequest, SearchResponse, HitDocument, HighlightSnippet,
    highlight_field, paginate,
};
use cave_search::tenant::TenantId;

fn tenant() -> TenantId {
    TenantId::from_str("test-tenant").unwrap()
}

// ── FieldMapping / IndexSchema ────────────────────────────────────────────────

#[test]
fn field_mapping_stores_field_types() {
    let mut schema = IndexSchema::new("products");
    schema.add_field("title", FieldType::Text);
    schema.add_field("price", FieldType::Float);
    schema.add_field("in_stock", FieldType::Bool);

    assert_eq!(schema.field_type("title"), Some(FieldType::Text));
    assert_eq!(schema.field_type("price"), Some(FieldType::Float));
    assert_eq!(schema.field_type("in_stock"), Some(FieldType::Bool));
    assert_eq!(schema.field_type("missing"), None);
}

#[test]
fn index_schema_name_roundtrip() {
    let schema = IndexSchema::new("my-index");
    assert_eq!(schema.name(), "my-index");
}

#[test]
fn field_mapping_list_fields() {
    let mut schema = IndexSchema::new("test");
    schema.add_field("a", FieldType::Text);
    schema.add_field("b", FieldType::Integer);
    let fields = schema.field_names();
    assert!(fields.contains(&"a"));
    assert!(fields.contains(&"b"));
}

// ── Document ──────────────────────────────────────────────────────────────────

#[test]
fn document_stores_fields() {
    let mut doc = Document::new(42);
    doc.set_field("title", "Rust Programming");
    doc.set_field("author", "Nobody");

    assert_eq!(doc.id(), 42);
    assert_eq!(doc.get_field("title"), Some("Rust Programming"));
    assert_eq!(doc.get_field("missing"), None);
}

#[test]
fn document_id_is_immutable() {
    let doc = Document::new(99);
    assert_eq!(doc.id(), 99);
}

// ── SearchRequest ─────────────────────────────────────────────────────────────

#[test]
fn search_request_default_pagination() {
    let req = SearchRequest::new("quick fox");
    assert_eq!(req.query_text(), "quick fox");
    assert_eq!(req.from(), 0);
    assert_eq!(req.size(), 10);  // default page size
}

#[test]
fn search_request_custom_pagination() {
    let req = SearchRequest::new("test")
        .with_from(20)
        .with_size(5);
    assert_eq!(req.from(), 20);
    assert_eq!(req.size(), 5);
}

#[test]
fn search_request_with_index() {
    let req = SearchRequest::new("test").with_index("products");
    assert_eq!(req.index(), Some("products"));
}

// ── SearchResponse / HitDocument ─────────────────────────────────────────────

#[test]
fn search_response_total_hits() {
    let hits = vec![
        HitDocument { doc_id: 1, score: 2.5, fields: std::collections::HashMap::new() },
        HitDocument { doc_id: 2, score: 1.8, fields: std::collections::HashMap::new() },
    ];
    let resp = SearchResponse::new(hits, 42);
    assert_eq!(resp.total(), 42);
    assert_eq!(resp.hits().len(), 2);
}

#[test]
fn search_response_empty() {
    let resp = SearchResponse::new(vec![], 0);
    assert_eq!(resp.total(), 0);
    assert!(resp.hits().is_empty());
}

// ── Highlight ─────────────────────────────────────────────────────────────────

#[test]
fn highlight_wraps_matching_term() {
    let snippets = highlight_field("the quick brown fox", "quick", "<em>", "</em>");
    assert!(
        snippets.iter().any(|s| s.fragment.contains("<em>quick</em>")),
        "highlight should wrap 'quick' with em tags; got: {:?}", snippets
    );
}

#[test]
fn highlight_no_match_returns_original() {
    let snippets = highlight_field("the quick brown fox", "nonexistent", "<b>", "</b>");
    // Should return the original text as a snippet with no highlighting
    assert!(!snippets.is_empty());
    // No highlight tags in result
    assert!(!snippets.iter().any(|s| s.fragment.contains("<b>")));
}

// ── Pagination ────────────────────────────────────────────────────────────────

#[test]
fn paginate_returns_correct_window() {
    let items: Vec<u32> = (0..100).collect();
    let page = paginate(&items, 10, 5);
    assert_eq!(page.len(), 5);
    assert_eq!(page[0], 10);
    assert_eq!(page[4], 14);
}

#[test]
fn paginate_beyond_end_returns_remaining() {
    let items: Vec<u32> = (0..7).collect();
    let page = paginate(&items, 5, 10);
    assert_eq!(page.len(), 2);
    assert_eq!(page[0], 5);
    assert_eq!(page[1], 6);
}

#[test]
fn paginate_empty_returns_empty() {
    let empty: Vec<u32> = vec![];
    assert!(paginate(&empty, 0, 10).is_empty());
}
