// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Projection evaluation for MongoDB-like field inclusion/exclusion.

use crate::bson::Document;
use serde_json::Value;

pub fn apply_projection(doc: &Document, projection: Option<&Document>) -> Document {
    let Some(proj) = projection else {
        return doc.clone();
    };

    // Inclusion mode is triggered by any field set to 1/true, or by a
    // `$elemMatch` projection operator. A bare `$slice` does *not* switch
    // modes — it adjusts an array in place on top of the default output.
    let has_includes = proj.iter().any(|(_, v)| {
        is_truthy_include(v) || is_op(v, "$elemMatch")
    });

    if has_includes {
        let mut result = Document::new();
        for (key, value) in proj {
            if is_truthy_include(value) {
                include_path(doc, key, &mut result);
            } else if let Some(spec) = op_spec(value, "$elemMatch") {
                if let Some(v) = doc.get(key) {
                    result.insert(key.clone(), elem_match_project(v, spec));
                }
            } else if let Some(spec) = op_spec(value, "$slice") {
                if let Some(v) = doc.get(key) {
                    result.insert(key.clone(), slice_value(v, spec));
                }
            }
        }
        // _id is included by default unless explicitly excluded.
        let id_excluded = proj
            .get("_id")
            .map(|v| matches!(v.as_i64(), Some(0)) || v.as_bool() == Some(false))
            .unwrap_or(false);
        if !id_excluded {
            if let Some(id) = doc.get("_id") {
                result.entry("_id".to_string()).or_insert_with(|| id.clone());
            }
        }
        result
    } else {
        // Exclusion / operator-only mode: start from the full document.
        let mut result = doc.clone();
        for (key, value) in proj {
            if matches!(value.as_i64(), Some(0)) || value.as_bool() == Some(false) {
                exclude_path(&mut result, key);
            } else if let Some(spec) = op_spec(value, "$slice") {
                if let Some(v) = result.get(key).cloned() {
                    result.insert(key.clone(), slice_value(&v, spec));
                }
            } else if let Some(spec) = op_spec(value, "$elemMatch") {
                if let Some(v) = result.get(key).cloned() {
                    result.insert(key.clone(), elem_match_project(&v, spec));
                }
            }
        }
        result
    }
}

fn is_truthy_include(v: &Value) -> bool {
    matches!(v.as_i64(), Some(n) if n != 0) || v.as_bool() == Some(true)
}

fn is_op(v: &Value, op: &str) -> bool {
    v.as_object().map(|o| o.contains_key(op)).unwrap_or(false)
}

fn op_spec<'a>(v: &'a Value, op: &str) -> Option<&'a Value> {
    v.as_object().and_then(|o| o.get(op))
}

/// Copy a (possibly dotted) inclusion path from `doc` into `result`, rebuilding
/// nested document structure as needed.
fn include_path(doc: &Document, path: &str, result: &mut Document) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.len() == 1 {
        if let Some(v) = doc.get(path) {
            result.insert(path.to_string(), v.clone());
        }
        return;
    }
    // Resolve the source value down the dotted path.
    let mut src = match doc.get(parts[0]) {
        Some(v) => v.clone(),
        None => return,
    };
    for p in &parts[1..] {
        src = match src.as_object().and_then(|o| o.get(*p)) {
            Some(v) => v.clone(),
            None => return,
        };
    }
    // Merge `{parts[0]: {parts[1]: {... : src}}}` into result.
    let nested = build_nested(&parts[1..], src);
    let top = parts[0].to_string();
    match result.get_mut(&top) {
        Some(Value::Object(existing)) => {
            if let Value::Object(add) = nested {
                deep_merge(existing, add);
            }
        }
        _ => {
            result.insert(top, nested);
        }
    }
}

fn build_nested(parts: &[&str], leaf: Value) -> Value {
    if parts.is_empty() {
        return leaf;
    }
    let mut m = serde_json::Map::new();
    m.insert(parts[0].to_string(), build_nested(&parts[1..], leaf));
    Value::Object(m)
}

fn deep_merge(into: &mut serde_json::Map<String, Value>, from: serde_json::Map<String, Value>) {
    for (k, v) in from {
        match (into.get_mut(&k), v) {
            (Some(Value::Object(a)), Value::Object(b)) => deep_merge(a, b),
            (_, v) => {
                into.insert(k, v);
            }
        }
    }
}

/// Remove a (possibly dotted) exclusion path from `doc`.
fn exclude_path(doc: &mut Document, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.len() == 1 {
        doc.remove(path);
        return;
    }
    if let Some(Value::Object(obj)) = doc.get_mut(parts[0]) {
        exclude_nested(obj, &parts[1..]);
    }
}

fn exclude_nested(obj: &mut serde_json::Map<String, Value>, parts: &[&str]) {
    if parts.len() == 1 {
        obj.remove(parts[0]);
        return;
    }
    if let Some(Value::Object(inner)) = obj.get_mut(parts[0]) {
        exclude_nested(inner, &parts[1..]);
    }
}

/// `$slice` projection: positive `n` → first n; negative `n` → last |n|;
/// `[skip, limit]` → skip then take limit.
fn slice_value(v: &Value, spec: &Value) -> Value {
    let Some(arr) = v.as_array() else {
        return v.clone();
    };
    let len = arr.len() as i64;
    let (start, count) = match spec {
        Value::Array(pair) if pair.len() == 2 => {
            let skip = pair[0].as_i64().unwrap_or(0);
            let limit = pair[1].as_i64().unwrap_or(0).max(0);
            let start = if skip < 0 { (len + skip).max(0) } else { skip.min(len) };
            (start, limit)
        }
        _ => {
            let n = spec.as_i64().unwrap_or(0);
            if n >= 0 {
                (0, n)
            } else {
                ((len + n).max(0), -n)
            }
        }
    };
    let start = start as usize;
    let end = (start + count as usize).min(arr.len());
    Value::Array(arr.get(start..end).map(|s| s.to_vec()).unwrap_or_default())
}

