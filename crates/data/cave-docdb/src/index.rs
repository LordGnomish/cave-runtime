// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Index metadata management and index-ordered document comparison.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub keys: BTreeMap<String, i32>, // field -> 1 (asc) or -1 (desc)
    pub unique: bool,
    /// Declared key order. MongoDB compound indexes are order-significant
    /// (`{a:1, b:-1}` differs from `{b:-1, a:1}`), but `keys` is a BTreeMap and
    /// loses that order. `key_order` records the original field sequence so
    /// index-ordered scans honour the declaration. Defaults (for older
    /// serialized state) to the BTreeMap's alphabetical iteration order.
    #[serde(default)]
    key_order: Vec<String>,
    /// Fields covered by a text index (`{field: "text"}`). Empty for ordinary
    /// b-tree indexes; non-empty marks this as a text index searched by `$text`.
    #[serde(default)]
    text_fields: Vec<String>,
}

impl Index {
    pub fn new(name: String, keys: BTreeMap<String, i32>, unique: bool) -> Self {
        let key_order = keys.keys().cloned().collect();
        Self {
            name,
            keys,
            unique,
            key_order,
            text_fields: Vec::new(),
        }
    }

    /// Build a text index over the named string fields.
    pub fn text(name: String, fields: Vec<String>) -> Self {
        Self {
            name,
            keys: BTreeMap::new(),
            unique: false,
            key_order: Vec::new(),
            text_fields: fields,
        }
    }

    /// Whether this is a text index.
    pub fn is_text(&self) -> bool {
        !self.text_fields.is_empty()
    }

    /// The fields a text index covers (empty for non-text indexes).
    pub fn text_fields(&self) -> &[String] {
        &self.text_fields
    }

    /// Build an index preserving the declared compound-key order.
    pub fn with_order(name: String, ordered: Vec<(String, i32)>, unique: bool) -> Self {
        let mut keys = BTreeMap::new();
        let mut key_order = Vec::with_capacity(ordered.len());
        for (field, dir) in ordered {
            keys.insert(field.clone(), dir);
            key_order.push(field);
        }
        Self {
            name,
            keys,
            unique,
            key_order,
            text_fields: Vec::new(),
        }
    }

    /// The index key fields in declared order, paired with their direction.
    pub fn ordered_keys(&self) -> Vec<(&str, i32)> {
        self.key_order
            .iter()
            .map(|f| (f.as_str(), *self.keys.get(f).unwrap_or(&1)))
            .collect()
    }

    /// Extract this document's sort key: one `(value, direction)` pair per
    /// index field, in declared order. A missing field yields JSON null, which
    /// sorts before any present value (MongoDB treats absent as lowest BSON).
    pub fn sort_key(&self, doc: &Value) -> Vec<(Value, i32)> {
        self.ordered_keys()
            .into_iter()
            .map(|(field, dir)| {
                let v = doc.get(field).cloned().unwrap_or(Value::Null);
                (v, dir)
            })
            .collect()
    }

    /// Compare two documents according to this index's ordering.
    pub fn compare(&self, a: &Value, b: &Value) -> Ordering {
        for (field, dir) in self.ordered_keys() {
            let av = a.get(field).unwrap_or(&Value::Null);
            let bv = b.get(field).unwrap_or(&Value::Null);
            let ord = cmp_bson(av, bv);
            if ord != Ordering::Equal {
                return if dir >= 0 { ord } else { ord.reverse() };
            }
        }
        Ordering::Equal
    }

    /// Sort documents in place into index order.
    pub fn sort_documents(&self, docs: &mut [Value]) {
        docs.sort_by(|a, b| self.compare(a, b));
    }
}

