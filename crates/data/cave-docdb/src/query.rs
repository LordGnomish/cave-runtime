// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query filter evaluation for MongoDB-like operators.

use crate::bson::Document;
use regex::Regex;
use serde_json::Value;

pub fn matches_query(doc: &Document, query: &Document) -> bool {
    for (key, query_value) in query {
        if key == "$and" {
            if let Some(conditions) = query_value.as_array() {
                if !conditions.iter().all(|cond| {
                    if let Some(cond_obj) = cond.as_object() {
                        let mut cond_doc = Document::new();
                        for (k, v) in cond_obj {
                            cond_doc.insert(k.clone(), v.clone());
                        }
                        matches_query(doc, &cond_doc)
                    } else {
                        false
                    }
                }) {
                    return false;
                }
            }
        } else if key == "$or" {
            if let Some(conditions) = query_value.as_array() {
                if !conditions.iter().any(|cond| {
                    if let Some(cond_obj) = cond.as_object() {
                        let mut cond_doc = Document::new();
                        for (k, v) in cond_obj {
                            cond_doc.insert(k.clone(), v.clone());
                        }
                        matches_query(doc, &cond_doc)
                    } else {
                        false
                    }
                }) {
                    return false;
                }
            }
        } else {
            match doc.get(key) {
                Some(doc_value) => {
                    if !matches_value(doc_value, query_value) {
                        return false;
                    }
                }
                None => {
                    // Check for $exists
                    if let Some(obj) = query_value.as_object() {
                        if let Some(Value::Bool(false)) = obj.get("$exists") {
                            // Document doesn't have field, which matches $exists: false
                            continue;
                        }
                    }
                    return false;
                }
            }
        }
    }
    true
}

pub fn matches_value(doc_value: &Value, query_value: &Value) -> bool {
    match query_value {
        Value::Object(obj) => {
            // Handle operators
            for (op, op_value) in obj {
                match op.as_str() {
                    "$eq" => {
                        if doc_value != op_value {
                            return false;
                        }
                    }
                    "$ne" => {
                        if doc_value == op_value {
                            return false;
                        }
                    }
                    "$gt" => {
                        if !compare_values(doc_value, op_value, |a, b| a > b) {
                            return false;
                        }
                    }
                    "$gte" => {
                        if !compare_values(doc_value, op_value, |a, b| a >= b) {
                            return false;
                        }
                    }
                    "$lt" => {
                        if !compare_values(doc_value, op_value, |a, b| a < b) {
                            return false;
                        }
                    }
                    "$lte" => {
                        if !compare_values(doc_value, op_value, |a, b| a <= b) {
                            return false;
                        }
                    }
                    "$in" => {
                        if let Some(arr) = op_value.as_array() {
                            if !arr.iter().any(|v| doc_value == v) {
                                return false;
                            }
                        } else {
                            return false;
                        }
                    }
                    "$nin" => {
                        if let Some(arr) = op_value.as_array() {
                            if arr.iter().any(|v| doc_value == v) {
                                return false;
                            }
                        }
                    }
                    "$exists" => {
                        let exists = op_value.as_bool().unwrap_or(false);
                        if !exists && !matches!(doc_value, Value::Null) {
                            return false;
                        }
                    }
                    "$regex" => {
                        if let Some(pattern) = op_value.as_str() {
                            if let Some(doc_str) = doc_value.as_str() {
                                if let Ok(re) = Regex::new(pattern) {
                                    if !re.is_match(doc_str) {
                                        return false;
                                    }
                                } else {
                                    return false;
                                }
                            } else {
                                return false;
                            }
                        }
                    }
                    _ => {
                        // Unknown operator, skip
                    }
                }
            }
            true
        }
        // Implicit equality
        _ => doc_value == query_value,
    }
}

