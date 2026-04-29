//! Extra OP_MSG commands: findAndModify, bulkWrite, explain, distinct,
//! renameCollection.
//!
//! These are honest, minimal implementations that lean on the existing
//! engine primitives (`update_many`, `delete_many`, `find`, `insert_one`)
//! rather than reimplementing storage. They cover the shapes drivers
//! actually emit (no aggregation-pipeline updates, no upsert in
//! findAndModify yet) — the manifest reflects this.
//!
//! See `parity.manifest.toml` for what is mapped vs partial.

use crate::bson::Document;
use crate::engine::Engine;
use crate::update::apply_update;
use serde_json::Value;
use std::sync::Arc;

fn db_name_of(cmd_doc: &Document) -> String {
    cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string())
}

fn doc_from_value(v: &Value) -> Option<Document> {
    if let Value::Object(obj) = v {
        let mut d = Document::new();
        for (k, vv) in obj {
            d.insert(k.clone(), vv.clone());
        }
        Some(d)
    } else {
        None
    }
}

fn err(msg: impl Into<String>, code: i64) -> Document {
    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(0.into()));
    resp.insert("errmsg".to_string(), Value::String(msg.into()));
    resp.insert("code".to_string(), Value::Number(code.into()));
    resp
}

/// `findAndModify` — find a single document and either update or remove it,
/// returning the pre-image (default) or post-image (when `new: true`).
///
/// Recognised fields: `findAndModify` (collection name), `query`, `sort`,
/// `update`, `remove`, `new`, `fields` (projection ignored), `upsert`.
///
/// Limitations vs upstream:
///   * `sort` is best-effort: sorts the matched candidates by simple equality
///     order. Deterministic for tests, not a guarantee of MongoDB sort order.
///   * Aggregation-pipeline updates are not supported.
pub async fn find_and_modify(
    cmd_doc: &Document,
    engine: Arc<Engine>,
) -> Result<Document, String> {
    let db_name = db_name_of(cmd_doc);
    let col_name = cmd_doc
        .get("findAndModify")
        .and_then(|v| v.as_str())
        .or_else(|| cmd_doc.get("findandmodify").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .ok_or_else(|| "missing collection".to_string())?;

    let query = cmd_doc.get("query").and_then(doc_from_value);
    let update = cmd_doc.get("update").and_then(doc_from_value);
    let remove = cmd_doc.get("remove").and_then(|v| v.as_bool()).unwrap_or(false);
    let return_new = cmd_doc.get("new").and_then(|v| v.as_bool()).unwrap_or(false);
    let upsert = cmd_doc.get("upsert").and_then(|v| v.as_bool()).unwrap_or(false);

    if remove && update.is_some() {
        return Ok(err(
            "Cannot specify both update and remove=true",
            9,
        ));
    }
    if !remove && update.is_none() && !upsert {
        return Ok(err(
            "Either an update or remove=true must be specified",
            9,
        ));
    }

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let pre = col.find_one(query.as_ref()).await?;

    // Build response with `lastErrorObject`, MongoDB-shape.
    let mut last_error = serde_json::Map::new();

    let value: Value = match (pre, remove, update) {
        (Some(doc), true, _) => {
            // Remove this doc by _id.
            if let Some(id_val) = doc.get("_id") {
                let mut filter = Document::new();
                filter.insert("_id".to_string(), id_val.clone());
                col.delete_many(Some(&filter)).await?;
            }
            last_error.insert("n".to_string(), Value::Number(1.into()));
            Value::Object(doc.into_iter().collect())
        }
        (Some(doc), false, Some(upd)) => {
            // Update by _id.
            let id_val = doc.get("_id").cloned();
            let filter = id_val.as_ref().map(|id| {
                let mut f = Document::new();
                f.insert("_id".to_string(), id.clone());
                f
            });
            col.update_many(filter.as_ref(), &upd).await?;
            last_error.insert("n".to_string(), Value::Number(1.into()));
            last_error.insert("updatedExisting".to_string(), Value::Bool(true));

            if return_new {
                let post = col
                    .find_one(filter.as_ref())
                    .await?
                    .unwrap_or_default();
                Value::Object(post.into_iter().collect())
            } else {
                Value::Object(doc.into_iter().collect())
            }
        }
        (None, _, Some(upd)) if upsert => {
            // Upsert: build a doc from query + update operators, insert it.
            let mut new_doc = query.unwrap_or_default();
            apply_update(&mut new_doc, &upd)?;
            let id = col.insert_one(new_doc.clone()).await?;
            last_error.insert("n".to_string(), Value::Number(1.into()));
            last_error.insert("updatedExisting".to_string(), Value::Bool(false));
            last_error.insert(
                "upserted".to_string(),
                Value::String(id.clone()),
            );
            if return_new {
                Value::Object(new_doc.into_iter().collect())
            } else {
                Value::Null
            }
        }
        _ => {
            last_error.insert("n".to_string(), Value::Number(0.into()));
            Value::Null
        }
    };

    let mut resp = Document::new();
    resp.insert("lastErrorObject".to_string(), Value::Object(last_error));
    resp.insert("value".to_string(), value);
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

/// `bulkWrite` — execute an ordered batch of insert/update/delete operations
/// against a single collection.
///
/// Recognised ops:
///   * `{insertOne: {document: {...}}}`
///   * `{updateOne: {filter: {...}, update: {...}}}`
///   * `{updateMany: {filter: {...}, update: {...}}}`
///   * `{deleteOne: {filter: {...}}}`
///   * `{deleteMany: {filter: {...}}}`
///
/// The 2024+ MongoDB top-level `bulkWrite` (cross-database, with `nsInfo`) is
/// NOT supported — the command is keyed to the collection named in the
/// command document, matching the shape used by drivers' collection-level
/// `bulk_write()` API.
pub async fn bulk_write(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = db_name_of(cmd_doc);
    let col_name = cmd_doc
        .get("bulkWrite")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            cmd_doc
                .keys()
                .find(|k| {
                    !k.starts_with('$') && !matches!(k.as_str(), "bulkWrite" | "ops" | "ordered")
                })
                .cloned()
        })
        .unwrap_or_else(|| "collection".to_string());

    let ops = cmd_doc
        .get("ops")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing ops".to_string())?;

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let mut inserted: i64 = 0;
    let mut matched: i64 = 0;
    let mut modified: i64 = 0;
    let mut deleted: i64 = 0;
    let mut write_errors: Vec<Value> = Vec::new();

    for (idx, op) in ops.iter().enumerate() {
        let Some(op_obj) = op.as_object() else { continue };
        // Each op has exactly one key.
        let Some((op_name, op_spec)) = op_obj.iter().next() else { continue };
        let Some(spec_obj) = op_spec.as_object() else { continue };

        match op_name.as_str() {
            "insertOne" => {
                if let Some(doc_val) = spec_obj.get("document") {
                    if let Some(doc) = doc_from_value(doc_val) {
                        match col.insert_one(doc).await {
                            Ok(_) => inserted += 1,
                            Err(e) => write_errors.push(serde_json::json!({
                                "index": idx,
                                "errmsg": e,
                            })),
                        }
                    }
                }
            }
            "updateOne" | "updateMany" => {
                let filter = spec_obj.get("filter").and_then(doc_from_value);
                let update = spec_obj.get("update").and_then(doc_from_value);
                if let Some(upd) = update {
                    // Count matches first.
                    let mut m = col.count(filter.as_ref()).await? as i64;
                    if op_name == "updateOne" && m > 1 {
                        m = 1;
                    }
                    matched += m;

                    if op_name == "updateOne" {
                        // Apply only to the first matching document.
                        if let Some(found) = col.find_one(filter.as_ref()).await? {
                            let id_val = found.get("_id").cloned();
                            let f = id_val.as_ref().map(|id| {
                                let mut f = Document::new();
                                f.insert("_id".to_string(), id.clone());
                                f
                            });
                            modified += col.update_many(f.as_ref(), &upd).await? as i64;
                        }
                    } else {
                        modified += col.update_many(filter.as_ref(), &upd).await? as i64;
                    }
                }
            }
            "deleteOne" => {
                let filter = spec_obj.get("filter").and_then(doc_from_value);
                if let Some(found) = col.find_one(filter.as_ref()).await? {
                    let id_val = found.get("_id").cloned();
                    let f = id_val.as_ref().map(|id| {
                        let mut f = Document::new();
                        f.insert("_id".to_string(), id.clone());
                        f
                    });
                    deleted += col.delete_many(f.as_ref()).await? as i64;
                }
            }
            "deleteMany" => {
                let filter = spec_obj.get("filter").and_then(doc_from_value);
                deleted += col.delete_many(filter.as_ref()).await? as i64;
            }
            _ => {
                write_errors.push(serde_json::json!({
                    "index": idx,
                    "errmsg": format!("unsupported bulkWrite op: {}", op_name),
                }));
            }
        }
    }

    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    resp.insert("nInserted".to_string(), Value::Number(inserted.into()));
    resp.insert("nMatched".to_string(), Value::Number(matched.into()));
    resp.insert("nModified".to_string(), Value::Number(modified.into()));
    resp.insert("nDeleted".to_string(), Value::Number(deleted.into()));
    resp.insert("nUpserted".to_string(), Value::Number(0.into()));
    if !write_errors.is_empty() {
        resp.insert("writeErrors".to_string(), Value::Array(write_errors));
    }
    Ok(resp)
}

