//! cave-docdb integration tests.
//!
//! Upstream parity reference: MongoDB community `jstests/core` (CRUD, query
//! operators, projection, $set/$inc/$push, indexes) and the BSON spec
//! (bsonspec.org). All tests run against the in-memory engine — no mongod
//! required.

use cave_docdb::bson::{decode_doc, encode_doc, Document};
use cave_docdb::cursor::{Cursor, CursorStore};
use cave_docdb::engine::Engine;
use cave_docdb::index::Index;
use cave_docdb::projection::apply_projection;
use cave_docdb::query::{matches_query, matches_value};
use cave_docdb::update::apply_update;
use serde_json::{json, Value};
use std::collections::BTreeMap;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn doc(pairs: &[(&str, Value)]) -> Document {
    let mut d = Document::new();
    for (k, v) in pairs {
        d.insert((*k).to_string(), v.clone());
    }
    d
}

fn op(name: &str, val: Value) -> Value {
    let mut m = serde_json::Map::new();
    m.insert(name.to_string(), val);
    Value::Object(m)
}

// ── BSON encode/decode ───────────────────────────────────────────────────────

#[test]
fn bson_round_trip_string() {
    let original = doc(&[("greeting", json!("hello world"))]);
    let bytes = encode_doc(&original).unwrap();
    let decoded = decode_doc(&bytes).unwrap();
    assert_eq!(decoded.get("greeting"), Some(&json!("hello world")));
}

