// SPDX-License-Identifier: AGPL-3.0-or-later
//! Backend trait integration — RED→GREEN suite.

use cave_docdb::backend::{Backend, MemoryBackend, WriteResult};
use serde_json::json;

fn doc(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn memory_backend_insert_then_find() {
    let b = MemoryBackend::new();
    let r = b
        .insert("orders", "items", vec![doc(json!({"sku": "A", "qty": 3}))])
        .await
        .unwrap();
    assert_eq!(r.affected, 1);
    let rows = b.find("orders", "items", Some(doc(json!({"sku": "A"})))).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("sku").unwrap(), "A");
}

#[tokio::test]
async fn memory_backend_update_one_field() {
    let b = MemoryBackend::new();
    b.insert("d", "c", vec![doc(json!({"_id":"x","v":1}))]).await.unwrap();
    let r = b
        .update(
            "d",
            "c",
            Some(doc(json!({"_id": "x"}))),
            doc(json!({"$set": {"v": 99}})),
        )
        .await
        .unwrap();
    assert_eq!(r.affected, 1);
    let rows = b.find("d", "c", Some(doc(json!({"_id":"x"})))).await.unwrap();
    assert_eq!(rows[0].get("v").unwrap().as_i64().unwrap(), 99);
}

#[tokio::test]
async fn memory_backend_delete_then_count_zero() {
    let b = MemoryBackend::new();
    b.insert("d", "c", vec![doc(json!({"sku":"X"}))]).await.unwrap();
    let r = b.delete("d", "c", Some(doc(json!({"sku":"X"})))).await.unwrap();
    assert_eq!(r.affected, 1);
    assert_eq!(b.count("d", "c", None).await.unwrap(), 0);
}

#[tokio::test]
async fn memory_backend_list_collections() {
    let b = MemoryBackend::new();
    b.insert("orders", "items", vec![doc(json!({"a":1}))]).await.unwrap();
    b.insert("orders", "log", vec![doc(json!({"a":1}))]).await.unwrap();
    let mut cols = b.list_collections("orders").await.unwrap();
    cols.sort();
    assert_eq!(cols, vec!["items".to_string(), "log".to_string()]);
}

#[tokio::test]
async fn memory_backend_list_databases() {
    let b = MemoryBackend::new();
    b.insert("a", "c", vec![doc(json!({"x":1}))]).await.unwrap();
    b.insert("b", "c", vec![doc(json!({"x":1}))]).await.unwrap();
    let mut dbs = b.list_databases().await.unwrap();
    dbs.sort();
    assert_eq!(dbs, vec!["a".to_string(), "b".to_string()]);
}

#[tokio::test]
async fn memory_backend_drop_collection_removes_data() {
    let b = MemoryBackend::new();
    b.insert("d", "c", vec![doc(json!({"v":1}))]).await.unwrap();
    b.drop_collection("d", "c").await.unwrap();
    assert_eq!(b.count("d", "c", None).await.unwrap(), 0);
}

#[tokio::test]
async fn memory_backend_drop_database_removes_collections() {
    let b = MemoryBackend::new();
    b.insert("d", "c1", vec![doc(json!({"v":1}))]).await.unwrap();
    b.insert("d", "c2", vec![doc(json!({"v":1}))]).await.unwrap();
    b.drop_database("d").await.unwrap();
    assert!(b.list_collections("d").await.unwrap().is_empty());
}

#[tokio::test]
async fn write_result_records_affected() {
    let r = WriteResult { affected: 7 };
    assert_eq!(r.affected, 7);
}
