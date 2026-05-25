// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Defaulting walker — walks openAPIV3Schema and fills `default:` values
//! into the spec when the key is absent.
//!
//! Upstream: internal/xcrd/render.go::defaultsFromSchema

use serde_json::{Map, Value};

/// Apply defaults from `schema` into `value`, mutating in place.
pub fn apply_defaults(schema: &Value, value: &mut Value) {
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        if !value.is_object() {
            *value = Value::Object(Map::new());
        }
        let obj = value.as_object_mut().unwrap();
        for (k, sub_schema) in props {
            if !obj.contains_key(k) {
                if let Some(d) = sub_schema.get("default") {
                    obj.insert(k.clone(), d.clone());
                }
            }
            // Recurse into nested objects/arrays
            if let Some(v) = obj.get_mut(k) {
                if v.is_object() {
                    apply_defaults(sub_schema, v);
                }
                if v.is_array() {
                    if let Some(items) = sub_schema.get("items") {
                        for item in v.as_array_mut().unwrap() {
                            apply_defaults(items, item);
                        }
                    }
                }
            }
        }
    }
}

/// Collect a flat list of (path → default) from a schema for introspection.
pub fn collect_defaults(schema: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    collect(schema, "$", &mut out);
    out
}

fn collect(schema: &Value, path: &str, out: &mut Vec<(String, Value)>) {
    if let Some(d) = schema.get("default") {
        out.push((path.to_string(), d.clone()));
    }
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        for (k, sub) in props {
            collect(sub, &format!("{}.{}", path, k), out);
        }
    }
    if let Some(items) = schema.get("items") {
        collect(items, &format!("{}[*]", path), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fills_missing() {
        let s = json!({"type":"object","properties":{"size":{"type":"integer","default":10}}});
        let mut v = json!({});
        apply_defaults(&s, &mut v);
        assert_eq!(v["size"], json!(10));
    }

    #[test]
    fn keeps_present() {
        let s = json!({"type":"object","properties":{"size":{"type":"integer","default":10}}});
        let mut v = json!({"size":5});
        apply_defaults(&s, &mut v);
        assert_eq!(v["size"], json!(5));
    }

    #[test]
    fn nested_defaults() {
        let s = json!({
            "type":"object",
            "properties":{
                "db":{"type":"object","properties":{"size":{"type":"integer","default":3}}}
            }
        });
        let mut v = json!({"db":{}});
        apply_defaults(&s, &mut v);
        assert_eq!(v["db"]["size"], json!(3));
    }

    #[test]
    fn creates_object_if_absent() {
        let s = json!({"type":"object","properties":{"a":{"type":"string","default":"x"}}});
        let mut v = json!(null);
        apply_defaults(&s, &mut v);
        assert_eq!(v["a"], json!("x"));
    }

    #[test]
    fn collect_finds_all_defaults() {
        let s = json!({
            "type":"object",
            "properties":{
                "a":{"default":1},
                "b":{"type":"object","properties":{"c":{"default":2}}}
            }
        });
        let defs = collect_defaults(&s);
        assert_eq!(defs.len(), 2);
    }

    #[test]
    fn array_item_default_applied() {
        let s = json!({
            "type":"object",
            "properties":{
                "list":{"type":"array","items":{"type":"object","properties":{"x":{"default":7}}}}
            }
        });
        let mut v = json!({"list":[{},{"x":1}]});
        apply_defaults(&s, &mut v);
        assert_eq!(v["list"][0]["x"], json!(7));
        assert_eq!(v["list"][1]["x"], json!(1));
    }

    #[test]
    fn no_defaults_no_change() {
        let s = json!({"type":"object","properties":{"x":{"type":"string"}}});
        let mut v = json!({});
        apply_defaults(&s, &mut v);
        assert_eq!(v, json!({}));
    }
}