#[test]
fn bson_round_trip_int32() {
    let original = doc(&[("n", json!(42))]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    assert_eq!(decoded.get("n").and_then(|v| v.as_i64()), Some(42));
}

#[test]
fn bson_round_trip_int64_large_value() {
    let original = doc(&[("big", json!(10_000_000_000_i64))]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    assert_eq!(decoded.get("big").and_then(|v| v.as_i64()), Some(10_000_000_000_i64));
}

#[test]
fn bson_round_trip_double() {
    let original = doc(&[("pi", json!(3.14))]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    let v = decoded.get("pi").unwrap().as_f64().unwrap();
    assert!((v - 3.14).abs() < 1e-9);
}

#[test]
fn bson_round_trip_bool() {
    let original = doc(&[("on", json!(true)), ("off", json!(false))]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    assert_eq!(decoded.get("on"), Some(&json!(true)));
    assert_eq!(decoded.get("off"), Some(&json!(false)));
}

#[test]
fn bson_round_trip_null() {
    let original = doc(&[("nothing", Value::Null)]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    assert_eq!(decoded.get("nothing"), Some(&Value::Null));
}

#[test]
fn bson_round_trip_array() {
    let original = doc(&[("nums", json!([1, 2, 3]))]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    let arr = decoded.get("nums").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_i64(), Some(1));
}

#[test]
fn bson_round_trip_nested_doc() {
    let original = doc(&[("user", json!({"name": "alice", "age": 30}))]);
    let decoded = decode_doc(&encode_doc(&original).unwrap()).unwrap();
    let inner = decoded.get("user").unwrap().as_object().unwrap();
    assert_eq!(inner.get("name"), Some(&json!("alice")));
}

#[test]
fn bson_round_trip_object_id() {
    // 24 hex chars get encoded as ObjectId
    let original = doc(&[("_id", json!("507f1f77bcf86cd799439011"))]);
    let bytes = encode_doc(&original).unwrap();
    let decoded = decode_doc(&bytes).unwrap();
    assert_eq!(decoded.get("_id"), Some(&json!("507f1f77bcf86cd799439011")));
}

#[test]
fn bson_decode_too_short_errors() {
    assert!(decode_doc(&[0, 0]).is_err());
}

// ── Query operators ──────────────────────────────────────────────────────────

#[test]
fn query_implicit_equality() {
    let d = doc(&[("status", json!("active"))]);
    assert!(matches_query(&d, &doc(&[("status", json!("active"))])));
    assert!(!matches_query(&d, &doc(&[("status", json!("inactive"))])));
}

#[test]
fn query_eq_operator() {
    let d = doc(&[("age", json!(30))]);
    assert!(matches_query(&d, &doc(&[("age", op("$eq", json!(30)))])));
}

#[test]
fn query_ne_operator() {
    let d = doc(&[("status", json!("active"))]);
    assert!(matches_query(&d, &doc(&[("status", op("$ne", json!("inactive")))])));
    assert!(!matches_query(&d, &doc(&[("status", op("$ne", json!("active")))])));
}

#[test]
fn query_gt_gte_operators() {
    let d = doc(&[("score", json!(50))]);
    assert!(matches_query(&d, &doc(&[("score", op("$gt", json!(10)))])));
    assert!(!matches_query(&d, &doc(&[("score", op("$gt", json!(50)))])));
    assert!(matches_query(&d, &doc(&[("score", op("$gte", json!(50)))])));
}

#[test]
fn query_lt_lte_operators() {
    let d = doc(&[("score", json!(50))]);
    assert!(matches_query(&d, &doc(&[("score", op("$lt", json!(100)))])));
    assert!(matches_query(&d, &doc(&[("score", op("$lte", json!(50)))])));
    assert!(!matches_query(&d, &doc(&[("score", op("$lt", json!(50)))])));
}

#[test]
fn query_in_operator() {
    let d = doc(&[("role", json!("admin"))]);
    assert!(matches_query(
        &d,
        &doc(&[("role", op("$in", json!(["admin", "user"])))])
    ));
    assert!(!matches_query(
        &d,
        &doc(&[("role", op("$in", json!(["banned"])))])
    ));
}

#[test]
fn query_nin_operator() {
    let d = doc(&[("role", json!("admin"))]);
    assert!(matches_query(
        &d,
        &doc(&[("role", op("$nin", json!(["banned"])))])
    ));
    assert!(!matches_query(
        &d,
        &doc(&[("role", op("$nin", json!(["admin"])))])
    ));
}

#[test]
fn query_regex_operator() {
    let d = doc(&[("email", json!("user@example.com"))]);
    assert!(matches_query(
        &d,
        &doc(&[("email", op("$regex", json!(".*@example\\.com")))])
    ));
}

#[test]
fn query_and_operator_all_must_match() {
    let d = doc(&[("age", json!(25)), ("status", json!("active"))]);
    let q = doc(&[(
        "$and",
        json!([{"age": 25}, {"status": "active"}]),
    )]);
    assert!(matches_query(&d, &q));
}

#[test]
fn query_and_short_circuits_on_mismatch() {
    let d = doc(&[("age", json!(25)), ("status", json!("active"))]);
    let q = doc(&[(
        "$and",
        json!([{"age": 25}, {"status": "banned"}]),
    )]);
    assert!(!matches_query(&d, &q));
}

#[test]
fn query_or_operator_any_match() {
    let d = doc(&[("status", json!("active"))]);
    let q = doc(&[(
        "$or",
        json!([{"status": "banned"}, {"status": "active"}]),
    )]);
    assert!(matches_query(&d, &q));
}

#[test]
fn query_exists_false_matches_missing_field() {
    let d = doc(&[("name", json!("alice"))]);
    assert!(matches_query(
        &d,
        &doc(&[("missing", op("$exists", json!(false)))])
    ));
}

#[test]
fn query_missing_field_implicit_match_fails() {
    let d = doc(&[("name", json!("alice"))]);
    assert!(!matches_query(&d, &doc(&[("age", json!(20))])));
}

#[test]
fn matches_value_unknown_operator_does_not_match_other_keys() {
    // Unknown operators are ignored — value still passes if no other constraints fail.
    let v = json!(42);
    assert!(matches_value(&v, &op("$unknown", json!(1))));
}

// ── Update operators ─────────────────────────────────────────────────────────

#[test]
fn update_set_overwrites_field() {
    let mut d = doc(&[("_id", json!("1")), ("name", json!("old"))]);
    apply_update(&mut d, &doc(&[("$set", json!({"name": "new"}))])).unwrap();
    assert_eq!(d.get("name"), Some(&json!("new")));
}

#[test]
fn update_unset_removes_field() {
    let mut d = doc(&[("_id", json!("1")), ("temp", json!("data"))]);
    apply_update(&mut d, &doc(&[("$unset", json!({"temp": 1}))])).unwrap();
    assert!(!d.contains_key("temp"));
}

#[test]
fn update_inc_increments_existing() {
    let mut d = doc(&[("_id", json!("1")), ("counter", json!(5))]);
    apply_update(&mut d, &doc(&[("$inc", json!({"counter": 3}))])).unwrap();
    assert_eq!(d.get("counter").and_then(|v| v.as_i64()), Some(8));
}

#[test]
fn update_inc_initializes_missing_field() {
    let mut d = doc(&[("_id", json!("1"))]);
    apply_update(&mut d, &doc(&[("$inc", json!({"new_count": 7}))])).unwrap();
    assert_eq!(d.get("new_count").and_then(|v| v.as_i64()), Some(7));
}

#[test]
fn update_push_appends_to_array() {
    let mut d = doc(&[("_id", json!("1")), ("tags", json!(["a"]))]);
    apply_update(&mut d, &doc(&[("$push", json!({"tags": "b"}))])).unwrap();
    let arr = d.get("tags").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[1], json!("b"));
}

#[test]
fn update_pull_removes_from_array() {
    let mut d = doc(&[("_id", json!("1")), ("tags", json!(["a", "b", "c"]))]);
    apply_update(&mut d, &doc(&[("$pull", json!({"tags": "b"}))])).unwrap();
    let arr = d.get("tags").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert!(!arr.contains(&json!("b")));
}

#[test]
fn update_add_to_set_avoids_duplicates() {
    let mut d = doc(&[("_id", json!("1")), ("tags", json!(["a"]))]);
    apply_update(&mut d, &doc(&[("$addToSet", json!({"tags": "a"}))])).unwrap();
    apply_update(&mut d, &doc(&[("$addToSet", json!({"tags": "b"}))])).unwrap();
    let arr = d.get("tags").unwrap().as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn update_rename_moves_field() {
    let mut d = doc(&[("_id", json!("1")), ("old_name", json!("v"))]);
    apply_update(&mut d, &doc(&[("$rename", json!({"old_name": "new_name"}))])).unwrap();
    assert!(!d.contains_key("old_name"));
    assert_eq!(d.get("new_name"), Some(&json!("v")));
}

#[test]
fn update_replacement_keeps_id() {
    let mut d = doc(&[("_id", json!("1")), ("a", json!(1)), ("b", json!(2))]);
    apply_update(&mut d, &doc(&[("c", json!(3))])).unwrap();
    assert!(!d.contains_key("a"));
    assert_eq!(d.get("c"), Some(&json!(3)));
    assert_eq!(d.get("_id"), Some(&json!("1")));
}

// ── Projection ───────────────────────────────────────────────────────────────

#[test]
fn projection_includes_specified_fields() {
    let d = doc(&[
        ("_id", json!("1")),
        ("name", json!("alice")),
        ("email", json!("a@b.c")),
    ]);
    let proj = doc(&[("name", json!(1))]);
    let result = apply_projection(&d, Some(&proj));
    assert!(result.contains_key("name"));
    assert!(result.contains_key("_id"));
    assert!(!result.contains_key("email"));
}

#[test]
fn projection_excludes_specified_fields() {
    let d = doc(&[
        ("_id", json!("1")),
        ("name", json!("alice")),
        ("password", json!("secret")),
    ]);
    let proj = doc(&[("password", json!(0))]);
    let result = apply_projection(&d, Some(&proj));
    assert!(!result.contains_key("password"));
    assert!(result.contains_key("name"));
}

#[test]
fn projection_excludes_id_when_requested() {
    let d = doc(&[("_id", json!("1")), ("name", json!("alice"))]);
    let proj = doc(&[("name", json!(1)), ("_id", json!(0))]);
    let result = apply_projection(&d, Some(&proj));
    assert!(!result.contains_key("_id"));
    assert!(result.contains_key("name"));
}

#[test]
fn projection_none_returns_full() {
    let d = doc(&[("a", json!(1)), ("b", json!(2))]);
    let result = apply_projection(&d, None);
    assert_eq!(result, d);
}

// ── Engine / Database / Collection ───────────────────────────────────────────

#[tokio::test]
async fn engine_create_database_and_collection() {
    let engine = Engine::new();
    let db = engine.get_or_create_database("app").await;
    let col = db.get_or_create_collection("users").await;
    let _ = col.insert_one(doc(&[("name", json!("alice"))])).await.unwrap();
    let stats = col.stats().await.unwrap();
    assert_eq!(stats.document_count, 1);
}

#[tokio::test]
async fn engine_lists_databases() {
    let engine = Engine::new();
    engine.get_or_create_database("a").await;
    engine.get_or_create_database("b").await;
    let dbs = engine.list_databases().await.unwrap();
    assert!(dbs.contains(&"a".to_string()));
    assert!(dbs.contains(&"b".to_string()));
}

#[tokio::test]
async fn engine_drop_database() {
    let engine = Engine::new();
    engine.get_or_create_database("doomed").await;
    engine.drop_database("doomed").await.unwrap();
    let dbs = engine.list_databases().await.unwrap();
    assert!(!dbs.contains(&"doomed".to_string()));
}

#[tokio::test]
async fn engine_stats_aggregates() {
    let engine = Engine::new();
    let db = engine.get_or_create_database("a").await;
    let col = db.get_or_create_collection("c").await;
    col.insert_one(doc(&[("x", json!(1))])).await.unwrap();
    col.insert_one(doc(&[("x", json!(2))])).await.unwrap();
    let stats = engine.stats().await.unwrap();
    assert_eq!(stats.database_count, 1);
    assert_eq!(stats.collection_count, 1);
    assert_eq!(stats.document_count, 2);
}

#[tokio::test]
async fn collection_auto_assigns_id_when_missing() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    let id = col.insert_one(doc(&[("name", json!("no-id"))])).await.unwrap();
    assert!(!id.is_empty());
    let found = col.find(None).await.unwrap();
    assert_eq!(found[0].get("_id").and_then(|v| v.as_str()), Some(id.as_str()));
}

#[tokio::test]
async fn collection_insert_many_returns_ids() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    let ids = col
        .insert_many(vec![
            doc(&[("a", json!(1))]),
            doc(&[("a", json!(2))]),
            doc(&[("a", json!(3))]),
        ])
        .await
        .unwrap();
    assert_eq!(ids.len(), 3);
}

#[tokio::test]
async fn collection_find_with_filter() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    col.insert_one(doc(&[("status", json!("active"))])).await.unwrap();
    col.insert_one(doc(&[("status", json!("inactive"))])).await.unwrap();
    let active = col
        .find(Some(&doc(&[("status", json!("active"))])))
        .await
        .unwrap();
    assert_eq!(active.len(), 1);
}

#[tokio::test]
async fn collection_find_one() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    col.insert_one(doc(&[("k", json!(1))])).await.unwrap();
    col.insert_one(doc(&[("k", json!(2))])).await.unwrap();
    let one = col.find_one(None).await.unwrap();
    assert!(one.is_some());
}

#[tokio::test]
async fn collection_update_many_with_set() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    for _ in 0..3 {
        col.insert_one(doc(&[("status", json!("pending"))])).await.unwrap();
    }
    let updated = col
        .update_many(None, &doc(&[("$set", json!({"status": "done"}))]))
        .await
        .unwrap();
    assert_eq!(updated, 3);
    let pending = col
        .find(Some(&doc(&[("status", json!("pending"))])))
        .await
        .unwrap();
    assert_eq!(pending.len(), 0);
}

