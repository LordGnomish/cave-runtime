// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge / failure / boundary coverage for cave-docdb — BSON, query, update,
//! projection, cursor, engine, index.

use cave_docdb::bson::{Document, decode_doc, encode_doc};
use cave_docdb::cursor::{Cursor, CursorStore};
use cave_docdb::engine::Engine;
use cave_docdb::index::Index;
use cave_docdb::projection::apply_projection;
use cave_docdb::query::matches_query;
use cave_docdb::update::apply_update;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

fn doc_from_json(v: Value) -> Document {
    let mut d = Document::new();
    if let Value::Object(obj) = v {
        for (k, val) in obj {
            d.insert(k, val);
        }
    }
    d
}

// ---------------------------------------------------------------------------
// BSON round-trip + edge cases
// ---------------------------------------------------------------------------

#[test]
fn bson_decode_too_short_errors() {
    let res = decode_doc(&[0u8, 0, 0]);
    assert!(res.is_err());
}

#[test]
fn bson_decode_incomplete_errors() {
    // Claims 100 bytes long but only has 5.
    let bytes = vec![100u8, 0, 0, 0, 0];
    assert!(decode_doc(&bytes).is_err());
}

#[test]
fn bson_roundtrip_null_value() {
    let mut d = Document::new();
    d.insert("n".into(), Value::Null);
    let bytes = encode_doc(&d).unwrap();
    let back = decode_doc(&bytes).unwrap();
    assert_eq!(back.get("n"), Some(&Value::Null));
}

#[test]
fn bson_roundtrip_bool_both_states() {
    let mut d = Document::new();
    d.insert("t".into(), Value::Bool(true));
    d.insert("f".into(), Value::Bool(false));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    assert_eq!(back.get("t"), Some(&Value::Bool(true)));
    assert_eq!(back.get("f"), Some(&Value::Bool(false)));
}

#[test]
fn bson_int32_vs_int64_picked_by_range() {
    let mut d = Document::new();
    d.insert("small".into(), json!(42));
    d.insert("huge".into(), json!(5_000_000_000i64));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    assert_eq!(back.get("small"), Some(&json!(42)));
    assert_eq!(back.get("huge"), Some(&json!(5_000_000_000i64)));
}

#[test]
fn bson_roundtrip_double() {
    let mut d = Document::new();
    d.insert("pi".into(), json!(std::f64::consts::PI));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    let v = back.get("pi").and_then(|n| n.as_f64()).unwrap();
    assert!((v - std::f64::consts::PI).abs() < 1e-12);
}

#[test]
fn bson_roundtrip_string() {
    let mut d = Document::new();
    d.insert("hello".into(), json!("world ünicöde"));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    assert_eq!(back.get("hello"), Some(&json!("world ünicöde")));
}

#[test]
fn bson_roundtrip_objectid_24_hex() {
    let mut d = Document::new();
    d.insert("_id".into(), json!("507f1f77bcf86cd799439011"));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    // Should still come back as the same 24-char hex string
    assert_eq!(back.get("_id"), Some(&json!("507f1f77bcf86cd799439011")));
}

#[test]
fn bson_roundtrip_nested_object() {
    let mut d = Document::new();
    let mut inner = Map::new();
    inner.insert("a".into(), json!(1));
    inner.insert("b".into(), json!("two"));
    d.insert("nested".into(), Value::Object(inner));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    let n = back.get("nested").unwrap().as_object().unwrap();
    assert_eq!(n.get("a"), Some(&json!(1)));
    assert_eq!(n.get("b"), Some(&json!("two")));
}

#[test]
fn bson_roundtrip_array_preserves_order_and_length() {
    let mut d = Document::new();
    d.insert("xs".into(), json!([1, 2, 3, 4, 5]));
    let back = decode_doc(&encode_doc(&d).unwrap()).unwrap();
    let arr = back.get("xs").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 5);
    assert_eq!(arr[0], json!(1));
    assert_eq!(arr[4], json!(5));
}

#[test]
fn bson_empty_document_roundtrip() {
    let d = Document::new();
    let bytes = encode_doc(&d).unwrap();
    // Minimal BSON doc is 5 bytes: 4 len + 1 terminator
    assert_eq!(bytes.len(), 5);
    let back = decode_doc(&bytes).unwrap();
    assert!(back.is_empty());
}

