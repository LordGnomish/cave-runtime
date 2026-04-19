//! Cursor management: getMore, killCursors.

use crate::bson::Document;
use crate::cursor::CursorStore;
use serde_json::Value;
use std::sync::Arc;

pub async fn get_more(cmd_doc: &Document, cursors: Arc<CursorStore>) -> Result<Document, String> {
    let cursor_id = cmd_doc
        .get("getMore")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "missing cursor id".to_string())?;

    let batch = cursors
        .get_mut(cursor_id, |cursor| {
            let batch = cursor.next_batch();
            let has_more = cursor.has_more();
            (batch, has_more, cursor.ns.clone())
        })
        .await
        .ok_or_else(|| "cursor not found".to_string())?;

    let (batch, has_more, ns) = batch;

    let mut cursor = serde_json::Map::new();
    cursor.insert("id".to_string(), Value::Number(if has_more { cursor_id } else { 0 }.into()));
    cursor.insert("ns".to_string(), Value::String(ns));

    let next_batch: Vec<Value> = batch
        .iter()
        .map(|doc| {
            Value::Object(
                doc.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
        .collect();
    cursor.insert("nextBatch".to_string(), Value::Array(next_batch));

    let mut resp = Document::new();
    resp.insert("cursor".to_string(), Value::Object(cursor));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

pub async fn kill_cursors(cmd_doc: &Document, cursors: Arc<CursorStore>) -> Result<Document, String> {
    let cursor_ids = cmd_doc
        .get("killCursors")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing cursor ids".to_string())?;

    for cursor_id_val in cursor_ids {
        if let Some(id) = cursor_id_val.as_i64() {
            let _ = cursors.kill(id).await;
        }
    }

    let mut resp = Document::new();
    resp.insert("cursorsKilled".to_string(), Value::Array(
        cursor_ids.iter().filter_map(|v| v.as_i64().map(|id| Value::Number(id.into()))).collect()
    ));
    resp.insert("cursorsNotFound".to_string(), Value::Array(vec![]));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}