#[tokio::test]
async fn collection_delete_many_with_filter() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    col.insert_one(doc(&[("type", json!("a"))])).await.unwrap();
    col.insert_one(doc(&[("type", json!("a"))])).await.unwrap();
    col.insert_one(doc(&[("type", json!("b"))])).await.unwrap();
    let deleted = col
        .delete_many(Some(&doc(&[("type", json!("a"))])))
        .await
        .unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(col.count(None).await.unwrap(), 1);
}

#[tokio::test]
async fn collection_count_with_and_without_filter() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    for i in 0..5 {
        let v = if i % 2 == 0 { "even" } else { "odd" };
        col.insert_one(doc(&[("k", json!(v))])).await.unwrap();
    }
    assert_eq!(col.count(None).await.unwrap(), 5);
    assert_eq!(
        col.count(Some(&doc(&[("k", json!("odd"))]))).await.unwrap(),
        2
    );
}

#[tokio::test]
async fn collection_drop_clears_data_and_indexes() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    col.insert_one(doc(&[("a", json!(1))])).await.unwrap();
    let mut keys = BTreeMap::new();
    keys.insert("a".into(), 1);
    col.add_index(Index::new("idx_a".into(), keys, false)).await.unwrap();
    col.drop().await.unwrap();
    let stats = col.stats().await.unwrap();
    assert_eq!(stats.document_count, 0);
    assert_eq!(stats.index_count, 0);
}

