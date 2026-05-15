// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query engine — mixed datasource queries, caching, query inspector,
//! and all Grafana transformations.

use crate::models::{DataFrame, DataFrameData, DataFrameSchema, FieldSchema, DsQueryRequest, DsQueryResponse, QueryResult};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

// ─── Query cache ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct CacheEntry {
    result: QueryResult,
    inserted_at: Instant,
    ttl: Duration,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

#[derive(Clone, Default)]
pub struct QueryCache {
    inner: Arc<RwLock<HashMap<String, CacheEntry>>>,
}

impl QueryCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<QueryResult> {
        let inner = self.inner.read().ok()?;
        let entry = inner.get(key)?;
        if entry.is_expired() {
            return None;
        }
        Some(entry.result.clone())
    }

    pub fn put(&self, key: String, result: QueryResult, ttl: Duration) {
        let mut inner = self.inner.write().unwrap();
        inner.insert(key, CacheEntry { result, inserted_at: Instant::now(), ttl });
    }

    /// Evict expired entries.
    pub fn evict(&self) {
        let mut inner = self.inner.write().unwrap();
        inner.retain(|_, v| !v.is_expired());
    }
}

/// Build a cache key from datasource uid, query expression, and time range.
pub fn cache_key(ds_uid: &str, expr: &str, from: &str, to: &str) -> String {
    format!("{ds_uid}:{from}:{to}:{expr}")
}

// ─── Query inspector ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryInspectorRequest {
    pub datasource_uid: String,
    pub ref_id: String,
    pub raw_query: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryInspectorResponse {
    pub request: serde_json::Value,
    pub response: serde_json::Value,
    pub execution_time_ms: u64,
    pub cache_hit: bool,
}

// ─── Transformations ─────────────────────────────────────────────────────────

/// Apply a list of Grafana transformations to a set of DataFrames.
pub fn apply_transformations(
    frames: Vec<DataFrame>,
    transformations: &[serde_json::Value],
) -> Vec<DataFrame> {
    let mut current = frames;
    for tx in transformations {
        let id = tx.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let options = tx.get("options").cloned().unwrap_or_default();
        current = apply_single_transformation(current, id, &options);
    }
    current
}

fn apply_single_transformation(
    frames: Vec<DataFrame>,
    id: &str,
    options: &serde_json::Value,
) -> Vec<DataFrame> {
    match id {
        "reduce" => transform_reduce(frames, options),
        "merge" => transform_merge(frames),
        "filterFieldsByName" => transform_filter_fields(frames, options),
        "filterByValue" => transform_filter_by_value(frames, options),
        "organize" => transform_organize(frames, options),
        "calculateField" => transform_calculate_field(frames, options),
        "groupBy" => transform_group_by(frames, options),
        "sortBy" => transform_sort_by(frames, options),
        "renameByRegex" => transform_rename_by_regex(frames, options),
        "concatenate" => transform_concatenate(frames),
        "convertFieldType" => transform_convert_field_type(frames, options),
        "limit" => transform_limit(frames, options),
        "seriesToRows" => transform_series_to_rows(frames),
        "joinByField" => transform_join_by_field(frames, options),
        "labelsToFields" => transform_labels_to_fields(frames, options),
        _ => frames, // Unknown transformation — pass through
    }
}

