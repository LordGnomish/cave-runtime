// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD port of PostgreSQL's hash access method
// (src/backend/access/hash/{hashfunc.c,hashinsert.c,hashsearch.c,hashpage.c}).
//
// Faithful behaviours asserted:
//   * equality-only probe returns the heap TIDs for a key
//   * duplicate keys accumulate a posting list
//   * the table grows its bucket count by incremental (linear) splits as the
//     load factor is exceeded, and all entries remain findable across splits
//   * distinct keys that collide into one bucket are disambiguated by key

use cave_rdbms::storage::index::HashIndex;
use cave_rdbms::types::SqlValue;

fn i4(n: i32) -> SqlValue {
    SqlValue::Int4(n)
}

#[test]
fn hash_equality_probe_returns_tids() {
    let mut idx = HashIndex::new();
    idx.insert(i4(10), 0);
    idx.insert(i4(20), 1);
    idx.insert(i4(30), 2);
    assert_eq!(idx.search(&i4(20)), vec![1]);
    assert!(idx.search(&i4(99)).is_empty());
    assert_eq!(idx.len(), 3);
}

#[test]
fn hash_duplicate_keys_accumulate_posting_list() {
    let mut idx = HashIndex::new();
    idx.insert(i4(7), 0);
    idx.insert(i4(7), 5);
    idx.insert(i4(7), 2);
    assert_eq!(idx.search(&i4(7)), vec![0, 5, 2]);
    assert_eq!(idx.len(), 3);
}

#[test]
fn hash_text_keys_are_supported() {
    let mut idx = HashIndex::new();
    idx.insert(SqlValue::Text("alice".into()), 0);
    idx.insert(SqlValue::Text("bob".into()), 1);
    assert_eq!(idx.search(&SqlValue::Text("bob".into())), vec![1]);
    assert!(idx.search(&SqlValue::Text("carol".into())).is_empty());
}

#[test]
fn hash_grows_buckets_by_incremental_split_and_keeps_everything() {
    let mut idx = HashIndex::new();
    let start_buckets = idx.bucket_count();
    for k in 0..400 {
        idx.insert(i4(k), k as usize);
    }
    // load factor must have driven new bucket splits
    assert!(
        idx.bucket_count() > start_buckets,
        "expected bucket growth, still {}",
        idx.bucket_count()
    );
    // everything still findable after rehoming
    for k in 0..400 {
        assert_eq!(idx.search(&i4(k)), vec![k as usize], "lost key {}", k);
    }
    assert_eq!(idx.len(), 400);
}