/// Compare two JSON values using a BSON-like canonical ordering. Null is the
/// lowest type; numbers compare numerically; strings lexically; bools false
/// before true. Cross-type comparisons fall back to a stable type rank.
fn cmp_bson(a: &Value, b: &Value) -> Ordering {
    fn rank(v: &Value) -> u8 {
        match v {
            Value::Null => 0,
            Value::Bool(_) => 1,
            Value::Number(_) => 2,
            Value::String(_) => 3,
            Value::Array(_) => 4,
            Value::Object(_) => 5,
        }
    }
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Number(x), Value::Number(y)) => {
            let xf = x.as_f64().unwrap_or(f64::NAN);
            let yf = y.as_f64().unwrap_or(f64::NAN);
            xf.partial_cmp(&yf).unwrap_or(Ordering::Equal)
        }
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn idx(pairs: &[(&str, i32)]) -> Index {
        let mut keys = BTreeMap::new();
        for (k, d) in pairs {
            keys.insert(k.to_string(), *d);
        }
        Index::new("test_idx".into(), keys, false)
    }

    fn idx_ordered(pairs: &[(&str, i32)]) -> Index {
        Index::with_order(
            "test_idx".into(),
            pairs.iter().map(|(k, d)| (k.to_string(), *d)).collect(),
            false,
        )
    }

    #[test]
    fn descending_single_index_orders_high_to_low() {
        let index = idx(&[("score", -1)]);
        let mut docs: Vec<Value> =
            vec![json!({"score": 1}), json!({"score": 3}), json!({"score": 2})];
        index.sort_documents(&mut docs);
        let order: Vec<i64> = docs.iter().map(|d| d["score"].as_i64().unwrap()).collect();
        assert_eq!(order, vec![3, 2, 1]);
    }

    #[test]
    fn compound_index_orders_by_first_key_then_second() {
        // {dept: 1, score: -1} — dept ascending, then score descending within dept.
        let index = idx_ordered(&[("dept", 1), ("score", -1)]);
        let mut docs: Vec<Value> = vec![
            json!({"dept": "b", "score": 5}),
            json!({"dept": "a", "score": 1}),
            json!({"dept": "a", "score": 9}),
            json!({"dept": "b", "score": 7}),
        ];
        index.sort_documents(&mut docs);
        let order: Vec<(String, i64)> = docs
            .iter()
            .map(|d| {
                (
                    d["dept"].as_str().unwrap().to_string(),
                    d["score"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("a".into(), 9),
                ("a".into(), 1),
                ("b".into(), 7),
                ("b".into(), 5),
            ]
        );
    }

    #[test]
    fn declared_order_is_preserved_independent_of_alphabetical() {
        // {score: -1, dept: 1}: first key is "score" even though "dept" < "score"
        // alphabetically — a BTreeMap-iteration index would get this wrong.
        let index = idx_ordered(&[("score", -1), ("dept", 1)]);
        assert_eq!(index.ordered_keys(), vec![("score", -1), ("dept", 1)]);
        let mut docs: Vec<Value> = vec![
            json!({"dept": "z", "score": 1}),
            json!({"dept": "a", "score": 9}),
            json!({"dept": "a", "score": 1}),
        ];
        index.sort_documents(&mut docs);
        // Primary key score desc: 9 first, then the two score=1 by dept asc (a, z).
        let order: Vec<(i64, String)> = docs
            .iter()
            .map(|d| {
                (
                    d["score"].as_i64().unwrap(),
                    d["dept"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![(9, "a".into()), (1, "a".into()), (1, "z".into())]
        );
    }

    #[test]
    fn missing_field_sorts_before_present_values() {
        let index = idx(&[("score", 1)]);
        let mut docs: Vec<Value> =
            vec![json!({"score": 2}), json!({"other": 1}), json!({"score": 1})];
        index.sort_documents(&mut docs);
        assert!(docs[0].get("score").is_none());
        assert_eq!(docs[1]["score"].as_i64(), Some(1));
        assert_eq!(docs[2]["score"].as_i64(), Some(2));
    }

    #[test]
    fn sort_key_reflects_compound_order_and_direction() {
        let index = idx_ordered(&[("a", 1), ("b", -1)]);
        let key = index.sort_key(&json!({"a": 5, "b": 7}));
        assert_eq!(key.len(), 2);
        assert_eq!(key[0].0.as_i64(), Some(5));
        assert_eq!(key[0].1, 1);
        assert_eq!(key[1].0.as_i64(), Some(7));
        assert_eq!(key[1].1, -1);
    }
}
