// SPDX-License-Identifier: AGPL-3.0-or-later
//! Index management commands.

use crate::bson::Document;
use crate::engine::Engine;
use crate::index::Index;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub async fn create_indexes(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let indexes = cmd_doc
        .get("indexes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing indexes".to_string())?;

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    for idx_spec in indexes {
        if let Some(idx_obj) = idx_spec.as_object() {
            let mut keys = BTreeMap::new();
            if let Some(key_spec) = idx_obj.get("key").and_then(|v| v.as_object()) {
                for (k, v) in key_spec {
                    if let Some(direction) = v.as_i64() {
                        keys.insert(k.clone(), direction as i32);
                    }
                }
            }

            let unique = idx_obj
                .get("unique")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let name = idx_obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("index")
                .to_string();

            let index = Index::new(name, keys, unique);
            col.add_index(index).await?;
        }
    }

    let mut resp = Document::new();
    resp.insert("numIndexesAfter".to_string(), Value::Number((indexes.len()).into()));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn drop_indexes(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let index_name = cmd_doc
        .get("index")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    if let Some(name) = index_name {
        col.drop_index(&name).await?;
    }

    let mut resp = Document::new();
    resp.insert("nIndexesWas".to_string(), Value::Number(1.into()));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn list_indexes(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let indexes = col.list_indexes().await?;

    let mut cursor = serde_json::Map::new();
    cursor.insert("id".to_string(), Value::Number(0.into()));
    cursor.insert(
        "ns".to_string(),
        Value::String(format!("{}.{}.$cmd.listIndexes", db_name, col_name)),
    );

    let first_batch: Vec<Value> = indexes
        .iter()
        .map(|idx| {
            let mut idx_obj = serde_json::Map::new();
            let mut keys = serde_json::Map::new();
            for (k, v) in &idx.keys {
                keys.insert(k.clone(), Value::Number((*v as i64).into()));
            }
            idx_obj.insert("key".to_string(), Value::Object(keys));
            idx_obj.insert("name".to_string(), Value::String(idx.name.clone()));
            if idx.unique {
                idx_obj.insert("unique".to_string(), Value::Bool(true));
            }
            Value::Object(idx_obj)
        })
        .collect();
    cursor.insert("firstBatch".to_string(), Value::Array(first_batch));

    let mut resp = Document::new();
    resp.insert("cursor".to_string(), Value::Object(cursor));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}
