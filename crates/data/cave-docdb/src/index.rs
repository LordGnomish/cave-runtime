// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Index metadata management.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub keys: BTreeMap<String, i32>, // field -> 1 (asc) or -1 (desc)
    pub unique: bool,
}

impl Index {
    pub fn new(name: String, keys: BTreeMap<String, i32>, unique: bool) -> Self {
        Self { name, keys, unique }
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
