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
        _ => {
            match (a.as_f64(), b.as_f64()) {
                (Some(av), Some(bv)) => {
                    if av.is_nan() || bv.is_nan() {
                        false
                    } else {
                        cmp(av as i64, bv as i64)
                    }
                }
                _ => false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        query.insert("$and".to_string(), Value::Array(vec![
            Value::Object(cond1),
            Value::Object(cond2),
        ]));

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
        query.insert("$or".to_string(), Value::Array(vec![
            Value::Object(cond1),
            Value::Object(cond2),
        ]));

        assert!(matches_query(&doc, &query));
    }

    #[test]
    fn test_regex_operator() {
        let mut doc = Document::new();
        doc.insert("email".to_string(), Value::String("test@example.com".to_string()));

        let mut op = serde_json::Map::new();
        op.insert("$regex".to_string(), Value::String(".*@example\\.com".to_string()));
        let mut query = Document::new();
        query.insert("email".to_string(), Value::Object(op));

        assert!(matches_query(&doc, &query));
    }
}
