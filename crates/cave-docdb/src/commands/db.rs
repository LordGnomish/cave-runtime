//! Database and collection operations.

use crate::bson::Document;
use crate::engine::Engine;
use serde_json::Value;
use std::sync::Arc;

pub async fn list_databases(engine: Arc<Engine>) -> Result<Document, String> {
    let databases = engine.list_databases().await?;
    let mut dbs = Vec::new();

    for db_name in databases {
        let mut db_obj = serde_json::Map::new();
        db_obj.insert("name".to_string(), Value::String(db_name));
        db_obj.insert("sizeOnDisk".to_string(), Value::Number(0.into()));
        db_obj.insert("empty".to_string(), Value::Bool(false));
        dbs.push(Value::Object(db_obj));
    }

    let mut resp = Document::new();
    resp.insert("databases".to_string(), Value::Array(dbs));
    resp.insert("totalSize".to_string(), Value::Number(0.into()));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn list_collections(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("listCollections")
        .and_then(|v| {
            if matches!(v, Value::Number(_)) {
                Some("current_db".to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            cmd_doc.get("$db").and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "admin".to_string());

    let db = match engine.get_database(&db_name).await {
        Some(db) => db,
        None => {
            let mut resp = Document::new();
            resp.insert("cursor".to_string(), Value::Object(serde_json::Map::new()));
            resp.insert("ok".to_string(), Value::Number(1.into()));
            return Ok(resp);
        }
    };

    let collections = db.list_collections().await?;
    let mut cursor = serde_json::Map::new();
    cursor.insert("id".to_string(), Value::Number(0.into()));
    cursor.insert("ns".to_string(), Value::String(format!("{}.$cmd.listCollections", db_name)));

    let mut firstBatch = Vec::new();
    for col_name in collections {
        let mut col_obj = serde_json::Map::new();
        col_obj.insert("name".to_string(), Value::String(col_name));
        col_obj.insert("type".to_string(), Value::String("collection".to_string()));
        firstBatch.push(Value::Object(col_obj));
    }
    cursor.insert("firstBatch".to_string(), Value::Array(firstBatch));

    let mut resp = Document::new();
    resp.insert("cursor".to_string(), Value::Object(cursor));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn create_collection(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
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

    let _db = engine.get_or_create_database(&db_name).await;
    let _col = _db.get_or_create_collection(&col_name).await;

    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn drop_collection(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
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

    if let Some(db) = engine.get_database(&db_name).await {
        db.drop_collection(&col_name).await?;
    }

    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn drop_database(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("dropDatabase")
        .and_then(|v| {
            if matches!(v, Value::Number(_)) {
                Some("current_db".to_string())
            } else {
                None
            }
        })
        .or_else(|| {
            cmd_doc.get("$db").and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "test".to_string());

    engine.drop_database(&db_name).await?;

    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}