/// Reduce: collapse each field to a single value using a reducer function.
fn transform_reduce(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let reducers = options.get("reducers")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["last"]);

    frames.into_iter().map(|frame| {
        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        let mut new_fields: Vec<FieldSchema> = vec![
            FieldSchema { name: "Field".into(), field_type: "string".into(), type_info: None, labels: None, config: None },
        ];
        let mut field_names_col: Vec<serde_json::Value> = Vec::new();
        let mut reducer_cols: Vec<Vec<serde_json::Value>> = vec![Vec::new(); reducers.len()];

        for (i, schema) in frame.schema.fields.iter().enumerate() {
            if schema.field_type == "time" {
                continue;
            }
            let vals: Vec<f64> = frame.data.values.get(i)
                .map(|col| col.iter().filter_map(|v| v.as_f64()).collect())
                .unwrap_or_default();

            field_names_col.push(serde_json::Value::String(schema.name.clone()));
            for (j, reducer) in reducers.iter().enumerate() {
                let v = apply_reducer_fn(reducer, &vals);
                reducer_cols[j].push(match v {
                    Some(f) => serde_json::json!(f),
                    None => serde_json::Value::Null,
                });
            }
        }

        for reducer in &reducers {
            new_fields.push(FieldSchema {
                name: reducer.to_string(),
                field_type: "number".into(),
                type_info: None,
                labels: None,
                config: None,
            });
        }

        let mut new_values = vec![field_names_col];
        new_values.extend(reducer_cols);

        DataFrame {
            schema: DataFrameSchema {
                ref_id: frame.schema.ref_id,
                name: frame.schema.name,
                fields: new_fields,
                meta: frame.schema.meta,
            },
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

fn apply_reducer_fn(name: &str, vals: &[f64]) -> Option<f64> {
    crate::alerting::apply_reducer(name, vals)
}

/// Merge: combine all frames into a single frame.
fn transform_merge(frames: Vec<DataFrame>) -> Vec<DataFrame> {
    if frames.is_empty() {
        return vec![];
    }
    // Find union of field names
    let mut field_names: Vec<String> = Vec::new();
    for frame in &frames {
        for field in &frame.schema.fields {
            if !field_names.contains(&field.name) {
                field_names.push(field.name.clone());
            }
        }
    }

    let mut merged_cols: Vec<Vec<serde_json::Value>> = vec![Vec::new(); field_names.len()];

    for frame in &frames {
        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        let field_idx: HashMap<&str, usize> = frame.schema.fields.iter().enumerate()
            .map(|(i, f)| (f.name.as_str(), i))
            .collect();

        for row in 0..row_count {
            for (col_idx, name) in field_names.iter().enumerate() {
                let val = if let Some(&src_idx) = field_idx.get(name.as_str()) {
                    frame.data.values.get(src_idx)
                        .and_then(|col| col.get(row))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null)
                } else {
                    serde_json::Value::Null
                };
                merged_cols[col_idx].push(val);
            }
        }
    }

    let first = &frames[0];
    vec![DataFrame {
        schema: DataFrameSchema {
            ref_id: first.schema.ref_id.clone(),
            name: "merged".into(),
            fields: field_names.iter().map(|n| FieldSchema {
                name: n.clone(),
                field_type: "string".into(),
                type_info: None,
                labels: None,
                config: None,
            }).collect(),
            meta: None,
        },
        data: DataFrameData { values: merged_cols, entities: None },
    }]
}

/// Filter fields by name (include or exclude pattern).
fn transform_filter_fields(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let include: Vec<String> = options.get("include").and_then(|v| v.get("names"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let exclude: Vec<String> = options.get("exclude").and_then(|v| v.get("names"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    frames.into_iter().map(|frame| {
        let keep: Vec<usize> = frame.schema.fields.iter().enumerate()
            .filter(|(_, f)| {
                if !include.is_empty() && !include.contains(&f.name) {
                    return false;
                }
                if exclude.contains(&f.name) {
                    return false;
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        let new_fields: Vec<FieldSchema> = keep.iter().map(|&i| frame.schema.fields[i].clone()).collect();
        let new_values: Vec<Vec<serde_json::Value>> = keep.iter()
            .map(|&i| frame.data.values.get(i).cloned().unwrap_or_default())
            .collect();

        DataFrame {
            schema: DataFrameSchema {
                fields: new_fields,
                ..frame.schema
            },
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

/// Filter rows by value condition.
fn transform_filter_by_value(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    // Simplified: filter rows where the specified field matches condition
    let field_name = options.get("filters").and_then(|f| f.as_array())
        .and_then(|arr| arr.first())
        .and_then(|f| f.get("fieldName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let match_type = options.get("filters").and_then(|f| f.as_array())
        .and_then(|arr| arr.first())
        .and_then(|f| f.get("config"))
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("isNotNull");

    frames.into_iter().map(|frame| {
        let field_idx = frame.schema.fields.iter().position(|f| f.name == field_name);
        let Some(idx) = field_idx else { return frame; };

        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        let keep_rows: Vec<usize> = (0..row_count).filter(|&row| {
            let val = frame.data.values.get(idx).and_then(|col| col.get(row));
            match match_type {
                "isNull" => matches!(val, None | Some(serde_json::Value::Null)),
                "isNotNull" => !matches!(val, None | Some(serde_json::Value::Null)),
                _ => true,
            }
        }).collect();

        let new_values: Vec<Vec<serde_json::Value>> = frame.data.values.iter()
            .map(|col| keep_rows.iter().filter_map(|&r| col.get(r).cloned()).collect())
            .collect();

        DataFrame {
            schema: frame.schema,
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

/// Organize: reorder, rename, and hide fields.
fn transform_organize(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let index_by_name = options.get("indexByName").cloned().unwrap_or_default();
    let rename_by_name = options.get("renameByName").cloned().unwrap_or_default();
    let exclude_by_name = options.get("excludeByName").cloned().unwrap_or_default();

    frames.into_iter().map(|mut frame| {
        // Apply renames
        for field in frame.schema.fields.iter_mut() {
            if let Some(new_name) = rename_by_name.get(&field.name).and_then(|v| v.as_str()) {
                field.name = new_name.to_string();
            }
        }
        // Remove excluded fields
        let keep: Vec<usize> = frame.schema.fields.iter().enumerate()
            .filter(|(_, f)| {
                !exclude_by_name.get(&f.name).and_then(|v| v.as_bool()).unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect();

        let new_fields: Vec<FieldSchema> = keep.iter().map(|&i| frame.schema.fields[i].clone()).collect();
        let new_values: Vec<Vec<serde_json::Value>> = keep.iter()
            .map(|&i| frame.data.values.get(i).cloned().unwrap_or_default())
            .collect();

        DataFrame {
            schema: DataFrameSchema { fields: new_fields, ..frame.schema },
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

/// Calculate field: add a new field computed from existing fields.
fn transform_calculate_field(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let alias = options.get("alias").and_then(|v| v.as_str()).unwrap_or("calculated");
    let mode = options.get("mode").and_then(|v| v.as_str()).unwrap_or("reduceRow");

    frames.into_iter().map(|frame| {
        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        let new_col: Vec<serde_json::Value> = match mode {
            "reduceRow" => {
                let reducer = options.get("reduce").and_then(|r| r.get("reducer"))
                    .and_then(|v| v.as_str()).unwrap_or("sum");
                (0..row_count).map(|row| {
                    let vals: Vec<f64> = frame.schema.fields.iter().enumerate()
                        .filter(|(_, f)| f.field_type == "number")
                        .filter_map(|(i, _)| {
                            frame.data.values.get(i)?.get(row)?.as_f64()
                        })
                        .collect();
                    match apply_reducer_fn(reducer, &vals) {
                        Some(v) => serde_json::json!(v),
                        None => serde_json::Value::Null,
                    }
                }).collect()
            }
            _ => vec![serde_json::Value::Null; row_count],
        };

        let mut new_fields = frame.schema.fields.clone();
        new_fields.push(FieldSchema {
            name: alias.to_string(),
            field_type: "number".into(),
            type_info: None,
            labels: None,
            config: None,
        });

        let mut new_values = frame.data.values.clone();
        new_values.push(new_col);

        DataFrame {
            schema: DataFrameSchema { fields: new_fields, ..frame.schema },
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

/// Group by: aggregate rows by a set of fields.
fn transform_group_by(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    // Simplified passthrough — full impl would aggregate rows
    frames
}

/// Sort by: sort rows by a field.
fn transform_sort_by(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let field_name = options.get("fields").and_then(|f| f.as_array())
        .and_then(|arr| arr.first())
        .and_then(|f| f.get("displayName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let desc = options.get("fields").and_then(|f| f.as_array())
        .and_then(|arr| arr.first())
        .and_then(|f| f.get("desc"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    frames.into_iter().map(|frame| {
        let field_idx = frame.schema.fields.iter().position(|f| f.name == field_name);
        let Some(sort_col) = field_idx else { return frame; };

        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        let mut indices: Vec<usize> = (0..row_count).collect();
        indices.sort_by(|&a, &b| {
            let va = frame.data.values.get(sort_col).and_then(|col| col.get(a));
            let vb = frame.data.values.get(sort_col).and_then(|col| col.get(b));
            let cmp = compare_json_values(va, vb);
            if desc { cmp.reverse() } else { cmp }
        });

        let new_values: Vec<Vec<serde_json::Value>> = frame.data.values.iter()
            .map(|col| indices.iter().filter_map(|&i| col.get(i).cloned()).collect())
            .collect();

        DataFrame {
            schema: frame.schema,
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

fn compare_json_values(a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) -> std::cmp::Ordering {
    match (a, b) {
        (Some(serde_json::Value::Number(x)), Some(serde_json::Value::Number(y))) => {
            x.as_f64().partial_cmp(&y.as_f64()).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Some(serde_json::Value::String(x)), Some(serde_json::Value::String(y))) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

/// Rename by regex.
fn transform_rename_by_regex(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let pattern = options.get("regex").and_then(|v| v.as_str()).unwrap_or("");
    let replacement = options.get("renamePattern").and_then(|v| v.as_str()).unwrap_or("$1");

    let Ok(re) = regex::Regex::new(pattern) else { return frames; };

    frames.into_iter().map(|mut frame| {
        for field in frame.schema.fields.iter_mut() {
            field.name = re.replace_all(&field.name, replacement).into_owned();
        }
        frame
    }).collect()
}

/// Concatenate frames into a single frame (same schema assumed).
fn transform_concatenate(frames: Vec<DataFrame>) -> Vec<DataFrame> {
    transform_merge(frames)
}

/// Convert field type.
fn transform_convert_field_type(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let conversions = options.get("conversions").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    frames.into_iter().map(|mut frame| {
        for conversion in &conversions {
            let field_name = conversion.get("targetField").and_then(|v| v.as_str()).unwrap_or("");
            let dest_type = conversion.get("destinationType").and_then(|v| v.as_str()).unwrap_or("string");
            if let Some(field) = frame.schema.fields.iter_mut().find(|f| f.name == field_name) {
                field.field_type = dest_type.to_string();
            }
        }
        frame
    }).collect()
}

/// Limit: keep only the first N rows.
fn transform_limit(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    let limit = options.get("limitField").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
    frames.into_iter().map(|frame| {
        let new_values: Vec<Vec<serde_json::Value>> = frame.data.values.iter()
            .map(|col| col.iter().take(limit).cloned().collect())
            .collect();
        DataFrame {
            schema: frame.schema,
            data: DataFrameData { values: new_values, entities: None },
        }
    }).collect()
}

/// Series to rows: pivot each series into rows with Time, Metric, Value columns.
fn transform_series_to_rows(frames: Vec<DataFrame>) -> Vec<DataFrame> {
    let mut times: Vec<serde_json::Value> = Vec::new();
    let mut metrics: Vec<serde_json::Value> = Vec::new();
    let mut values: Vec<serde_json::Value> = Vec::new();

    for frame in &frames {
        let time_idx = frame.schema.fields.iter().position(|f| f.field_type == "time");
        let val_idx = frame.schema.fields.iter().position(|f| f.field_type == "number");
        let metric_name = frame.schema.name.clone();

        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        for row in 0..row_count {
            let ts = time_idx.and_then(|i| frame.data.values.get(i)?.get(row).cloned())
                .unwrap_or(serde_json::Value::Null);
            let val = val_idx.and_then(|i| frame.data.values.get(i)?.get(row).cloned())
                .unwrap_or(serde_json::Value::Null);
            times.push(ts);
            metrics.push(serde_json::Value::String(metric_name.clone()));
            values.push(val);
        }
    }

    vec![DataFrame {
        schema: DataFrameSchema {
            ref_id: frames.first().map(|f| f.schema.ref_id.clone()).unwrap_or_default(),
            name: "rows".into(),
            fields: vec![
                FieldSchema { name: "Time".into(), field_type: "time".into(), type_info: None, labels: None, config: None },
                FieldSchema { name: "Metric".into(), field_type: "string".into(), type_info: None, labels: None, config: None },
                FieldSchema { name: "Value".into(), field_type: "number".into(), type_info: None, labels: None, config: None },
            ],
            meta: None,
        },
        data: DataFrameData { values: vec![times, metrics, values], entities: None },
    }]
}

/// Join by field: join frames by a common field (like SQL JOIN).
fn transform_join_by_field(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    // Simplified outer join on time field
    let by_field = options.get("byField").and_then(|v| v.as_str()).unwrap_or("Time");
    transform_merge(frames) // Simplified
}

/// Labels to fields: expand label key-value pairs into separate columns.
fn transform_labels_to_fields(frames: Vec<DataFrame>, options: &serde_json::Value) -> Vec<DataFrame> {
    frames.into_iter().map(|mut frame| {
        let mut extra_fields: Vec<FieldSchema> = Vec::new();
        for field in &frame.schema.fields {
            if let Some(labels) = &field.labels {
                for key in labels.keys() {
                    if !extra_fields.iter().any(|f| &f.name == key) {
                        extra_fields.push(FieldSchema {
                            name: key.clone(),
                            field_type: "string".into(),
                            type_info: None,
                            labels: None,
                            config: None,
                        });
                    }
                }
            }
        }

        let row_count = frame.data.values.first().map(|v| v.len()).unwrap_or(0);
        let source_field = frame.schema.fields.first().cloned();

        for extra in &extra_fields {
            let col: Vec<serde_json::Value> = if let Some(f) = &source_field {
                f.labels.as_ref()
                    .and_then(|lbls| lbls.get(&extra.name))
                    .map(|v| vec![serde_json::Value::String(v.clone()); row_count])
                    .unwrap_or_else(|| vec![serde_json::Value::Null; row_count])
            } else {
                vec![serde_json::Value::Null; row_count]
            };
            frame.data.values.push(col);
            frame.schema.fields.push(extra.clone());
        }
        frame
    }).collect()
}
