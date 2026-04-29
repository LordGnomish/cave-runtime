//! Aggregation pipeline: $match, $project, $limit, $skip, $sort, $group.

use crate::bson::Document;
use crate::engine::Engine;
use crate::query::matches_query;
use serde_json::Value;
use std::sync::Arc;

pub async fn aggregate(cmd_doc: &Document, engine: Arc<Engine>) -> Result<Document, String> {
    let db_name = cmd_doc
        .get("$db")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "test".to_string());

    // Collection name: value of "aggregate" key (standard format) or
    // first non-$ non-parameter key (cave-docdb native format).
    let col_name = cmd_doc
        .get("aggregate")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            cmd_doc
                .keys()
                .find(|k| !k.starts_with('$') && !matches!(k.as_str(), "aggregate" | "pipeline" | "cursor"))
                .cloned()
                .unwrap_or_else(|| "collection".to_string())
        });

    let pipeline = cmd_doc
        .get("pipeline")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing pipeline".to_string())?;

    let db = engine.get_or_create_database(&db_name).await;
    let col = db.get_or_create_collection(&col_name).await;

    let all_docs = col.find(None).await?;
    let mut results = all_docs;

    for stage in pipeline {
        if let Some(stage_obj) = stage.as_object() {
            if let Some((stage_name, stage_spec)) = stage_obj.iter().next() {
                results = match stage_name.as_str() {
                    "$match" => {
                        if let Some(filter_obj) = stage_spec.as_object() {
                            let mut filter = Document::new();
                            for (k, v) in filter_obj {
                                filter.insert(k.clone(), v.clone());
                            }
                            results
                                .into_iter()
                                .filter(|doc| matches_query(doc, &filter))
                                .collect()
                        } else {
                            results
                        }
                    }
                    "$project" => {
                        if let Some(proj_obj) = stage_spec.as_object() {
                            let mut proj = Document::new();
                            for (k, v) in proj_obj {
                                proj.insert(k.clone(), v.clone());
                            }
                            results
                                .into_iter()
                                .map(|doc| {
                                    let mut result = Document::new();
                                    for (key, value) in &proj {
                                        if let Some(num) = value.as_i64() {
                                            if num == 1 {
                                                if let Some(v) = doc.get(key) {
                                                    result.insert(key.clone(), v.clone());
                                                }
                                            }
                                        }
                                    }
                                    result
                                })
                                .collect()
                        } else {
                            results
                        }
                    }
                    "$limit" => {
                        if let Some(limit) = stage_spec.as_i64() {
                            results.into_iter().take(limit as usize).collect()
                        } else {
                            results
                        }
                    }
                    "$skip" => {
                        if let Some(skip) = stage_spec.as_i64() {
                            results.into_iter().skip(skip as usize).collect()
                        } else {
                            results
                        }
                    }
                    "$sort" => {
                        if let Some(sort_obj) = stage_spec.as_object() {
                            results.sort_by(|a, b| {
                                for (key, direction) in sort_obj {
                                    let dir =
                                        if let Some(d) = direction.as_i64() { d } else { 1 };
                                    let av = a.get(key);
                                    let bv = b.get(key);
                                    let cmp = compare_values(av, bv);
                                    if cmp != std::cmp::Ordering::Equal {
                                        return if dir > 0 { cmp } else { cmp.reverse() };
                                    }
                                }
                                std::cmp::Ordering::Equal
                            });
                            results
                        } else {
                            results
                        }
                    }
                    "$group" => {
                        if let Some(group_obj) = stage_spec.as_object() {
                            group_stage(&results, group_obj)
                        } else {
                            results
                        }
                    }
                    "$unwind" => unwind_stage(&results, stage_spec),
                    "$lookup" => {
                        if let Some(spec) = stage_spec.as_object() {
                            lookup_stage(&results, spec, engine.clone(), &db_name).await
                        } else {
                            results
                        }
                    }
                    "$count" => {
                        if let Some(name) = stage_spec.as_str() {
                            let mut d = Document::new();
                            d.insert(name.to_string(), Value::Number((results.len() as i64).into()));
                            vec![d]
                        } else {
                            results
                        }
                    }
                    "$addFields" | "$set" => {
                        if let Some(spec) = stage_spec.as_object() {
                            results
                                .into_iter()
                                .map(|mut d| {
                                    for (k, v) in spec {
                                        d.insert(k.clone(), v.clone());
                                    }
                                    d
                                })
                                .collect()
                        } else {
                            results
                        }
                    }
                    _ => results,
                };
            }
        }
    }

    let mut cursor = serde_json::Map::new();
    cursor.insert("id".to_string(), Value::Number(0.into()));
    cursor.insert(
        "ns".to_string(),
        Value::String(format!("{}.{}", db_name, col_name)),
    );

    let first_batch: Vec<Value> = results
        .iter()
        .map(|doc| {
            Value::Object(
                doc.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            )
        })
        .collect();
    cursor.insert("firstBatch".to_string(), Value::Array(first_batch));

    let mut resp = Document::new();
    resp.insert("cursor".to_string(), Value::Object(cursor));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

