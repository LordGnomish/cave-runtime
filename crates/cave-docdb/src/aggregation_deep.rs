// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Aggregation pipeline — deep stages.
//!
//! Parity with `src/mongo/db/pipeline/document_source_*.cpp`
//! (MongoDB r7.0.0 spec) and the FerretDB Go translator's
//! `internal/handlers/pg/aggregation/*` equivalents.
//!
//! Stages implemented here:
//!
//! * `$lookup`  — left-outer join against another collection
//! * `$facet`   — fan-out pipeline; each branch consumes the same input
//!                and emits to a named array
//! * `$bucket`  — group input documents into buckets by ascending
//!                boundaries with a default catch-all

use crate::bson::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LookupStage {
    pub from: String,
    pub local_field: String,
    pub foreign_field: String,
    pub as_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FacetStage {
    pub branches: std::collections::BTreeMap<String, Vec<DeepStage>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BucketStage {
    pub group_by: String,
    pub boundaries: Vec<f64>,
    pub default: Option<String>,
    pub output: std::collections::BTreeMap<String, BucketAccumulator>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", content = "field")]
pub enum BucketAccumulator {
    Count,
    Sum(String),
    Avg(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "stage")]
pub enum DeepStage {
    Lookup(LookupStage),
    Facet(FacetStage),
    Bucket(BucketStage),
    Limit(usize),
}

#[derive(Debug, Default, Clone)]
pub struct Collections {
    pub inner: std::collections::BTreeMap<String, Vec<Document>>,
}

impl Collections {
    pub fn insert(&mut self, name: impl Into<String>, docs: Vec<Document>) {
        self.inner.insert(name.into(), docs);
    }

    pub fn get(&self, name: &str) -> &[Document] {
        self.inner
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// Execute a sequence of deep stages against the input. The first
/// argument is the input cursor; the second is the catalog of
/// collections used by `$lookup` (and any nested pipelines).
pub fn run(input: Vec<Document>, stages: &[DeepStage], catalog: &Collections) -> Vec<Document> {
    let mut current = input;
    for st in stages {
        current = apply_stage(current, st, catalog);
    }
    current
}

fn apply_stage(
    input: Vec<Document>,
    stage: &DeepStage,
    catalog: &Collections,
) -> Vec<Document> {
    match stage {
        DeepStage::Lookup(l) => apply_lookup(input, l, catalog),
        DeepStage::Facet(f) => apply_facet(input, f, catalog),
        DeepStage::Bucket(b) => apply_bucket(input, b),
        DeepStage::Limit(n) => input.into_iter().take(*n).collect(),
    }
}

fn apply_lookup(input: Vec<Document>, l: &LookupStage, catalog: &Collections) -> Vec<Document> {
    let foreign = catalog.get(&l.from);
    let mut out = Vec::with_capacity(input.len());
    for mut doc in input {
        let lv = doc.get(&l.local_field).cloned();
        let matches: Vec<Value> = foreign
            .iter()
            .filter(|fdoc| match (&lv, fdoc.get(&l.foreign_field)) {
                (Some(a), Some(b)) => values_eq(a, b),
                _ => false,
            })
            .map(|fdoc| Value::Object(doc_to_json(fdoc)))
            .collect();
        doc.insert(l.as_field.clone(), Value::Array(matches));
        out.push(doc);
    }
    out
}

fn apply_facet(input: Vec<Document>, f: &FacetStage, catalog: &Collections) -> Vec<Document> {
    let mut result_doc = Document::new();
    for (name, sub) in &f.branches {
        let sub_out = run(input.clone(), sub, catalog);
        let arr: Vec<Value> = sub_out
            .into_iter()
            .map(|d| Value::Object(doc_to_json(&d)))
            .collect();
        result_doc.insert(name.clone(), Value::Array(arr));
    }
    vec![result_doc]
}

fn apply_bucket(input: Vec<Document>, b: &BucketStage) -> Vec<Document> {
    let mut buckets: Vec<(String, Vec<Document>)> = b
        .boundaries
        .windows(2)
        .map(|w| (format!("[{},{})", w[0], w[1]), Vec::new()))
        .collect();
    let mut default_bucket = (
        b.default.clone().unwrap_or_else(|| "other".into()),
        Vec::new(),
    );
    for doc in input {
        let val = doc.get(&b.group_by).and_then(value_to_f64);
        let mut placed = false;
        if let Some(v) = val {
            for (i, w) in b.boundaries.windows(2).enumerate() {
                if v >= w[0] && v < w[1] {
                    buckets[i].1.push(doc.clone());
                    placed = true;
                    break;
                }
            }
        }
        if !placed {
            default_bucket.1.push(doc);
        }
    }
    let mut out = Vec::new();
    let mut emit = |label: String, docs: Vec<Document>| {
        if docs.is_empty() {
            return;
        }
        let mut acc_doc = Document::new();
        acc_doc.insert("_id".into(), Value::String(label));
        for (name, acc) in &b.output {
            acc_doc.insert(name.clone(), accumulate(acc, &docs));
        }
        out.push(acc_doc);
    };
    for (label, docs) in buckets.into_iter() {
        emit(label, docs);
    }
    emit(default_bucket.0, default_bucket.1);
    out
}

fn accumulate(acc: &BucketAccumulator, docs: &[Document]) -> Value {
    match acc {
        BucketAccumulator::Count => Value::from(docs.len() as i64),
        BucketAccumulator::Sum(field) => {
            let s: f64 = docs.iter().filter_map(|d| d.get(field).and_then(value_to_f64)).sum();
            if (s.fract()).abs() < f64::EPSILON {
                Value::from(s as i64)
            } else {
                Value::from(s)
            }
        }
        BucketAccumulator::Avg(field) => {
            let vals: Vec<f64> = docs
                .iter()
                .filter_map(|d| d.get(field).and_then(value_to_f64))
                .collect();
            if vals.is_empty() {
                Value::Null
            } else {
                Value::from(vals.iter().sum::<f64>() / vals.len() as f64)
            }
        }
    }
}

fn doc_to_json(d: &Document) -> serde_json::Map<String, Value> {
    d.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn values_eq(a: &Value, b: &Value) -> bool {
    a == b
}

fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(pairs: &[(&str, Value)]) -> Document {
        let mut d = Document::new();
        for (k, v) in pairs {
            d.insert((*k).to_string(), v.clone());
        }
        d
    }

    #[test]
    fn lookup_left_outer_joins() {
        let mut cat = Collections::default();
        cat.insert(
            "orders",
            vec![
                doc(&[("_id", json!(1)), ("user_id", json!(42))]),
                doc(&[("_id", json!(2)), ("user_id", json!(99))]),
                doc(&[("_id", json!(3)), ("user_id", json!(42))]),
            ],
        );
        let input = vec![
            doc(&[("_id", json!(42)), ("name", json!("alice"))]),
            doc(&[("_id", json!(7)), ("name", json!("nobody"))]),
        ];
        let out = run(
            input,
            &[DeepStage::Lookup(LookupStage {
                from: "orders".into(),
                local_field: "_id".into(),
                foreign_field: "user_id".into(),
                as_field: "orders".into(),
            })],
            &cat,
        );
        let alice = &out[0];
        assert!(alice.contains_key("orders"));
        let orders = alice.get("orders").unwrap().as_array().unwrap();
        assert_eq!(orders.len(), 2);
        let nobody = &out[1];
        let empty = nobody.get("orders").unwrap().as_array().unwrap();
        assert!(empty.is_empty(), "left-outer must emit empty array, not drop the row");
    }

    #[test]
    fn facet_runs_each_branch_in_parallel() {
        let input = vec![
            doc(&[("price", json!(10))]),
            doc(&[("price", json!(20))]),
            doc(&[("price", json!(30))]),
        ];
        let mut branches: std::collections::BTreeMap<String, Vec<DeepStage>> =
            std::collections::BTreeMap::new();
        branches.insert("first_two".into(), vec![DeepStage::Limit(2)]);
        branches.insert("first_one".into(), vec![DeepStage::Limit(1)]);
        let out = run(
            input,
            &[DeepStage::Facet(FacetStage { branches })],
            &Collections::default(),
        );
        assert_eq!(out.len(), 1);
        let first_two = out[0].get("first_two").unwrap().as_array().unwrap();
        let first_one = out[0].get("first_one").unwrap().as_array().unwrap();
        assert_eq!(first_two.len(), 2);
        assert_eq!(first_one.len(), 1);
    }

    #[test]
    fn bucket_groups_by_boundaries() {
        let input = vec![
            doc(&[("price", json!(5))]),
            doc(&[("price", json!(15))]),
            doc(&[("price", json!(25))]),
            doc(&[("price", json!(150))]),
        ];
        let mut output: std::collections::BTreeMap<String, BucketAccumulator> =
            std::collections::BTreeMap::new();
        output.insert("count".into(), BucketAccumulator::Count);
        let out = run(
            input,
            &[DeepStage::Bucket(BucketStage {
                group_by: "price".into(),
                boundaries: vec![0.0, 10.0, 20.0, 100.0],
                default: Some("expensive".into()),
                output,
            })],
            &Collections::default(),
        );
        // 4 buckets: [0,10) -> 5, [10,20) -> 15, [20,100) -> 25, default -> 150
        assert_eq!(out.len(), 4);
        let labels: Vec<String> = out
            .iter()
            .map(|d| d.get("_id").and_then(|v| v.as_str()).unwrap().to_string())
            .collect();
        assert!(labels.contains(&"[0,10)".to_string()));
        assert!(labels.contains(&"expensive".to_string()));
    }

    #[test]
    fn bucket_sum_accumulator() {
        let input = vec![
            doc(&[("price", json!(5)), ("qty", json!(2))]),
            doc(&[("price", json!(7)), ("qty", json!(3))]),
        ];
        let mut output = std::collections::BTreeMap::new();
        output.insert("total_qty".into(), BucketAccumulator::Sum("qty".into()));
        let out = run(
            input,
            &[DeepStage::Bucket(BucketStage {
                group_by: "price".into(),
                boundaries: vec![0.0, 100.0],
                default: None,
                output,
            })],
            &Collections::default(),
        );
        assert_eq!(out[0].get("total_qty").unwrap().as_i64(), Some(5));
    }

    #[test]
    fn bucket_avg_accumulator() {
        let input = vec![
            doc(&[("price", json!(5)), ("rating", json!(4.0))]),
            doc(&[("price", json!(6)), ("rating", json!(5.0))]),
        ];
        let mut output = std::collections::BTreeMap::new();
        output.insert("avg_rating".into(), BucketAccumulator::Avg("rating".into()));
        let out = run(
            input,
            &[DeepStage::Bucket(BucketStage {
                group_by: "price".into(),
                boundaries: vec![0.0, 100.0],
                default: None,
                output,
            })],
            &Collections::default(),
        );
        assert_eq!(out[0].get("avg_rating").unwrap().as_f64(), Some(4.5));
    }

    #[test]
    fn lookup_with_no_match_emits_empty_array() {
        let cat = Collections::default();
        let input = vec![doc(&[("k", json!(1))])];
        let out = run(
            input,
            &[DeepStage::Lookup(LookupStage {
                from: "missing".into(),
                local_field: "k".into(),
                foreign_field: "k".into(),
                as_field: "joined".into(),
            })],
            &cat,
        );
        assert_eq!(
            out[0].get("joined").unwrap().as_array().unwrap().len(),
            0
        );
    }

    #[test]
    fn limit_stage_truncates_input() {
        let input = vec![
            doc(&[("n", json!(1))]),
            doc(&[("n", json!(2))]),
            doc(&[("n", json!(3))]),
        ];
        let out = run(input, &[DeepStage::Limit(2)], &Collections::default());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = run(
            Vec::new(),
            &[DeepStage::Limit(10)],
            &Collections::default(),
        );
        assert!(out.is_empty());
    }
}
