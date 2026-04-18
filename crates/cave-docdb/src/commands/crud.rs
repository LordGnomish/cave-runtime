//! CRUD operations: find, insert, update, delete, count.

use crate::bson::Document;
use crate::cursor::CursorStore;
use crate::engine::Engine;
use crate::projection::apply_projection;
use serde_json::Value;
use std::sync::Arc;

pub async fn find(
    cmd_doc: &Document,
    engine: Arc<Engine>,
    cursors: Arc<CursorStore>,
) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .and_then(|k| {
            if cmd_doc.get(k).is_some() {
                Some(k.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "collection".to_string());

    let filter_spec = cmd_doc.get("filter");
    let filter = filter_spec.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let projection = cmd_doc.get("projection");
    let projection_doc = projection.as_ref().and_then(|p| {
        if let Value::Object(obj) = p {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let limit = cmd_doc
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as usize;
    let skip = cmd_doc
        .get("skip")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as usize;

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let docs = col.find(filter.as_ref()).await?;

    let mut filtered = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        if i < skip {
            continue;
        }
        if limit > 0 && filtered.len() >= limit {
            break;
        }
        let projected = apply_projection(doc, projection_doc.as_ref());
        filtered.push(projected);
    }

    let cursor_id = cursors.create(
        format!("{}.{}", db_name, col_name),
        filtered.clone(),
        100,
    ).await;

    let mut cursor = serde_json::Map::new();
    cursor.insert("id".to_string(), Value::Number(cursor_id.into()));
    cursor.insert(
        "ns".to_string(),
        Value::String(format!("{}.{}", db_name, col_name)),
    );

    let firstBatch: Vec<Value> = filtered
        .iter()
        .map(|doc| {
            Value::Object(
                doc.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
        .collect();
    cursor.insert("firstBatch".to_string(), Value::Array(firstBatch));

    let mut resp = Document::new();
    resp.insert("cursor".to_string(), Value::Object(cursor));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn insert(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$') && *k != "documents")
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let documents = cmd_doc
        .get("documents")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing documents".to_string())?;

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let mut ids = Vec::new();
    for doc_val in documents {
        if let Some(obj) = doc_val.as_object() {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            let id = col.insert_one(doc).await?;
            ids.push(id);
        }
    }

    let mut resp = Document::new();
    resp.insert(
        "insertedIds".to_string(),
        Value::Array(ids.iter().map(|id| Value::String(id.clone())).collect()),
    );
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn update(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let filter_spec = cmd_doc.get("filter");
    let filter = filter_spec.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let update_spec = cmd_doc
        .get("update")
        .ok_or_else(|| "missing update".to_string())?;

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    if let Some(obj) = update_spec.as_object() {
        let mut update = Document::new();
        for (k, v) in obj {
            update.insert(k.clone(), v.clone());
        }

        let modified = col.update_many(filter.as_ref(), &update).await?;

        let mut resp = Document::new();
        resp.insert("modifiedCount".to_string(), Value::Number(modified.into()));
        resp.insert("ok".to_string(), Value::Number(1.into()));
        Ok(resp)
    } else {
        Err("invalid update spec".to_string())
    }
}

pub async fn delete(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let filter_spec = cmd_doc.get("filter");
    let filter = filter_spec.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let deleted = col.delete_many(filter.as_ref()).await?;

    let mut resp = Document::new();
    resp.insert("deletedCount".to_string(), Value::Number(deleted.into()));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn count(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    let col_name = cmd_doc
        .keys()
        .find(|k| !k.starts_with('$'))
        .cloned()
        .unwrap_or_else(|| "collection".to_string());

    let filter_spec = cmd_doc.get("filter");
    let filter = filter_spec.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let count = col.count(filter.as_ref()).await?;

    let mut resp = Document::new();
    resp.insert("n".to_string(), Value::Number(count.into()));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}
