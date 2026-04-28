//! OpenSearch aggregation evaluator — terms / sum / avg / min / max / value_count.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::index::IndexData;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    Terms { field: String, size: Option<usize> },
    Sum { field: String },
    Avg { field: String },
    Min { field: String },
    Max { field: String },
    ValueCount { field: String },
    Cardinality { field: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregationResult {
    pub name: String,
    pub value: Value,
}

pub fn execute(name: &str, agg: &Aggregation, data: &IndexData) -> AggregationResult {
    let value = match agg {
        Aggregation::Terms { field, size } => terms_agg(field, *size, data),
        Aggregation::Sum { field } => Value::from(numeric_sum(field, data)),
        Aggregation::Avg { field } => match numeric_avg(field, data) {
            Some(v) => Value::from(v),
            None => Value::Null,
        },
        Aggregation::Min { field } => match numeric_min(field, data) {
            Some(v) => Value::from(v),
            None => Value::Null,
        },
        Aggregation::Max { field } => match numeric_max(field, data) {
            Some(v) => Value::from(v),
            None => Value::Null,
        },
        Aggregation::ValueCount { field } => Value::from(value_count(field, data)),
        Aggregation::Cardinality { field } => Value::from(cardinality(field, data) as u64),
    };
    AggregationResult {
        name: name.to_string(),
        value,
    }
}

fn terms_agg(field: &str, size: Option<usize>, data: &IndexData) -> Value {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for doc in data.docs.values() {
        if let Some(v) = doc.source.get(field) {
            let key = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    let mut sorted: Vec<(String, u64)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let cap = size.unwrap_or(10);
    let buckets: Vec<Value> = sorted
        .into_iter()
        .take(cap)
        .map(|(k, c)| serde_json::json!({"key": k, "doc_count": c}))
        .collect();
    serde_json::json!({"buckets": buckets})
}

fn numeric_sum(field: &str, data: &IndexData) -> f64 {
    data.docs
        .values()
        .filter_map(|d| d.source.get(field).and_then(|v| v.as_f64()))
        .sum()
}

fn numeric_avg(field: &str, data: &IndexData) -> Option<f64> {
    let values: Vec<f64> = data
        .docs
        .values()
        .filter_map(|d| d.source.get(field).and_then(|v| v.as_f64()))
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

fn numeric_min(field: &str, data: &IndexData) -> Option<f64> {
    data.docs
        .values()
        .filter_map(|d| d.source.get(field).and_then(|v| v.as_f64()))
        .fold(None, |acc, v| match acc {
            None => Some(v),
            Some(a) if v < a => Some(v),
            other => other,
        })
}

fn numeric_max(field: &str, data: &IndexData) -> Option<f64> {
    data.docs
        .values()
        .filter_map(|d| d.source.get(field).and_then(|v| v.as_f64()))
        .fold(None, |acc, v| match acc {
            None => Some(v),
            Some(a) if v > a => Some(v),
            other => other,
        })
}

fn value_count(field: &str, data: &IndexData) -> u64 {
    data.docs
        .values()
        .filter(|d| d.source.get(field).map(|v| !v.is_null()).unwrap_or(false))
        .count() as u64
}

fn cardinality(field: &str, data: &IndexData) -> usize {
    use std::collections::HashSet;
    let mut s: HashSet<String> = HashSet::new();
    for doc in data.docs.values() {
        if let Some(v) = doc.source.get(field) {
            s.insert(v.to_string());
        }
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexRegistry;
    use crate::mapping::{FieldType, IndexMapping};
    use serde_json::json;

    fn setup() -> (IndexRegistry, &'static str) {
        let r = IndexRegistry::new();
        let mut m = IndexMapping::new();
        m.add_field("color", FieldType::Keyword);
        m.add_field("price", FieldType::Long);
        r.create("p", m).unwrap();
        r.index_doc("p", "1", json!({"color": "red", "price": 10})).unwrap();
        r.index_doc("p", "2", json!({"color": "blue", "price": 20})).unwrap();
        r.index_doc("p", "3", json!({"color": "red", "price": 30})).unwrap();
        r.index_doc("p", "4", json!({"color": "red", "price": 40})).unwrap();
        (r, "p")
    }

    fn run(r: &IndexRegistry, n: &str, name: &str, a: Aggregation) -> AggregationResult {
        r.with_index(n, |d| execute(name, &a, d)).unwrap()
    }

    #[test]
    fn terms_agg_returns_buckets_descending() {
        let (r, n) = setup();
        let res = run(
            &r,
            n,
            "by_color",
            Aggregation::Terms {
                field: "color".into(),
                size: None,
            },
        );
        let buckets = res.value["buckets"].as_array().unwrap();
        assert_eq!(buckets[0]["key"], "red");
        assert_eq!(buckets[0]["doc_count"], 3);
    }

    #[test]
    fn terms_agg_respects_size_cap() {
        let (r, n) = setup();
        let res = run(
            &r,
            n,
            "by_color",
            Aggregation::Terms {
                field: "color".into(),
                size: Some(1),
            },
        );
        assert_eq!(res.value["buckets"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn sum_returns_total() {
        let (r, n) = setup();
        let res = run(&r, n, "sum_price", Aggregation::Sum { field: "price".into() });
        assert_eq!(res.value.as_f64().unwrap(), 100.0);
    }

    #[test]
    fn avg_returns_mean() {
        let (r, n) = setup();
        let res = run(&r, n, "avg_price", Aggregation::Avg { field: "price".into() });
        assert_eq!(res.value.as_f64().unwrap(), 25.0);
    }

    #[test]
    fn min_returns_smallest() {
        let (r, n) = setup();
        let res = run(&r, n, "min", Aggregation::Min { field: "price".into() });
        assert_eq!(res.value.as_f64().unwrap(), 10.0);
    }

    #[test]
    fn max_returns_largest() {
        let (r, n) = setup();
        let res = run(&r, n, "max", Aggregation::Max { field: "price".into() });
        assert_eq!(res.value.as_f64().unwrap(), 40.0);
    }

    #[test]
    fn value_count_counts_non_null() {
        let (r, n) = setup();
        let res = run(
            &r,
            n,
            "vc",
            Aggregation::ValueCount {
                field: "price".into(),
            },
        );
        assert_eq!(res.value.as_u64().unwrap(), 4);
    }

    #[test]
    fn cardinality_distinct_count() {
        let (r, n) = setup();
        let res = run(
            &r,
            n,
            "c",
            Aggregation::Cardinality {
                field: "color".into(),
            },
        );
        assert_eq!(res.value.as_u64().unwrap(), 2);
    }

    #[test]
    fn avg_on_empty_field_returns_null() {
        let r = IndexRegistry::new();
        r.create("p", IndexMapping::new()).unwrap();
        let res = run(&r, "p", "a", Aggregation::Avg { field: "missing".into() });
        assert!(res.value.is_null());
    }

    #[test]
    fn deserialize_terms_agg_from_json() {
        let s = r#"{"terms":{"field":"color","size":5}}"#;
        let a: Aggregation = serde_json::from_str(s).unwrap();
        match a {
            Aggregation::Terms { field, size } => {
                assert_eq!(field, "color");
                assert_eq!(size, Some(5));
            }
            other => panic!("expected Terms, got {:?}", other),
        }
    }
}