#[tokio::test]
async fn collection_index_management() {
    let engine = Engine::new();
    let col = engine
        .get_or_create_database("a")
        .await
        .get_or_create_collection("c")
        .await;
    let mut keys = BTreeMap::new();
    keys.insert("email".into(), 1);
    col.add_index(Index::new("idx_email".into(), keys, true))
        .await
        .unwrap();
    let indexes = col.list_indexes().await.unwrap();
    assert_eq!(indexes.len(), 1);
    assert!(indexes[0].unique);
    col.drop_index("idx_email").await.unwrap();
    assert!(col.list_indexes().await.unwrap().is_empty());
}

#[tokio::test]
async fn database_drop_collection() {
    let db = Engine::new().get_or_create_database("a").await;
    let col = db.get_or_create_collection("doomed").await;
    col.insert_one(doc(&[("x", json!(1))])).await.unwrap();
    db.drop_collection("doomed").await.unwrap();
    let cols = db.list_collections().await.unwrap();
    assert!(!cols.contains(&"doomed".to_string()));
}

#[tokio::test]
async fn database_drop_clears_all_collections() {
    let db = Engine::new().get_or_create_database("a").await;
    db.get_or_create_collection("c1").await;
    db.get_or_create_collection("c2").await;
    db.drop().await.unwrap();
    assert!(db.list_collections().await.unwrap().is_empty());
}