/// `$elemMatch` projection: keep only the first array element matching the
/// sub-query (wrapped in a single-element array, MongoDB semantics).
fn elem_match_project(v: &Value, spec: &Value) -> Value {
    let Some(arr) = v.as_array() else {
        return v.clone();
    };
    let Some(criteria) = spec.as_object() else {
        return Value::Array(vec![]);
    };
    let sub: Document = criteria.iter().map(|(k, x)| (k.clone(), x.clone())).collect();
    for e in arr {
        if let Some(eo) = e.as_object() {
            let ed: Document = eo.iter().map(|(k, x)| (k.clone(), x.clone())).collect();
            if crate::query::matches_query(&ed, &sub) {
                return Value::Array(vec![e.clone()]);
            }
        }
    }
    Value::Array(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inclusion_projection() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("alice".to_string()));
        doc.insert(
            "email".to_string(),
            Value::String("alice@example.com".to_string()),
        );

        let mut proj = Document::new();
        proj.insert("name".to_string(), Value::Number(1.into()));

        let result = apply_projection(&doc, Some(&proj));
        assert!(result.contains_key("name"));
        assert!(result.contains_key("_id")); // _id included by default
        assert!(!result.contains_key("email"));
    }

    #[test]
    fn test_exclusion_projection() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("alice".to_string()));
        doc.insert(
            "email".to_string(),
            Value::String("alice@example.com".to_string()),
        );
        doc.insert("password".to_string(), Value::String("secret".to_string()));

        let mut proj = Document::new();
        proj.insert("password".to_string(), Value::Number(0.into()));

        let result = apply_projection(&doc, Some(&proj));
        assert!(result.contains_key("name"));
        assert!(result.contains_key("_id"));
        assert!(result.contains_key("email"));
        assert!(!result.contains_key("password"));
    }

    #[test]
    fn test_no_projection() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("alice".to_string()));

        let result = apply_projection(&doc, None);
        assert_eq!(result, doc);
    }

    // ── Cycle 4: $slice / $elemMatch / dotted-path projection ──────────────────

    use serde_json::json;

    fn doc_of(v: Value) -> Document {
        v.as_object().unwrap().iter().map(|(k, x)| (k.clone(), x.clone())).collect()
    }

    fn proj_of(v: Value) -> Document {
        doc_of(v)
    }

    #[test]
    fn test_slice_positive_takes_first_n() {
        let doc = doc_of(json!({"_id": 1, "tags": ["a", "b", "c", "d"]}));
        let proj = proj_of(json!({"tags": {"$slice": 2}}));
        let r = apply_projection(&doc, Some(&proj));
        assert_eq!(r.get("tags").unwrap(), &json!(["a", "b"]));
        // $slice is an inclusion-style operator but does not flip to pure
        // inclusion: other fields remain unless excluded.
        assert!(r.contains_key("_id"));
    }

    #[test]
    fn test_slice_negative_takes_last_n() {
        let doc = doc_of(json!({"_id": 1, "tags": ["a", "b", "c", "d"]}));
        let proj = proj_of(json!({"tags": {"$slice": -2}}));
        let r = apply_projection(&doc, Some(&proj));
        assert_eq!(r.get("tags").unwrap(), &json!(["c", "d"]));
    }

    #[test]
    fn test_slice_skip_limit() {
        let doc = doc_of(json!({"tags": ["a", "b", "c", "d", "e"]}));
        let proj = proj_of(json!({"tags": {"$slice": [1, 2]}}));
        let r = apply_projection(&doc, Some(&proj));
        assert_eq!(r.get("tags").unwrap(), &json!(["b", "c"]));
    }

    #[test]
    fn test_elemmatch_projection_first_match() {
        let doc = doc_of(json!({
            "_id": 1,
            "items": [{"k": "x", "n": 1}, {"k": "y", "n": 2}, {"k": "y", "n": 3}]
        }));
        let proj = proj_of(json!({"items": {"$elemMatch": {"k": "y"}}}));
        let r = apply_projection(&doc, Some(&proj));
        // Only the first matching element is projected.
        assert_eq!(r.get("items").unwrap(), &json!([{"k": "y", "n": 2}]));
        assert!(r.contains_key("_id"));
    }

    #[test]
    fn test_dotted_path_inclusion() {
        let doc = doc_of(json!({
            "_id": 1,
            "name": "a",
            "addr": {"city": "NYC", "zip": "10001"}
        }));
        let proj = proj_of(json!({"addr.city": 1}));
        let r = apply_projection(&doc, Some(&proj));
        // Nested inclusion keeps only addr.city, drops name and addr.zip.
        assert_eq!(r.get("addr").unwrap(), &json!({"city": "NYC"}));
        assert!(!r.contains_key("name"));
        assert!(r.contains_key("_id"));
    }

    #[test]
    fn test_dotted_path_exclusion() {
        let doc = doc_of(json!({
            "_id": 1,
            "addr": {"city": "NYC", "zip": "10001"}
        }));
        let proj = proj_of(json!({"addr.zip": 0}));
        let r = apply_projection(&doc, Some(&proj));
        assert_eq!(r.get("addr").unwrap(), &json!({"city": "NYC"}));
    }
}