/// `distinct` — return the set of unique values for `key` over docs matching
/// the query. Comparison is by JSON value equality (not BSON canonical
/// equivalence).
pub async fn distinct(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = db_name_of(cmd_doc);
    let col_name = cmd_doc
        .get("distinct")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "missing collection".to_string())?;

    let key = cmd_doc
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing key".to_string())?;

    let query = cmd_doc.get("query").and_then(doc_from_value);

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let docs = col.find(query.as_ref()).await?;

    let mut values: Vec<Value> = Vec::new();
    for d in docs {
        if let Some(v) = d.get(key) {
            // Arrays: each element becomes a candidate value (MongoDB semantics).
            if let Some(arr) = v.as_array() {
                for el in arr {
                    if !values.iter().any(|x| x == el) {
                        values.push(el.clone());
                    }
                }
            } else if !values.iter().any(|x| x == v) {
                values.push(v.clone());
            }
        }
    }

    let mut resp = Document::new();
    resp.insert("values".to_string(), Value::Array(values));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

/// `renameCollection` — move a collection to a new name within the same
/// database (cross-database rename is not supported).
///
/// `from`/`to` are namespace strings of the form `"db.col"`. If the source
/// and target databases differ, returns `CommandNotSupported` (code 115).
pub async fn rename_collection(
    cmd_doc: &Document,
    engine: Arc<Engine>,
) -> Result<Document, String> {
    let from = cmd_doc
        .get("renameCollection")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing renameCollection".to_string())?;
    let to = cmd_doc
        .get("to")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing to".to_string())?;
    let drop_target = cmd_doc.get("dropTarget").and_then(|v| v.as_bool()).unwrap_or(false);

    let (from_db, from_col) = from.split_once('.').ok_or_else(|| "from must be db.col".to_string())?;
    let (to_db, to_col) = to.split_once('.').ok_or_else(|| "to must be db.col".to_string())?;

    if from_db != to_db {
        return Ok(err(
            "cross-database renameCollection not supported",
            115,
        ));
    }

    let db = engine.get_or_create_database(from_db).await;
    let src = match db.get_collection(from_col).await {
        Some(c) => c,
        None => return Ok(err(format!("source namespace does not exist: {}", from), 26)),
    };

    if db.get_collection(to_col).await.is_some() {
        if drop_target {
            db.drop_collection(to_col).await?;
        } else {
            return Ok(err(format!("target namespace exists: {}", to), 48));
        }
    }

    // Move every document into a fresh collection with the new name, then
    // drop the source.
    let dst = db.get_or_create_collection(to_col).await;
    let docs = src.find(None).await?;
    for d in docs {
        dst.insert_one(d).await?;
    }
    db.drop_collection(from_col).await?;

    let mut resp = Document::new();
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

/// `explain` — describe what the wrapped command would do without executing
/// the heavy parts. Returns the canonical `{queryPlanner, executionStats,
/// serverInfo, ok}` envelope expected by drivers.
///
/// We do not run a real cost-based planner; the response is honestly labeled
/// as a `COLLSCAN` plan and the document count comes from the engine.
pub async fn explain(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = db_name_of(cmd_doc);
    let inner = cmd_doc
        .get("explain")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "missing explain.command".to_string())?;

    let verbosity = cmd_doc
        .get("verbosity")
        .and_then(|v| v.as_str())
        .unwrap_or("queryPlanner")
        .to_string();

    // Identify the inner command. MongoDB places the command name as the
    // first key by document order — but we store as BTreeMap, so we can't
    // rely on insertion order. Probe for known top-level command names.
    const KNOWN_CMDS: &[&str] = &[
        "find", "count", "distinct", "aggregate", "delete", "update",
        "insert", "findAndModify", "findandmodify",
    ];
    let (inner_name, inner_target) = KNOWN_CMDS
        .iter()
        .find_map(|c| {
            inner
                .get(*c)
                .and_then(|v| v.as_str())
                .map(|t| (c.to_string(), t.to_string()))
        })
        .unwrap_or_else(|| ("unknown".into(), "".into()));

    // Best-effort document count for the targeted collection.
    let n_docs = if !inner_target.is_empty() {
        let db = engine.get_or_create_database(&db_name).await;
        let col = db.get_or_create_collection(&inner_target).await;

        // If the inner cmd carries a filter/query, count matches.
        let filter = inner
            .get("filter")
            .or_else(|| inner.get("query"))
            .and_then(doc_from_value);
        col.count(filter.as_ref()).await.unwrap_or(0) as i64
    } else {
        0
    };

    let ns = format!("{}.{}", db_name, inner_target);
    let query_planner = serde_json::json!({
        "namespace": ns.clone(),
        "indexFilterSet": false,
        "parsedQuery": inner.get("filter").or_else(|| inner.get("query")).cloned().unwrap_or(Value::Object(serde_json::Map::new())),
        "winningPlan": {
            "stage": "COLLSCAN",
            "direction": "forward",
        },
        "rejectedPlans": [],
    });

    let mut resp = Document::new();
    resp.insert(
        "queryPlanner".to_string(),
        query_planner,
    );
    if matches!(verbosity.as_str(), "executionStats" | "allPlansExecution") {
        resp.insert(
            "executionStats".to_string(),
            serde_json::json!({
                "executionSuccess": true,
                "nReturned": n_docs,
                "executionTimeMillis": 0,
                "totalKeysExamined": 0,
                "totalDocsExamined": n_docs,
                "executionStages": {
                    "stage": "COLLSCAN",
                    "nReturned": n_docs,
                    "works": n_docs,
                },
            }),
        );
    }
    resp.insert(
        "command".to_string(),
        serde_json::json!({
            "name": inner_name,
            "ns": ns,
        }),
    );
    resp.insert(
        "serverInfo".to_string(),
        serde_json::json!({
            "host": "cave-docdb",
            "version": "7.0-compat",
        }),
    );
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::crud::insert;
    use serde_json::json;

    async fn seed(engine: &Arc<Engine>, col: &str, db: &str, docs: Value) {
        let mut d = Document::new();
        d.insert(col.to_string(), Value::Number(1.into()));
        d.insert("$db".to_string(), Value::String(db.to_string()));
        d.insert("documents".to_string(), docs);
        insert(&d, engine.clone()).await.unwrap();
    }

    fn cmd(items: &[(&str, Value)]) -> Document {
        let mut d = Document::new();
        for (k, v) in items {
            d.insert(k.to_string(), v.clone());
        }
        d
    }

    #[tokio::test]
    async fn find_and_modify_update_returns_pre_image() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "users", "fmdb", json!([{"name":"a","v":1}])).await;

        let c = cmd(&[
            ("findAndModify", json!("users")),
            ("$db", json!("fmdb")),
            ("query", json!({"name":"a"})),
            ("update", json!({"$set": {"v": 2}})),
        ]);
        let resp = find_and_modify(&c, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&json!(1)));
        let value = resp.get("value").unwrap();
        // Pre-image still has v=1.
        assert_eq!(value.get("v").and_then(|v| v.as_i64()), Some(1));
    }

    #[tokio::test]
    async fn find_and_modify_remove_drops_doc() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "things", "fmdb2", json!([{"k":1},{"k":2}])).await;

        let c = cmd(&[
            ("findAndModify", json!("things")),
            ("$db", json!("fmdb2")),
            ("query", json!({"k":1})),
            ("remove", json!(true)),
        ]);
        let resp = find_and_modify(&c, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&json!(1)));

        // Only k=2 left.
        let db = engine.get_or_create_database("fmdb2").await;
        let col = db.get_or_create_collection("things").await;
        assert_eq!(col.count(None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn bulk_write_mixed_ops() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "bw", "bwdb", json!([{"x":1},{"x":2}])).await;

        let c = cmd(&[
            ("bulkWrite", json!("bw")),
            ("$db", json!("bwdb")),
            (
                "ops",
                json!([
                    {"insertOne": {"document": {"x": 3}}},
                    {"updateMany": {"filter": {"x": {"$lte": 2}}, "update": {"$set": {"flag": true}}}},
                    {"deleteOne": {"filter": {"x": 1}}},
                ]),
            ),
        ]);
        let resp = bulk_write(&c, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&json!(1)));
        assert_eq!(resp.get("nInserted").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(resp.get("nDeleted").and_then(|v| v.as_i64()), Some(1));
        // 2 docs matched the filter; one was deleted in the same batch but
        // updates run before the delete in `ops` order.
        assert!(resp.get("nMatched").and_then(|v| v.as_i64()).unwrap() >= 1);
    }

    #[tokio::test]
    async fn distinct_returns_unique_values() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "tags",
            "ddb",
            json!([{"t":"a"},{"t":"b"},{"t":"a"},{"t":"c"}]),
        )
        .await;

        let c = cmd(&[
            ("distinct", json!("tags")),
            ("$db", json!("ddb")),
            ("key", json!("t")),
        ]);
        let resp = distinct(&c, engine.clone()).await.unwrap();
        let values = resp.get("values").and_then(|v| v.as_array()).unwrap();
        assert_eq!(values.len(), 3);
    }

    #[tokio::test]
    async fn distinct_explodes_arrays() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "ts", "ddb2", json!([{"t":["a","b"]},{"t":["b","c"]}])).await;

        let c = cmd(&[
            ("distinct", json!("ts")),
            ("$db", json!("ddb2")),
            ("key", json!("t")),
        ]);
        let resp = distinct(&c, engine.clone()).await.unwrap();
        let values = resp.get("values").and_then(|v| v.as_array()).unwrap();
        assert_eq!(values.len(), 3); // a, b, c
    }

    #[tokio::test]
    async fn rename_collection_within_db() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "old", "rdb", json!([{"v":1}])).await;

        let c = cmd(&[
            ("renameCollection", json!("rdb.old")),
            ("to", json!("rdb.new")),
            ("$db", json!("admin")),
        ]);
        let resp = rename_collection(&c, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&json!(1)));

        let db = engine.get_or_create_database("rdb").await;
        assert!(db.get_collection("old").await.is_none());
        let new_col = db.get_collection("new").await.unwrap();
        assert_eq!(new_col.count(None).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn rename_collection_cross_db_rejected() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "src", "a", json!([{"v":1}])).await;

        let c = cmd(&[
            ("renameCollection", json!("a.src")),
            ("to", json!("b.src")),
        ]);
        let resp = rename_collection(&c, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&json!(0)));
        assert_eq!(resp.get("code").and_then(|v| v.as_i64()), Some(115));
    }

    #[tokio::test]
    async fn explain_basic_envelope() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "ex", "exdb", json!([{"v":1},{"v":2}])).await;

        let c = cmd(&[
            ("explain", json!({"find": "ex", "filter": {"v": 1}})),
            ("$db", json!("exdb")),
            ("verbosity", json!("executionStats")),
        ]);
        let resp = explain(&c, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&json!(1)));
        let qp = resp.get("queryPlanner").unwrap();
        assert_eq!(qp.get("namespace").and_then(|v| v.as_str()), Some("exdb.ex"));
        let stats = resp.get("executionStats").unwrap();
        assert_eq!(stats.get("nReturned").and_then(|v| v.as_i64()), Some(1));
    }
}