fn compare_values<F>(a: &Value, b: &Value, cmp: F) -> bool
where
    F: Fn(i64, i64) -> bool,
{
    match (a.as_i64(), b.as_i64()) {
        (Some(av), Some(bv)) => cmp(av, bv),
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(av), Some(bv)) => {
                if av.is_nan() || bv.is_nan() {
                    false
                } else {
                    cmp(av as i64, bv as i64)
                }
            }
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_implicit_eq() {
        let mut doc = Document::new();
        doc.insert("name".to_string(), Value::String("alice".to_string()));

        let mut query = Document::new();
        query.insert("name".to_string(), Value::String("alice".to_string()));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_explicit_eq() {
        let mut doc = Document::new();
        doc.insert("age".to_string(), Value::Number(30.into()));

        let mut op = serde_json::Map::new();
        op.insert("$eq".to_string(), Value::Number(30.into()));
        let mut query = Document::new();
        query.insert("age".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_ne_operator() {
        let mut doc = Document::new();
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut op = serde_json::Map::new();
        op.insert("$ne".to_string(), Value::String("inactive".to_string()));
        let mut query = Document::new();
        query.insert("status".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_gt_operator() {
        let mut doc = Document::new();
        doc.insert("score".to_string(), Value::Number(100.into()));

        let mut op = serde_json::Map::new();
        op.insert("$gt".to_string(), Value::Number(50.into()));
        let mut query = Document::new();
        query.insert("score".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_in_operator() {
        let mut doc = Document::new();
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut op = serde_json::Map::new();
        op.insert(
            "$in".to_string(),
            Value::Array(vec![
                Value::String("active".to_string()),
                Value::String("pending".to_string()),
            ]),
        );
        let mut query = Document::new();
        query.insert("status".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_and_operator() {
        let mut doc = Document::new();
        doc.insert("age".to_string(), Value::Number(25.into()));
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut cond1 = serde_json::Map::new();
        cond1.insert("age".to_string(), Value::Number(25.into()));
        let mut cond2 = serde_json::Map::new();
        cond2.insert("status".to_string(), Value::String("active".to_string()));

        let mut query = Document::new();
        query.insert(
            "$and".to_string(),
            Value::Array(vec![Value::Object(cond1), Value::Object(cond2)]),
        );

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_or_operator() {
        let mut doc = Document::new();
        doc.insert("status".to_string(), Value::String("active".to_string()));

        let mut cond1 = serde_json::Map::new();
        cond1.insert("status".to_string(), Value::String("active".to_string()));
        let mut cond2 = serde_json::Map::new();
        cond2.insert("status".to_string(), Value::String("inactive".to_string()));

        let mut query = Document::new();
        query.insert(
            "$or".to_string(),
            Value::Array(vec![Value::Object(cond1), Value::Object(cond2)]),
        );

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_regex_operator() {
        let mut doc = Document::new();
        doc.insert(
            "email".to_string(),
            Value::String("test@example.com".to_string()),
        );

        let mut op = serde_json::Map::new();
        op.insert(
            "$regex".to_string(),
            Value::String(".*@example\\.com".to_string()),
        );
        let mut query = Document::new();
        query.insert("email".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }

    // ── Cycle 1a: $type / $size / $all / $not / $nor + missing-field semantics ──

    fn q(field: &str, op: &str, v: Value) -> Document {
        let mut m = serde_json::Map::new();
        m.insert(op.to_string(), v);
        let mut query = Document::new();
        query.insert(field.to_string(), Value::Object(m));
        query
    }

    fn d(field: &str, v: Value) -> Document {
        let mut doc = Document::new();
        doc.insert(field.to_string(), v);
        doc
    }

    #[test]
    fn test_type_string_alias() {
        let doc = d("name", json!("alice"));
        assert!(matches_query(&doc, &q("name", "$type", json!("string"))));
        assert!(!matches_query(&doc, &q("name", "$type", json!("int"))));
    }

    #[test]
    fn test_type_number_groups_int_and_double() {
        assert!(matches_query(&d("n", json!(7)), &q("n", "$type", json!("int"))));
        assert!(matches_query(
            &d("n", json!(7.5)),
            &q("n", "$type", json!("double"))
        ));
        // "number" alias matches both int and double.
        assert!(matches_query(&d("n", json!(7)), &q("n", "$type", json!("number"))));
        assert!(matches_query(
            &d("n", json!(7.5)),
            &q("n", "$type", json!("number"))
        ));
    }

    #[test]
    fn test_type_numeric_bson_code_and_array_form() {
        // BSON code 2 == string; array form matches any listed type.
        assert!(matches_query(&d("s", json!("x")), &q("s", "$type", json!(2))));
        assert!(matches_query(
            &d("s", json!("x")),
            &q("s", "$type", json!(["bool", "string"]))
        ));
        assert!(!matches_query(
            &d("s", json!(true)),
            &q("s", "$type", json!(["string", "int"]))
        ));
    }

    #[test]
    fn test_size_operator() {
        let doc = d("tags", json!(["a", "b", "c"]));
        assert!(matches_query(&doc, &q("tags", "$size", json!(3))));
        assert!(!matches_query(&doc, &q("tags", "$size", json!(2))));
        // non-array never matches $size.
        assert!(!matches_query(&d("tags", json!("a")), &q("tags", "$size", json!(1))));
    }

    #[test]
    fn test_all_operator() {
        let doc = d("tags", json!(["a", "b", "c"]));
        assert!(matches_query(&doc, &q("tags", "$all", json!(["a", "c"]))));
        assert!(!matches_query(&doc, &q("tags", "$all", json!(["a", "z"]))));
    }

    #[test]
    fn test_not_operator_negates_inner() {
        // $not negates an operator expression.
        let doc = d("age", json!(30));
        assert!(matches_query(&doc, &q("age", "$not", json!({"$gt": 50}))));
        assert!(!matches_query(&doc, &q("age", "$not", json!({"$gt": 10}))));
    }

    #[test]
    fn test_not_matches_missing_field() {
        // A missing field does not match $gt, so $not over it matches.
        let doc = d("other", json!(1));
        assert!(matches_query(&doc, &q("age", "$not", json!({"$gt": 10}))));
    }

    #[test]
    fn test_nor_top_level() {
        let doc = d("status", json!("active"));
        let mut query = Document::new();
        query.insert(
            "$nor".to_string(),
            json!([{"status": "inactive"}, {"status": "deleted"}]),
        );
        // none of the conditions match -> $nor true
        assert!(matches_query(&doc, &query));

        let mut query2 = Document::new();
        query2.insert(
            "$nor".to_string(),
            json!([{"status": "active"}, {"status": "deleted"}]),
        );
        // one condition matches -> $nor false
        assert!(!matches_query(&doc, &query2));
    }

    #[test]
    fn test_ne_matches_missing_field() {
        // $ne against a value matches documents lacking the field entirely.
        let doc = d("other", json!(1));
        assert!(matches_query(&doc, &q("status", "$ne", json!("active"))));
    }

    #[test]
    fn test_nin_matches_missing_field() {
        let doc = d("other", json!(1));
        assert!(matches_query(
            &doc,
            &q("status", "$nin", json!(["active", "pending"]))
        ));
    }
}