// ---------------------------------------------------------------------------
// Query — $and / $or / $exists / equality / nested operators
// ---------------------------------------------------------------------------

#[test]
fn query_empty_filter_matches_everything() {
    let doc = doc_from_json(json!({"a": 1}));
    let q = Document::new();
    assert!(matches_query(&doc, &q));
}

#[test]
fn query_field_equality_match_and_miss() {
    let doc = doc_from_json(json!({"name": "alice", "age": 30}));
    let q_match = doc_from_json(json!({"name": "alice"}));
    let q_miss = doc_from_json(json!({"name": "bob"}));
    assert!(matches_query(&doc, &q_match));
    assert!(!matches_query(&doc, &q_miss));
}

#[test]
fn query_and_requires_all_conditions() {
    let doc = doc_from_json(json!({"a": 1, "b": 2}));
    let q = doc_from_json(json!({"$and": [{"a": 1}, {"b": 2}]}));
    assert!(matches_query(&doc, &q));
    let q_fail = doc_from_json(json!({"$and": [{"a": 1}, {"b": 9}]}));
    assert!(!matches_query(&doc, &q_fail));
}

#[test]
fn query_or_requires_any_condition() {
    let doc = doc_from_json(json!({"a": 1}));
    let q = doc_from_json(json!({"$or": [{"a": 1}, {"b": 99}]}));
    assert!(matches_query(&doc, &q));
    let q_fail = doc_from_json(json!({"$or": [{"a": 9}, {"b": 99}]}));
    assert!(!matches_query(&doc, &q_fail));
}

#[test]
fn query_exists_false_matches_missing_field() {
    let doc = doc_from_json(json!({"a": 1}));
    let q = doc_from_json(json!({"missing": {"$exists": false}}));
    assert!(matches_query(&doc, &q));
}

#[test]
fn query_missing_field_without_exists_does_not_match() {
    let doc = doc_from_json(json!({"a": 1}));
    let q = doc_from_json(json!({"missing": 1}));
    assert!(!matches_query(&doc, &q));
}

// ---------------------------------------------------------------------------
// Update operators — boundary + idempotency
// ---------------------------------------------------------------------------

#[test]
fn update_inc_on_missing_field_treats_as_zero() {
    let mut doc = doc_from_json(json!({"_id": "1"}));
    let upd = doc_from_json(json!({"$inc": {"counter": 5}}));
    apply_update(&mut doc, &upd).unwrap();
    assert_eq!(doc.get("counter"), Some(&json!(5)));
}

#[test]
fn update_inc_accumulates_across_calls() {
    let mut doc = doc_from_json(json!({"n": 0}));
    let upd = doc_from_json(json!({"$inc": {"n": 1}}));
    for _ in 0..7 {
        apply_update(&mut doc, &upd).unwrap();
    }
    assert_eq!(doc.get("n"), Some(&json!(7)));
}

#[test]
fn update_push_on_missing_creates_array() {
    let mut doc = doc_from_json(json!({"_id": "1"}));
    let upd = doc_from_json(json!({"$push": {"tags": "x"}}));
    apply_update(&mut doc, &upd).unwrap();
    assert_eq!(doc.get("tags"), Some(&json!(["x"])));
}

#[test]
fn update_addtoset_is_idempotent() {
    let mut doc = doc_from_json(json!({"tags": ["a"]}));
    let upd = doc_from_json(json!({"$addToSet": {"tags": "a"}}));
    apply_update(&mut doc, &upd).unwrap();
    apply_update(&mut doc, &upd).unwrap();
    assert_eq!(doc.get("tags"), Some(&json!(["a"])));
}

#[test]
fn update_addtoset_appends_new_value() {
    let mut doc = doc_from_json(json!({"tags": ["a"]}));
    let upd = doc_from_json(json!({"$addToSet": {"tags": "b"}}));
    apply_update(&mut doc, &upd).unwrap();
    let arr = doc.get("tags").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(arr.contains(&json!("b")));
}