// ── Cursor ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cursor_batches_documents() {
    let docs: Vec<Document> = (0..10)
        .map(|i| doc(&[("n", json!(i))]))
        .collect();
    let mut cur = Cursor::new(1, "ns".into(), docs, 3);
    let b1 = cur.next_batch();
    assert_eq!(b1.len(), 3);
    assert!(cur.has_more());
    let b2 = cur.next_batch();
    assert_eq!(b2.len(), 3);
    let b3 = cur.next_batch();
    assert_eq!(b3.len(), 3);
    let b4 = cur.next_batch();
    assert_eq!(b4.len(), 1);
    assert!(!cur.has_more());
}

#[tokio::test]
async fn cursor_store_create_get_kill() {
    let store = CursorStore::new();
    let id = store.create("ns".into(), vec![doc(&[("a", json!(1))])], 10).await;
    let len = store.get_mut(id, |c| c.documents.len()).await;
    assert_eq!(len, Some(1));
    assert!(store.kill(id).await);
    assert!(!store.kill(id).await);
}

// ── Index model ──────────────────────────────────────────────────────────────

#[test]
fn index_serializes_to_json() {
    let mut keys = BTreeMap::new();
    keys.insert("created".into(), -1);
    let idx = Index::new("idx_created".into(), keys, false);
    let json = serde_json::to_string(&idx).unwrap();
    assert!(json.contains("idx_created"));
    let back: Index = serde_json::from_str(&json).unwrap();
    assert_eq!(back.keys.get("created"), Some(&-1));
}