/// `$unwind` — explode an array field into one document per element.
///
/// Accepts either the short form `{"$unwind": "$path"}` or the long form
/// `{"$unwind": {"path": "$path", "preserveNullAndEmptyArrays": bool}}`.
fn unwind_stage(results: &[Document], spec: &Value) -> Vec<Document> {
    let (path, preserve) = match spec {
        Value::String(s) => (s.clone(), false),
        Value::Object(o) => {
            let p = o.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let pres = o
                .get("preserveNullAndEmptyArrays")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            (p, pres)
        }
        _ => return results.to_vec(),
    };
    let field_name = path.trim_start_matches('$').to_string();
    if field_name.is_empty() {
        return results.to_vec();
    }

    let mut out = Vec::with_capacity(results.len());
    for d in results {
        match d.get(&field_name) {
            Some(Value::Array(arr)) if !arr.is_empty() => {
                for el in arr {
                    let mut nd = d.clone();
                    nd.insert(field_name.clone(), el.clone());
                    out.push(nd);
                }
            }
            Some(Value::Array(_)) | None | Some(Value::Null) => {
                if preserve {
                    out.push(d.clone());
                }
            }
            Some(other) => {
                // Non-array scalar: treat as a single-element unwind.
                let mut nd = d.clone();
                nd.insert(field_name.clone(), other.clone());
                out.push(nd);
            }
        }
    }
    out
}

