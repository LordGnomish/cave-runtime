// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's GIN access method
// (src/backend/access/gin/{gininsert.c,ginget.c,ginlogic.c}).
//
// GIN is an inverted index: each indexed datum decomposes into several keys
// (array elements, tsvector lexemes, jsonb paths), and the entry tree maps
// each key to a sorted, de-duplicated posting list of heap TIDs. The
// consistent function combines per-key matches — overlap (`&&`, ANY) is the
// union of posting lists, containment (`@>`, ALL) is their intersection.
//
// Faithful behaviours asserted:
//   * a row contributes one posting-list membership per decomposed key
//   * posting lists are sorted ascending and de-duplicated
//   * ALL-query (@>) returns the intersection across query keys
//   * ANY-query (overlap) returns the union across query keys

use cave_rdbms::storage::index::GinIndex;
use cave_rdbms::types::SqlValue;

fn t(s: &str) -> SqlValue {
    SqlValue::Text(s.into())
}

#[test]
fn gin_posting_lists_are_sorted_and_deduped() {
    let mut gin = GinIndex::new();
    gin.insert(5, vec![t("a")]);
    gin.insert(1, vec![t("a")]);
    gin.insert(5, vec![t("a")]); // duplicate (tid,key) collapses
    assert_eq!(gin.posting_list(&t("a")), vec![1, 5]);
    assert_eq!(gin.key_count(), 1);
}

#[test]
fn gin_containment_all_is_intersection() {
    let mut gin = GinIndex::new();
    gin.insert(0, vec![t("a"), t("b"), t("c")]);
    gin.insert(1, vec![t("b"), t("c")]);
    gin.insert(2, vec![t("c"), t("d")]);
    // rows whose array @> {b,c}
    assert_eq!(gin.search_all(&[t("b"), t("c")]), vec![0, 1]);
    // @> {c} matches everyone
    assert_eq!(gin.search_all(&[t("c")]), vec![0, 1, 2]);
    // @> {a,d} matches no single row
    assert!(gin.search_all(&[t("a"), t("d")]).is_empty());
}

#[test]
fn gin_overlap_any_is_union() {
    let mut gin = GinIndex::new();
    gin.insert(0, vec![t("a"), t("b")]);
    gin.insert(1, vec![t("b"), t("c")]);
    gin.insert(2, vec![t("d")]);
    // rows whose array && {a,d}
    assert_eq!(gin.search_any(&[t("a"), t("d")]), vec![0, 2]);
    // && {b} → rows 0,1
    assert_eq!(gin.search_any(&[t("b")]), vec![0, 1]);
}

#[test]
fn gin_empty_query_and_missing_keys() {
    let mut gin = GinIndex::new();
    gin.insert(0, vec![t("x")]);
    assert!(gin.search_all(&[]).is_empty());
    assert!(gin.search_any(&[]).is_empty());
    assert!(gin.search_all(&[t("zzz")]).is_empty());
    assert!(gin.posting_list(&t("zzz")).is_empty());
}
