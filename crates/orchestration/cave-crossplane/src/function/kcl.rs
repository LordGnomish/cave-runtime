// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! function-kcl — deterministic stub evaluator.
//!
//! Upstream: function-kcl/kcl.go (full KCL bytecode interpreter)
//!
//! Charter scope cut: full KCL evaluator is delegated to a future `cave-kcl`
//! crate. This stub interprets a tiny safe subset:
//!   - `key = "string"` / `key = 12` / `key = true` literal assignments
//!   - `_observed.<path>` references resolved against the observed input
//!   - Returns a JSON object collecting all assignments
//! That covers the common Crossplane usage pattern of stamping a composed
//! resource manifest from observed XR state without inviting RCE.

use crate::error::{CrossplaneError, CrossplaneResult};
use serde_json::{Map, Value};

pub fn evaluate_kcl(source: &str, observed: &Value) -> CrossplaneResult<Value> {
    let mut out = Map::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // key = value
        let Some(eq) = line.find('=') else {
            continue;
        };
        let key = line[..eq].trim();
        let val_raw = line[eq + 1..].trim();
        if key.is_empty() {
            return Err(CrossplaneError::Internal(format!(
                "kcl: empty key in line: {}",
                line
            )));
        }
        let val = parse_value(val_raw, observed)?;
        out.insert(key.to_string(), val);
    }
    Ok(Value::Object(out))
}

fn parse_value(raw: &str, observed: &Value) -> CrossplaneResult<Value> {
    // string literal
    if (raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2)
        || (raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2)
    {
        return Ok(Value::String(raw[1..raw.len() - 1].to_string()));
    }
    // boolean
    if raw == "true" {
        return Ok(Value::Bool(true));
    }
    if raw == "false" {
        return Ok(Value::Bool(false));
    }
    if raw == "null" || raw == "None" {
        return Ok(Value::Null);
    }
    // observed reference
    if let Some(path) = raw.strip_prefix("_observed.") {
        return Ok(resolve_path(observed, path));
    }
    // integer
    if let Ok(i) = raw.parse::<i64>() {
        return Ok(serde_json::json!(i));
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Ok(serde_json::json!(f));
    }
    // fallback: treat as string
    Ok(Value::String(raw.to_string()))
}

fn resolve_path(obj: &Value, path: &str) -> Value {
    let mut cur = obj;
    for seg in path.split('.') {
        match cur {
            Value::Object(m) => match m.get(seg) {
                Some(v) => cur = v,
                None => return Value::Null,
            },
            _ => return Value::Null,
        }
    }
    cur.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_source_empty_output() {
        let v = evaluate_kcl("", &json!({})).unwrap();
        assert_eq!(v, json!({}));
    }

    #[test]
    fn string_literal() {
        let v = evaluate_kcl(r#"name = "alice""#, &json!({})).unwrap();
        assert_eq!(v["name"], json!("alice"));
    }

    #[test]
    fn integer_literal() {
        let v = evaluate_kcl("size = 10", &json!({})).unwrap();
        assert_eq!(v["size"], json!(10));
    }

    #[test]
    fn boolean_literal() {
        let v = evaluate_kcl("active = true", &json!({})).unwrap();
        assert_eq!(v["active"], json!(true));
    }

    #[test]
    fn null_literal() {
        let v = evaluate_kcl("x = null", &json!({})).unwrap();
        assert_eq!(v["x"], json!(null));
    }

    #[test]
    fn observed_reference() {
        let v = evaluate_kcl("name = _observed.spec.name", &json!({"spec":{"name":"db1"}}))
            .unwrap();
        assert_eq!(v["name"], json!("db1"));
    }

    #[test]
    fn observed_missing_path_returns_null() {
        let v = evaluate_kcl("name = _observed.x.y", &json!({})).unwrap();
        assert_eq!(v["name"], json!(null));
    }

    #[test]
    fn comment_lines_skipped() {
        let v = evaluate_kcl("# this is a comment\nx = 1", &json!({})).unwrap();
        assert_eq!(v["x"], json!(1));
        assert_eq!(v.as_object().unwrap().len(), 1);
    }

    #[test]
    fn empty_key_errors() {
        assert!(evaluate_kcl(" = 1", &json!({})).is_err());
    }

    #[test]
    fn float_literal() {
        let v = evaluate_kcl("ratio = 0.5", &json!({})).unwrap();
        assert!(v["ratio"].as_f64().unwrap() > 0.4);
    }

    #[test]
    fn multiple_assignments() {
        let v = evaluate_kcl(
            r#"a = "x"
b = 2
c = true"#,
            &json!({}),
        )
        .unwrap();
        assert_eq!(v.as_object().unwrap().len(), 3);
    }

    #[test]
    fn fallback_string() {
        let v = evaluate_kcl("x = some-thing", &json!({})).unwrap();
        assert_eq!(v["x"], json!("some-thing"));
    }
}