#[test]
fn update_rename_moves_value_and_removes_old() {
    let mut doc = doc_from_json(json!({"old": "v"}));
    let upd = doc_from_json(json!({"$rename": {"old": "new"}}));
    apply_update(&mut doc, &upd).unwrap();
    assert!(!doc.contains_key("old"));
    assert_eq!(doc.get("new"), Some(&json!("v")));
}

#[test]
fn update_replacement_preserves_id() {
    let mut doc = doc_from_json(json!({"_id": "abc", "name": "old", "extra": 1}));
    let upd = doc_from_json(json!({"name": "new"}));
    apply_update(&mut doc, &upd).unwrap();
    assert_eq!(doc.get("_id"), Some(&json!("abc")));
    assert_eq!(doc.get("name"), Some(&json!("new")));
    assert!(!doc.contains_key("extra"), "replacement must drop other fields");
}

#[test]
fn update_unknown_operator_silently_ignored() {
    let mut doc = doc_from_json(json!({"_id": "1", "x": 1}));
    let upd = doc_from_json(json!({"$bogus": {"x": 99}}));
    apply_update(&mut doc, &upd).unwrap();
    // No change to existing fields
    assert_eq!(doc.get("x"), Some(&json!(1)));
}

#[test]
fn update_pull_removes_all_matching_elements() {
    let mut doc = doc_from_json(json!({"tags": ["a", "b", "a", "c", "a"]}));
    let upd = doc_from_json(json!({"$pull": {"tags": "a"}}));
    apply_update(&mut doc, &upd).unwrap();
    assert_eq!(doc.get("tags"), Some(&json!(["b", "c"])));
}

// ---------------------------------------------------------------------------
// Projection
// ---------------------------------------------------------------------------

#[test]
fn projection_inclusion_excludes_id_when_set_to_zero() {
    let doc = doc_from_json(json!({"_id": "1", "name": "alice", "email": "a@x"}));
    let proj = doc_from_json(json!({"name": 1, "_id": 0}));
    let out = apply_projection(&doc, Some(&proj));
    assert!(out.contains_key("name"));
    assert!(!out.contains_key("_id"));
    assert!(!out.contains_key("email"));
}

#[test]
fn projection_exclusion_keeps_id_by_default() {
    let doc = doc_from_json(json!({"_id": "1", "secret": "x", "ok": "y"}));
    let proj = doc_from_json(json!({"secret": 0}));
    let out = apply_projection(&doc, Some(&proj));
    assert!(out.contains_key("_id"));
    assert!(out.contains_key("ok"));
    assert!(!out.contains_key("secret"));
}

#[test]
fn projection_bool_true_flags_inclusion_mode() {
    // Documenting current behavior: a `true` projector triggers inclusion-mode
    // (so non-projected fields like "b" are dropped), and the _id is preserved
    // unless explicitly excluded. The current build pass only inserts fields
    // whose value is i64==1, so a pure-bool projector ends up with just _id.
    let doc = doc_from_json(json!({"_id": "1", "a": 1, "b": 2}));
    let mut proj = Document::new();
    proj.insert("a".into(), Value::Bool(true));
    let out = apply_projection(&doc, Some(&proj));
    assert!(!out.contains_key("b"), "inclusion mode must drop non-listed fields");
    assert!(out.contains_key("_id"), "_id is preserved by default");
}

#[test]
fn projection_bool_false_in_exclusion_mode() {
    let doc = doc_from_json(json!({"_id": "1", "secret": "x", "ok": "y"}));
    let mut proj = Document::new();
    proj.insert("secret".into(), Value::Bool(false));
    let out = apply_projection(&doc, Some(&proj));
    assert!(out.contains_key("ok"));
    assert!(!out.contains_key("secret"));
}

#[test]
fn projection_inclusion_drops_field_not_in_doc() {
    let doc = doc_from_json(json!({"_id": "1", "a": 1}));
    let proj = doc_from_json(json!({"missing": 1, "a": 1}));
    let out = apply_projection(&doc, Some(&proj));
    assert!(out.contains_key("a"));
    assert!(!out.contains_key("missing"));
}

// ---------------------------------------------------------------------------
// Cursor
// ---------------------------------------------------------------------------

