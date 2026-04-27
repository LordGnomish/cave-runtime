//! Aggregation pipeline — sans-engine evaluator over `serde_json::Value`.
//!
//! Implements the most-used MongoDB pipeline stages:
//!
//! * `$match`   — filter by simple equality predicates.
//! * `$project` — include / exclude fields by name.
//! * `$group`   — sum / count / first / last / push.
//! * `$sort`    — multi-key ascending / descending.
//! * `$limit`   — keep first N.
//! * `$skip`    — drop first N.
//! * `$unwind`  — explode an array field into one doc per element.
//! * `$lookup`  — local-foreign join (left outer).
//! * `$count`   — pipeline-terminating count.
//! * `$addFields` — extend each document with computed scalars.

use std::collections::BTreeMap;

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Match(BTreeMap<String, Value>),
    Project(BTreeMap<String, bool>),
    Group {
        by: String,
        accumulators: Vec<(String, Accumulator)>,
    },
    Sort(Vec<(String, SortOrder)>),
    Limit(usize),
    Skip(usize),
    Unwind(String),
    Lookup {
        from: String,
        local_field: String,
        foreign_field: String,
        as_field: String,
    },
    Count(String),
    AddFields(BTreeMap<String, Value>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Accumulator {
    Sum(String),
    Count,
    First(String),
    Last(String),
    Push(String),
    Avg(String),
    Min(String),
    Max(String),
}

pub fn run(
    stages: &[Stage],
    input: &[Value],
    foreign_collections: &BTreeMap<String, Vec<Value>>,
) -> Vec<Value> {
    let mut docs: Vec<Value> = input.to_vec();
    for stage in stages {
        docs = run_stage(stage, docs, foreign_collections);
    }
    docs
}

fn run_stage(
    stage: &Stage,
    docs: Vec<Value>,
    foreigners: &BTreeMap<String, Vec<Value>>,
) -> Vec<Value> {
    match stage {
        Stage::Match(filter) => docs
            .into_iter()
            .filter(|d| filter.iter().all(|(k, v)| field(d, k) == Some(v.clone())))
            .collect(),
        Stage::Project(spec) => docs
            .into_iter()
            .map(|d| project(d, spec))
            .collect(),
        Stage::Group { by, accumulators } => group(docs, by, accumulators),
        Stage::Sort(keys) => {
            let mut docs = docs;
            docs.sort_by(|a, b| compare_docs(a, b, keys));
            docs
        }
        Stage::Limit(n) => docs.into_iter().take(*n).collect(),
        Stage::Skip(n) => docs.into_iter().skip(*n).collect(),
        Stage::Unwind(field_name) => docs
            .into_iter()
            .flat_map(|d| unwind(d, field_name))
            .collect(),
        Stage::Lookup {
            from,
            local_field,
            foreign_field,
            as_field,
        } => {
            let foreign = foreigners.get(from).cloned().unwrap_or_default();
            docs.into_iter()
                .map(|mut d| {
                    let local = field(&d, local_field);
                    let matches: Vec<Value> = foreign
                        .iter()
                        .filter(|f| field(f, foreign_field) == local)
                        .cloned()
                        .collect();
                    if let Some(obj) = d.as_object_mut() {
                        obj.insert(as_field.clone(), Value::Array(matches));
                    }
                    d
                })
                .collect()
        }
        Stage::Count(name) => {
            let count = docs.len();
            vec![json!({ name.clone(): count as i64 })]
        }
        Stage::AddFields(spec) => docs
            .into_iter()
            .map(|mut d| {
                if let Some(obj) = d.as_object_mut() {
                    for (k, v) in spec {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                d
            })
            .collect(),
    }
}

fn project(doc: Value, spec: &BTreeMap<String, bool>) -> Value {
    let include: bool = spec.values().any(|v| *v);
    if let Some(obj) = doc.as_object() {
        let mut out = serde_json::Map::new();
        for (k, v) in obj {
            let included = match (include, spec.get(k)) {
                (true, Some(true)) => true,
                (true, Some(false)) => false,
                (true, None) => k == "_id",
                (false, Some(false)) => false,
                (false, _) => true,
            };
            if included {
                out.insert(k.clone(), v.clone());
            }
        }
        Value::Object(out)
    } else {
        doc
    }
}

fn group(
    docs: Vec<Value>,
    by: &str,
    accumulators: &[(String, Accumulator)],
) -> Vec<Value> {
    let mut buckets: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for d in docs {
        let key = field(&d, by)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".into());
        buckets.entry(key).or_default().push(d);
    }
    let mut out = Vec::with_capacity(buckets.len());
    for (k, group_docs) in buckets {
        let mut row = serde_json::Map::new();
        let key_value: Value = serde_json::from_str(&k).unwrap_or(Value::String(k));
        row.insert("_id".into(), key_value);
        for (name, acc) in accumulators {
            let v = compute_acc(acc, &group_docs);
            row.insert(name.clone(), v);
        }
        out.push(Value::Object(row));
    }
    out
}

fn compute_acc(acc: &Accumulator, docs: &[Value]) -> Value {
    match acc {
        Accumulator::Sum(field_name) => {
            let mut total = 0.0;
            for d in docs {
                if let Some(v) = field(d, field_name) {
                    if let Some(n) = v.as_f64() {
                        total += n;
                    }
                }
            }
            json!(total)
        }
        Accumulator::Count => json!(docs.len() as i64),
        Accumulator::First(f) => docs
            .first()
            .and_then(|d| field(d, f))
            .unwrap_or(Value::Null),
        Accumulator::Last(f) => docs
            .last()
            .and_then(|d| field(d, f))
            .unwrap_or(Value::Null),
        Accumulator::Push(f) => Value::Array(
            docs.iter()
                .filter_map(|d| field(d, f))
                .collect(),
        ),
        Accumulator::Avg(f) => {
            let mut total = 0.0;
            let mut n = 0;
            for d in docs {
                if let Some(v) = field(d, f).and_then(|v| v.as_f64()) {
                    total += v;
                    n += 1;
                }
            }
            if n == 0 {
                Value::Null
            } else {
                json!(total / n as f64)
            }
        }
        Accumulator::Min(f) => docs
            .iter()
            .filter_map(|d| field(d, f))
            .min_by(|a, b| compare_values(a, b))
            .unwrap_or(Value::Null),
        Accumulator::Max(f) => docs
            .iter()
            .filter_map(|d| field(d, f))
            .max_by(|a, b| compare_values(a, b))
            .unwrap_or(Value::Null),
    }
}

fn unwind(doc: Value, field_name: &str) -> Vec<Value> {
    let arr = field(&doc, field_name);
    let arr = match arr {
        Some(Value::Array(a)) => a,
        _ => return vec![],
    };
    arr.into_iter()
        .map(|el| {
            let mut new_doc = doc.clone();
            if let Some(obj) = new_doc.as_object_mut() {
                obj.insert(field_name.to_string(), el);
            }
            new_doc
        })
        .collect()
}

fn field(doc: &Value, name: &str) -> Option<Value> {
    doc.as_object().and_then(|o| o.get(name)).cloned()
}

fn compare_docs(a: &Value, b: &Value, keys: &[(String, SortOrder)]) -> std::cmp::Ordering {
    for (k, ord) in keys {
        let va = field(a, k).unwrap_or(Value::Null);
        let vb = field(b, k).unwrap_or(Value::Null);
        let cmp = compare_values(&va, &vb);
        let cmp = match ord {
            SortOrder::Asc => cmp,
            SortOrder::Desc => cmp.reverse(),
        };
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
    }
    std::cmp::Ordering::Equal
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&y.as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json_str: &str) -> Value {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn match_filter_equality() {
        let docs = vec![doc(r#"{"x":1}"#), doc(r#"{"x":2}"#)];
        let mut filter = BTreeMap::new();
        filter.insert("x".into(), json!(2));
        let out = run(&[Stage::Match(filter)], &docs, &BTreeMap::new());
        assert_eq!(out, vec![doc(r#"{"x":2}"#)]);
    }

    #[test]
    fn project_include_explicit() {
        let docs = vec![doc(r#"{"a":1,"b":2,"c":3}"#)];
        let mut spec = BTreeMap::new();
        spec.insert("a".into(), true);
        spec.insert("c".into(), true);
        let out = run(&[Stage::Project(spec)], &docs, &BTreeMap::new());
        let m = out[0].as_object().unwrap();
        assert!(m.contains_key("a"));
        assert!(m.contains_key("c"));
        assert!(!m.contains_key("b"));
    }

    #[test]
    fn project_exclude_explicit() {
        let docs = vec![doc(r#"{"a":1,"b":2,"c":3}"#)];
        let mut spec = BTreeMap::new();
        spec.insert("b".into(), false);
        let out = run(&[Stage::Project(spec)], &docs, &BTreeMap::new());
        let m = out[0].as_object().unwrap();
        assert!(m.contains_key("a"));
        assert!(!m.contains_key("b"));
        assert!(m.contains_key("c"));
    }

    #[test]
    fn group_count_per_key() {
        let docs = vec![
            doc(r#"{"k":"a"}"#),
            doc(r#"{"k":"a"}"#),
            doc(r#"{"k":"b"}"#),
        ];
        let stage = Stage::Group {
            by: "k".into(),
            accumulators: vec![("n".into(), Accumulator::Count)],
        };
        let out = run(&[stage], &docs, &BTreeMap::new());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn group_sum_field() {
        let docs = vec![doc(r#"{"k":"a","v":1}"#), doc(r#"{"k":"a","v":2}"#)];
        let stage = Stage::Group {
            by: "k".into(),
            accumulators: vec![("total".into(), Accumulator::Sum("v".into()))],
        };
        let out = run(&[stage], &docs, &BTreeMap::new());
        assert_eq!(out[0]["total"].as_f64(), Some(3.0));
    }

    #[test]
    fn group_avg_field() {
        let docs = vec![
            doc(r#"{"k":"a","v":2}"#),
            doc(r#"{"k":"a","v":4}"#),
            doc(r#"{"k":"a","v":6}"#),
        ];
        let stage = Stage::Group {
            by: "k".into(),
            accumulators: vec![("mean".into(), Accumulator::Avg("v".into()))],
        };
        let out = run(&[stage], &docs, &BTreeMap::new());
        assert_eq!(out[0]["mean"].as_f64(), Some(4.0));
    }

    #[test]
    fn group_min_max() {
        let docs = vec![
            doc(r#"{"k":"a","v":3}"#),
            doc(r#"{"k":"a","v":1}"#),
            doc(r#"{"k":"a","v":2}"#),
        ];
        let stage = Stage::Group {
            by: "k".into(),
            accumulators: vec![
                ("lo".into(), Accumulator::Min("v".into())),
                ("hi".into(), Accumulator::Max("v".into())),
            ],
        };
        let out = run(&[stage], &docs, &BTreeMap::new());
        assert_eq!(out[0]["lo"].as_i64(), Some(1));
        assert_eq!(out[0]["hi"].as_i64(), Some(3));
    }

    #[test]
    fn sort_ascending() {
        let docs = vec![
            doc(r#"{"v":3}"#),
            doc(r#"{"v":1}"#),
            doc(r#"{"v":2}"#),
        ];
        let out = run(
            &[Stage::Sort(vec![("v".into(), SortOrder::Asc)])],
            &docs,
            &BTreeMap::new(),
        );
        assert_eq!(out[0]["v"].as_i64(), Some(1));
        assert_eq!(out[2]["v"].as_i64(), Some(3));
    }

    #[test]
    fn sort_descending() {
        let docs = vec![
            doc(r#"{"v":1}"#),
            doc(r#"{"v":3}"#),
            doc(r#"{"v":2}"#),
        ];
        let out = run(
            &[Stage::Sort(vec![("v".into(), SortOrder::Desc)])],
            &docs,
            &BTreeMap::new(),
        );
        assert_eq!(out[0]["v"].as_i64(), Some(3));
    }

    #[test]
    fn limit_keeps_first_n() {
        let docs = vec![doc(r#"{"v":1}"#), doc(r#"{"v":2}"#), doc(r#"{"v":3}"#)];
        let out = run(&[Stage::Limit(2)], &docs, &BTreeMap::new());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn skip_drops_first_n() {
        let docs = vec![doc(r#"{"v":1}"#), doc(r#"{"v":2}"#), doc(r#"{"v":3}"#)];
        let out = run(&[Stage::Skip(2)], &docs, &BTreeMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["v"].as_i64(), Some(3));
    }

    #[test]
    fn unwind_explodes_array() {
        let docs = vec![doc(r#"{"id":1,"items":[10,20,30]}"#)];
        let out = run(&[Stage::Unwind("items".into())], &docs, &BTreeMap::new());
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["items"].as_i64(), Some(10));
    }

    #[test]
    fn unwind_empty_array_emits_nothing() {
        let docs = vec![doc(r#"{"id":1,"items":[]}"#)];
        let out = run(&[Stage::Unwind("items".into())], &docs, &BTreeMap::new());
        assert!(out.is_empty());
    }

    #[test]
    fn lookup_left_outer_join() {
        let docs = vec![doc(r#"{"_id":1,"name":"alice","oid":10}"#)];
        let mut foreigners = BTreeMap::new();
        foreigners.insert(
            "orders".into(),
            vec![doc(r#"{"id":10,"item":"book"}"#)],
        );
        let stage = Stage::Lookup {
            from: "orders".into(),
            local_field: "oid".into(),
            foreign_field: "id".into(),
            as_field: "orders".into(),
        };
        let out = run(&[stage], &docs, &foreigners);
        let arr = out[0]["orders"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn count_emits_single_doc() {
        let docs = vec![doc(r#"{"v":1}"#); 5];
        let out = run(
            &[Stage::Count("n".into())],
            &docs,
            &BTreeMap::new(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["n"].as_i64(), Some(5));
    }

    #[test]
    fn add_fields_extends_documents() {
        let docs = vec![doc(r#"{"v":1}"#)];
        let mut spec = BTreeMap::new();
        spec.insert("computed".into(), json!(42));
        let out = run(&[Stage::AddFields(spec)], &docs, &BTreeMap::new());
        assert_eq!(out[0]["computed"].as_i64(), Some(42));
    }

    #[test]
    fn pipeline_match_then_count() {
        let docs = vec![
            doc(r#"{"k":1}"#),
            doc(r#"{"k":2}"#),
            doc(r#"{"k":1}"#),
        ];
        let mut filter = BTreeMap::new();
        filter.insert("k".into(), json!(1));
        let out = run(
            &[Stage::Match(filter), Stage::Count("n".into())],
            &docs,
            &BTreeMap::new(),
        );
        assert_eq!(out[0]["n"].as_i64(), Some(2));
    }

    #[test]
    fn pipeline_sort_skip_limit() {
        let docs: Vec<Value> = (0..10).map(|i| json!({"v": i})).collect();
        let out = run(
            &[
                Stage::Sort(vec![("v".into(), SortOrder::Desc)]),
                Stage::Skip(2),
                Stage::Limit(3),
            ],
            &docs,
            &BTreeMap::new(),
        );
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["v"].as_i64(), Some(7));
        assert_eq!(out[2]["v"].as_i64(), Some(5));
    }

    #[test]
    fn group_first_returns_first_value() {
        let docs = vec![doc(r#"{"k":"a","v":1}"#), doc(r#"{"k":"a","v":2}"#)];
        let stage = Stage::Group {
            by: "k".into(),
            accumulators: vec![("first_v".into(), Accumulator::First("v".into()))],
        };
        let out = run(&[stage], &docs, &BTreeMap::new());
        assert_eq!(out[0]["first_v"].as_i64(), Some(1));
    }

    #[test]
    fn group_push_accumulates_values() {
        let docs = vec![
            doc(r#"{"k":"a","v":1}"#),
            doc(r#"{"k":"a","v":2}"#),
            doc(r#"{"k":"a","v":3}"#),
        ];
        let stage = Stage::Group {
            by: "k".into(),
            accumulators: vec![("vs".into(), Accumulator::Push("v".into()))],
        };
        let out = run(&[stage], &docs, &BTreeMap::new());
        assert_eq!(out[0]["vs"].as_array().unwrap().len(), 3);
    }
}