/// `$lookup` — left outer join against another collection in the same DB.
///
/// Supports the simple `{from, localField, foreignField, as}` shape. The
/// pipeline-form `$lookup` (with sub-pipeline + `let`) is NOT supported.
async fn lookup_stage(
    results: &[Document],
    spec: &serde_json::Map<String, Value>,
    engine: Arc<Engine>,
    db_name: &str,
) -> Vec<Document> {
    let from = spec.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let local_field = spec.get("localField").and_then(|v| v.as_str()).unwrap_or("");
    let foreign_field = spec.get("foreignField").and_then(|v| v.as_str()).unwrap_or("");
    let as_field = spec.get("as").and_then(|v| v.as_str()).unwrap_or("");
    if from.is_empty() || as_field.is_empty() {
        return results.to_vec();
    }

    let db = engine.get_or_create_database(db_name).await;
    let foreign_col = db.get_or_create_collection(from).await;
    let foreign_docs = foreign_col.find(None).await.unwrap_or_default();

    results
        .iter()
        .map(|d| {
            let local_val = d.get(local_field).cloned();
            let matches: Vec<Value> = foreign_docs
                .iter()
                .filter(|f| match (&local_val, f.get(foreign_field)) {
                    (Some(l), Some(r)) => l == r,
                    _ => false,
                })
                .map(|f| Value::Object(f.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
                .collect();
            let mut nd = d.clone();
            nd.insert(as_field.to_string(), Value::Array(matches));
            nd
        })
        .collect()
}

fn group_stage(results: &[Document], group_spec: &serde_json::Map<String, Value>) -> Vec<Document> {
    use std::collections::HashMap;

    let mut groups: HashMap<String, Vec<Document>> = HashMap::new();

    for doc in results {
        let group_key = group_spec
            .get("_id")
            .and_then(|v| v.as_str())
            .unwrap_or("all");

        let key = if let Some(key_field) = group_key.strip_prefix('$') {
            doc.get(key_field)
                .map(|v| format!("{:?}", v))
                .unwrap_or_else(|| "null".to_string())
        } else {
            "all".to_string()
        };

        groups.entry(key).or_insert_with(Vec::new).push(doc.clone());
    }

    let mut grouped_results = Vec::new();

    for (key, docs) in groups {
        let mut result = Document::new();
        result.insert("_id".to_string(), Value::String(key));

        for (field, spec) in group_spec {
            if field == "_id" {
                continue;
            }

            if let Some(spec_obj) = spec.as_object() {
                if let Some((op, op_field)) = spec_obj.iter().next() {
                    match op.as_str() {
                        "$sum" => {
                            if let Some(field_name) = op_field.as_str() {
                                let field_name = field_name.trim_start_matches('$');
                                let sum: i64 = docs
                                    .iter()
                                    .filter_map(|d| d.get(field_name).and_then(|v| v.as_i64()))
                                    .sum();
                                result.insert(field.clone(), Value::Number(sum.into()));
                            } else if op_field.as_i64() == Some(1) {
                                result.insert(field.clone(), Value::Number((docs.len() as i64).into()));
                            }
                        }
                        "$count" => {
                            result.insert(field.clone(), Value::Number((docs.len() as i64).into()));
                        }
                        _ => {}
                    }
                }
            }
        }

        grouped_results.push(result);
    }

    grouped_results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::crud::insert;
    use serde_json::json;

    async fn seed(engine: &Arc<Engine>, col: &str, db: &str, docs: serde_json::Value) {
        let mut d = Document::new();
        d.insert(col.to_string(), Value::Number(1.into()));
        d.insert("$db".to_string(), Value::String(db.to_string()));
        d.insert("documents".to_string(), docs);
        insert(&d, engine.clone()).await.unwrap();
    }

    #[tokio::test]
    async fn test_aggregate_match_stage() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "scores", "agg_db", json!([{"score": 80}, {"score": 90}, {"score": 70}])).await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("scores".to_string()));
        cmd.insert("$db".to_string(), Value::String("agg_db".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$match": {"score": {"$gte": 80}}}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 2); // scores 80 and 90
    }

    #[tokio::test]
    async fn test_aggregate_limit_skip() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "nums", "agg_db2", json!([{"n": 1}, {"n": 2}, {"n": 3}, {"n": 4}])).await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("nums".to_string()));
        cmd.insert("$db".to_string(), Value::String("agg_db2".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$skip": 1}, {"$limit": 2}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 2);
    }

    #[tokio::test]
    async fn test_aggregate_unwind_explodes_array() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "lists",
            "uw_db",
            json!([{"id": 1, "tags": ["a", "b", "c"]}, {"id": 2, "tags": ["x"]}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("lists".to_string()));
        cmd.insert("$db".to_string(), Value::String("uw_db".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$unwind": "$tags"}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 4); // 3 + 1
    }

    #[tokio::test]
    async fn test_aggregate_lookup_left_outer_join() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "users",
            "lk_db",
            json!([{"_id": "u1", "name": "alice"}, {"_id": "u2", "name": "bob"}]),
        )
        .await;
        seed(
            &engine,
            "orders",
            "lk_db",
            json!([{"uid": "u1", "item": "book"}, {"uid": "u1", "item": "pen"}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("users".to_string()));
        cmd.insert("$db".to_string(), Value::String("lk_db".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{
                "$lookup": {
                    "from": "orders",
                    "localField": "_id",
                    "foreignField": "uid",
                    "as": "orders",
                }
            }]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 2);
        // alice has 2 orders, bob has 0.
        let alice = batch.iter().find(|d| d.get("name").and_then(|v| v.as_str()) == Some("alice")).unwrap();
        assert_eq!(alice.get("orders").and_then(|v| v.as_array()).unwrap().len(), 2);
        let bob = batch.iter().find(|d| d.get("name").and_then(|v| v.as_str()) == Some("bob")).unwrap();
        assert_eq!(bob.get("orders").and_then(|v| v.as_array()).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_aggregate_count_stage() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "items", "cnt_db", json!([{"v":1},{"v":2},{"v":3}])).await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("items".to_string()));
        cmd.insert("$db".to_string(), Value::String("cnt_db".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$count": "total"}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].get("total").and_then(|v| v.as_i64()), Some(3));
    }

    #[tokio::test]
    async fn test_aggregate_addfields_stage() {
        let engine = Arc::new(Engine::new());
        seed(&engine, "p", "af_db", json!([{"x":1}])).await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("p".to_string()));
        cmd.insert("$db".to_string(), Value::String("af_db".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$addFields": {"y": 42}}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch[0].get("y").and_then(|v| v.as_i64()), Some(42));
    }

    #[tokio::test]
    async fn test_aggregate_group_sum() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "sales",
            "agg_db3",
            json!([
                {"dept": "eng", "amount": 100},
                {"dept": "eng", "amount": 200},
                {"dept": "hr", "amount": 50}
            ]),
        ).await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("sales".to_string()));
        cmd.insert("$db".to_string(), Value::String("agg_db3".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$group": {"_id": "$dept", "total": {"$sum": "$amount"}}}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        assert_eq!(resp.get("ok"), Some(&Value::Number(1.into())));
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 2); // two departments
    }
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(av), Some(bv)) => {
            if let (Some(an), Some(bn)) = (av.as_i64(), bv.as_i64()) {
                an.cmp(&bn)
            } else if let (Some(as_), Some(bs)) = (av.as_str(), bv.as_str()) {
                as_.cmp(bs)
            } else {
                std::cmp::Ordering::Equal
            }
        }
    }
}