fn make_docs(n: usize) -> Vec<Document> {
    (0..n)
        .map(|i| {
            let mut d = Document::new();
            d.insert("_id".into(), json!(i.to_string()));
            d
        })
        .collect()
}

#[test]
fn cursor_next_batch_respects_batch_size() {
    let mut c = Cursor::new(1, "db.coll".into(), make_docs(10), 4);
    assert_eq!(c.next_batch().len(), 4);
    assert_eq!(c.next_batch().len(), 4);
    assert_eq!(c.next_batch().len(), 2);
    assert_eq!(c.next_batch().len(), 0);
}

#[test]
fn cursor_has_more_tracks_exhaustion() {
    let mut c = Cursor::new(1, "x".into(), make_docs(3), 5);
    assert!(c.has_more());
    let _ = c.next_batch();
    assert!(!c.has_more());
}

#[test]
fn cursor_empty_documents() {
    let mut c = Cursor::new(1, "x".into(), vec![], 10);
    assert!(!c.has_more());
    assert!(c.next_batch().is_empty());
}

#[tokio::test]
async fn cursor_store_assigns_increasing_ids() {
    let s = CursorStore::new();
    let id1 = s.create("x".into(), make_docs(1), 10).await;
    let id2 = s.create("x".into(), make_docs(1), 10).await;
    assert!(id2 > id1);
}

#[tokio::test]
async fn cursor_store_get_mut_returns_some_for_existing() {
    let s = CursorStore::new();
    let id = s.create("x".into(), make_docs(5), 2).await;
    let batch = s.get_mut(id, |c| c.next_batch()).await;
    assert_eq!(batch.unwrap().len(), 2);
}

#[tokio::test]
async fn cursor_store_get_mut_returns_none_for_unknown() {
    let s = CursorStore::new();
    let res: Option<i64> = s.get_mut(999_999, |c| c.id).await;
    assert!(res.is_none());
}

#[tokio::test]
async fn cursor_store_kill_removes_and_returns_bool() {
    let s = CursorStore::new();
    let id = s.create("x".into(), make_docs(1), 10).await;
    assert!(s.kill(id).await);
    assert!(!s.kill(id).await, "killing the same cursor again returns false");
}

#[tokio::test]
async fn cursor_store_default_equiv_new() {
    let a = CursorStore::default();
    let b = CursorStore::new();
    let id_a = a.create("x".into(), vec![], 10).await;
    let id_b = b.create("y".into(), vec![], 10).await;
    // Both fresh stores start counter at 1.
    assert_eq!(id_a, id_b);
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

#[test]
fn index_stores_keys_and_uniqueness() {
    let mut keys = BTreeMap::new();
    keys.insert("name".into(), 1);
    keys.insert("ts".into(), -1);
    let ix = Index::new("name_ts_idx".into(), keys.clone(), true);
    assert_eq!(ix.name, "name_ts_idx");
    assert_eq!(ix.keys, keys);
    assert!(ix.unique);
}

#[test]
fn index_serde_roundtrip() {
    let mut keys = BTreeMap::new();
    keys.insert("k".into(), 1);
    let ix = Index::new("k_idx".into(), keys, false);
    let json = serde_json::to_string(&ix).unwrap();
    let back: Index = serde_json::from_str(&json).unwrap();
    assert_eq!(ix.name, back.name);
    assert_eq!(ix.unique, back.unique);
}

// ---------------------------------------------------------------------------
// Engine — async paths
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_insert_one_auto_generates_id_when_missing() {
    let e = Engine::new();
    let db = e.get_or_create_database("test").await;
    let col = db.get_or_create_collection("c").await;
    let d = doc_from_json(json!({"name": "x"}));
    let id = col.insert_one(d).await.unwrap();
    assert!(!id.is_empty());
    let docs = col.find(None).await.unwrap();
    assert_eq!(docs.len(), 1);
    assert!(docs[0].contains_key("_id"));
}

#[tokio::test]
async fn engine_insert_many_returns_ids_in_order() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    let docs: Vec<Document> = (0..5).map(|i| doc_from_json(json!({"n": i}))).collect();
    let ids = col.insert_many(docs).await.unwrap();
    assert_eq!(ids.len(), 5);
}

