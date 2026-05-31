// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
                .find(|k| {
                    !k.starts_with('$')
                        && !matches!(k.as_str(), "aggregate" | "pipeline" | "cursor")
                })
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
                                    let dir = if let Some(d) = direction.as_i64() {
                                        d
                                    } else {
                                        1
                                    };
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
                    "$unwind" => unwind_stage(results, stage_spec),
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
        .map(|doc| Value::Object(doc.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
        .collect();
    cursor.insert("firstBatch".to_string(), Value::Array(first_batch));

    let mut resp = Document::new();
    resp.insert("cursor".to_string(), Value::Object(cursor));
    resp.insert("ok".to_string(), Value::Number(1.into()));
    Ok(resp)
}

/// `$unwind` — deconstruct an array field, emitting one document per element.
///
/// Accepts either the short form (`"$field"`) or the object form
/// (`{ path: "$field", preserveNullAndEmptyArrays: bool, includeArrayIndex: "name" }`),
/// matching MongoDB r7.0 / FerretDB semantics:
///   * array values fan out, one output document per element;
///   * a non-array, non-null scalar yields exactly one document, unchanged;
///   * missing field, JSON null, or empty array drop the document unless
///     `preserveNullAndEmptyArrays` is set, in which case the field is removed
///     (or left null) and the document passes through once;
///   * `includeArrayIndex` records the zero-based element offset (null for the
///     preserved non-array path).
fn unwind_stage(input: Vec<Document>, spec: &Value) -> Vec<Document> {
    let (path_spec, preserve, include_index) = match spec {
        Value::String(s) => (s.as_str(), false, None),
        Value::Object(o) => (
            o.get("path").and_then(|v| v.as_str()).unwrap_or(""),
            o.get("preserveNullAndEmptyArrays")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            o.get("includeArrayIndex")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        ),
        _ => return input,
    };
    let field = path_spec.trim_start_matches('$');
    if field.is_empty() {
        return input;
    }

    let mut out = Vec::new();
    for doc in input {
        match doc.get(field) {
            Some(Value::Array(elems)) if !elems.is_empty() => {
                for (i, elem) in elems.iter().enumerate() {
                    let mut d = doc.clone();
                    d.insert(field.to_string(), elem.clone());
                    if let Some(idx_name) = &include_index {
                        d.insert(idx_name.clone(), Value::Number((i as i64).into()));
                    }
                    out.push(d);
                }
            }
            // Empty array, JSON null, or missing field.
            Some(Value::Array(_)) | Some(Value::Null) | None => {
                if preserve {
                    let mut d = doc.clone();
                    d.remove(field);
                    if let Some(idx_name) = &include_index {
                        d.insert(idx_name.clone(), Value::Null);
                    }
                    out.push(d);
                }
            }
            // Non-array, non-null scalar: emit unchanged (index is null).
            Some(_) => {
                let mut d = doc.clone();
                if let Some(idx_name) = &include_index {
                    d.insert(idx_name.clone(), Value::Null);
                }
                out.push(d);
            }
        }
    }
    out
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
                                result.insert(
                                    field.clone(),
                                    Value::Number((docs.len() as i64).into()),
                                );
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
        seed(
            &engine,
            "scores",
            "agg_db",
            json!([{"score": 80}, {"score": 90}, {"score": 70}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("scores".to_string()));
        cmd.insert("$db".to_string(), Value::String("agg_db".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$match": {"score": {"$gte": 80}}}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let cursor = resp.get("cursor").and_then(|v| v.as_object()).unwrap();
        let batch = cursor.get("firstBatch").and_then(|v| v.as_array()).unwrap();
        assert_eq!(batch.len(), 2); // scores 80 and 90
    }

    #[tokio::test]
    async fn test_aggregate_limit_skip() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "nums",
            "agg_db2",
            json!([{"n": 1}, {"n": 2}, {"n": 3}, {"n": 4}]),
        )
        .await;

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
        )
        .await;

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

    fn batch_of(resp: &Document) -> Vec<Value> {
        resp.get("cursor")
            .and_then(|v| v.as_object())
            .and_then(|c| c.get("firstBatch"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap()
    }

    #[tokio::test]
    async fn test_unwind_expands_array_one_doc_per_element() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "items",
            "uw_db1",
            json!([{"_id": 1, "tags": ["a", "b", "c"]}, {"_id": 2, "tags": ["x"]}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("items".to_string()));
        cmd.insert("$db".to_string(), Value::String("uw_db1".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$unwind": "$tags"}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        // 3 elements from doc 1 + 1 element from doc 2 = 4 docs
        assert_eq!(batch.len(), 4);
        // Each emitted doc has the array field replaced by a single scalar element.
        let tags: Vec<&str> = batch
            .iter()
            .filter_map(|d| d.get("tags").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(tags, vec!["a", "b", "c", "x"]);
    }

    #[tokio::test]
    async fn test_unwind_drops_missing_and_empty_by_default() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "items",
            "uw_db2",
            json!([
                {"_id": 1, "tags": ["a", "b"]},
                {"_id": 2, "tags": []},
                {"_id": 3},
                {"_id": 4, "tags": Value::Null}
            ]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("items".to_string()));
        cmd.insert("$db".to_string(), Value::String("uw_db2".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$unwind": "$tags"}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        // Only doc 1's two elements survive; empty/missing/null are dropped.
        assert_eq!(batch.len(), 2);
    }

    #[tokio::test]
    async fn test_unwind_preserve_null_and_empty_arrays() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "items",
            "uw_db3",
            json!([
                {"_id": 1, "tags": ["a"]},
                {"_id": 2, "tags": []},
                {"_id": 3}
            ]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("items".to_string()));
        cmd.insert("$db".to_string(), Value::String("uw_db3".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$unwind": {"path": "$tags", "preserveNullAndEmptyArrays": true}}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        // doc1 (1 elem) + doc2 (preserved, no tags) + doc3 (preserved) = 3
        assert_eq!(batch.len(), 3);
    }

    #[tokio::test]
    async fn test_unwind_include_array_index() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "items",
            "uw_db4",
            json!([{"_id": 1, "tags": ["a", "b", "c"]}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("items".to_string()));
        cmd.insert("$db".to_string(), Value::String("uw_db4".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$unwind": {"path": "$tags", "includeArrayIndex": "idx"}}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        assert_eq!(batch.len(), 3);
        let idxs: Vec<i64> = batch
            .iter()
            .filter_map(|d| d.get("idx").and_then(|v| v.as_i64()))
            .collect();
        assert_eq!(idxs, vec![0, 1, 2]);
    }

    fn group_result_for(batch: &[Value], id: &str) -> serde_json::Map<String, Value> {
        batch
            .iter()
            .find(|d| d.get("_id").and_then(|v| v.as_str()) == Some(id))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn test_group_accumulators_avg_min_max() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "sales",
            "acc_db1",
            json!([
                {"dept": "eng", "amount": 100},
                {"dept": "eng", "amount": 300},
                {"dept": "hr", "amount": 50}
            ]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("sales".to_string()));
        cmd.insert("$db".to_string(), Value::String("acc_db1".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$group": {
                "_id": "$dept",
                "avg": {"$avg": "$amount"},
                "min": {"$min": "$amount"},
                "max": {"$max": "$amount"}
            }}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        let eng = group_result_for(&batch, "eng");
        assert_eq!(eng.get("avg").and_then(|v| v.as_f64()), Some(200.0));
        assert_eq!(eng.get("min").and_then(|v| v.as_f64()), Some(100.0));
        assert_eq!(eng.get("max").and_then(|v| v.as_f64()), Some(300.0));
    }

    #[tokio::test]
    async fn test_group_accumulators_first_last_push_addtoset() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "events",
            "acc_db2",
            json!([
                {"g": "a", "v": 1},
                {"g": "a", "v": 2},
                {"g": "a", "v": 2}
            ]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("events".to_string()));
        cmd.insert("$db".to_string(), Value::String("acc_db2".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$group": {
                "_id": "$g",
                "first": {"$first": "$v"},
                "last": {"$last": "$v"},
                "all": {"$push": "$v"},
                "uniq": {"$addToSet": "$v"}
            }}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        let a = group_result_for(&batch, "a");
        assert_eq!(a.get("first").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(a.get("last").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(
            a.get("all").and_then(|v| v.as_array()).map(|v| v.len()),
            Some(3)
        );
        // $addToSet de-dupes -> {1, 2}
        assert_eq!(
            a.get("uniq").and_then(|v| v.as_array()).map(|v| v.len()),
            Some(2)
        );
    }

    #[tokio::test]
    async fn test_group_id_null_aggregates_all() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "nums",
            "acc_db3",
            json!([{"n": 1}, {"n": 2}, {"n": 3}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("nums".to_string()));
        cmd.insert("$db".to_string(), Value::String("acc_db3".to_string()));
        cmd.insert(
            "pipeline".to_string(),
            json!([{"$group": {"_id": Value::Null, "total": {"$sum": "$n"}, "avg": {"$avg": "$n"}}}]),
        );

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].get("total").and_then(|v| v.as_i64()), Some(6));
        assert_eq!(batch[0].get("avg").and_then(|v| v.as_f64()), Some(2.0));
    }

    #[tokio::test]
    async fn test_unwind_scalar_field_emitted_unchanged() {
        let engine = Arc::new(Engine::new());
        seed(
            &engine,
            "items",
            "uw_db5",
            json!([{"_id": 1, "tags": "solo"}]),
        )
        .await;

        let mut cmd = Document::new();
        cmd.insert("aggregate".to_string(), Value::String("items".to_string()));
        cmd.insert("$db".to_string(), Value::String("uw_db5".to_string()));
        cmd.insert("pipeline".to_string(), json!([{"$unwind": "$tags"}]));

        let resp = aggregate(&cmd, engine.clone()).await.unwrap();
        let batch = batch_of(&resp);
        // A non-array, non-null scalar yields exactly one document, unchanged.
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].get("tags").and_then(|v| v.as_str()), Some("solo"));
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