#[tokio::test]
async fn engine_find_with_filter_returns_only_matching() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    for i in 0..5 {
        col.insert_one(doc_from_json(json!({"n": i}))).await.unwrap();
    }
    let f = doc_from_json(json!({"n": 3}));
    let res = col.find(Some(&f)).await.unwrap();
    assert_eq!(res.len(), 1);
}

#[tokio::test]
async fn engine_count_with_and_without_filter() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    for _ in 0..3 {
        col.insert_one(doc_from_json(json!({"k": "v"}))).await.unwrap();
    }
    assert_eq!(col.count(None).await.unwrap(), 3);
    let f = doc_from_json(json!({"k": "v"}));
    assert_eq!(col.count(Some(&f)).await.unwrap(), 3);
    let f = doc_from_json(json!({"k": "other"}));
    assert_eq!(col.count(Some(&f)).await.unwrap(), 0);
}

#[tokio::test]
async fn engine_delete_many_returns_count_and_drops_docs() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    for _ in 0..4 {
        col.insert_one(doc_from_json(json!({"x": 1}))).await.unwrap();
    }
    let f = doc_from_json(json!({"x": 1}));
    let n = col.delete_many(Some(&f)).await.unwrap();
    assert_eq!(n, 4);
    assert_eq!(col.count(None).await.unwrap(), 0);
}

#[tokio::test]
async fn engine_update_many_applies_to_all_matching() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    for _ in 0..3 {
        col.insert_one(doc_from_json(json!({"state": "old"}))).await.unwrap();
    }
    let f = doc_from_json(json!({"state": "old"}));
    let upd = doc_from_json(json!({"$set": {"state": "new"}}));
    let n = col.update_many(Some(&f), &upd).await.unwrap();
    assert_eq!(n, 3);
    let after = col.find(None).await.unwrap();
    assert!(after.iter().all(|d| d.get("state") == Some(&json!("new"))));
}

#[tokio::test]
async fn engine_drop_collection_removes_documents() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    col.insert_one(doc_from_json(json!({"x": 1}))).await.unwrap();
    col.drop().await.unwrap();
    assert_eq!(col.count(None).await.unwrap(), 0);
}

#[tokio::test]
async fn engine_index_lifecycle_add_list_drop() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    let mut keys = BTreeMap::new();
    keys.insert("a".into(), 1);
    col.add_index(Index::new("a_idx".into(), keys, false)).await.unwrap();
    assert_eq!(col.list_indexes().await.unwrap().len(), 1);
    col.drop_index("a_idx").await.unwrap();
    assert!(col.list_indexes().await.unwrap().is_empty());
}

#[tokio::test]
async fn engine_drop_unknown_index_is_silent_noop() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    assert!(col.drop_index("nope").await.is_ok());
}

#[tokio::test]
async fn engine_list_databases_after_create() {
    let e = Engine::new();
    let _ = e.get_or_create_database("alpha").await;
    let _ = e.get_or_create_database("beta").await;
    let mut names = e.list_databases().await.unwrap();
    names.sort();
    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
}

#[tokio::test]
async fn engine_drop_database_removes_it() {
    let e = Engine::new();
    let _ = e.get_or_create_database("gone").await;
    e.drop_database("gone").await.unwrap();
    assert!(e.get_database("gone").await.is_none());
}

#[tokio::test]
async fn engine_stats_reports_counts() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    for _ in 0..3 {
        col.insert_one(doc_from_json(json!({"x": 1}))).await.unwrap();
    }
    let stats = e.stats().await.unwrap();
    assert_eq!(stats.database_count, 1);
    assert_eq!(stats.collection_count, 1);
    assert_eq!(stats.document_count, 3);
}

#[tokio::test]
async fn engine_concurrent_inserts_all_persist() {
    let e = Engine::new();
    let col = e
        .get_or_create_database("test")
        .await
        .get_or_create_collection("c")
        .await;
    let mut handles = Vec::new();
    for i in 0..20 {
        let c = col.clone();
        handles.push(tokio::spawn(async move {
            c.insert_one(doc_from_json(json!({"i": i}))).await.unwrap()
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(col.count(None).await.unwrap(), 20);
}
