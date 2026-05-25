// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! openAPIV3Schema validator — type / required / minLength / maxLength /
//! pattern / minimum / maximum / enum.
//!
//! Upstream: internal/xcrd/schemas.go

use crate::error::{CrossplaneError, CrossplaneResult};
use regex::Regex;
use serde_json::Value;

/// Validate `value` against `schema`. Returns the first error encountered.
pub fn validate_spec(schema: &Value, value: &Value) -> CrossplaneResult<()> {
    validate_at(schema, value, "$")
}

fn validate_at(schema: &Value, value: &Value, path: &str) -> CrossplaneResult<()> {
    // type
    if let Some(t) = schema.get("type").and_then(|v| v.as_str()) {
        if !type_matches(t, value) {
            return Err(CrossplaneError::XrdValidation(format!(
                "{}: expected type {} but got {}",
                path,
                t,
                value_kind(value)
            )));
        }
    }

    match value {
        Value::Object(o) => {
            // required
            if let Some(req) = schema.get("required").and_then(|v| v.as_array()) {
                for r in req {
                    if let Some(name) = r.as_str() {
                        if !o.contains_key(name) {
                            return Err(CrossplaneError::XrdValidation(format!(
                                "{}: required field '{}' missing",
                                path, name
                            )));
                        }
                    }
                }
            }
            // properties
            if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
                for (k, sub_schema) in props {
                    if let Some(sub_value) = o.get(k) {
                        validate_at(sub_schema, sub_value, &format!("{}.{}", path, k))?;
                    }
                }
            }
        }
        Value::String(s) => {
            if let Some(min) = schema.get("minLength").and_then(|v| v.as_u64()) {
                if (s.chars().count() as u64) < min {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: minLength {} not met (got {})",
                        path,
                        min,
                        s.chars().count()
                    )));
                }
            }
            if let Some(max) = schema.get("maxLength").and_then(|v| v.as_u64()) {
                if (s.chars().count() as u64) > max {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: maxLength {} exceeded (got {})",
                        path,
                        max,
                        s.chars().count()
                    )));
                }
            }
            if let Some(pat) = schema.get("pattern").and_then(|v| v.as_str()) {
                let re = Regex::new(pat).map_err(|e| {
                    CrossplaneError::XrdValidation(format!(
                        "{}: invalid pattern {}: {}",
                        path, pat, e
                    ))
                })?;
                if !re.is_match(s) {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: value '{}' does not match pattern '{}'",
                        path, s, pat
                    )));
                }
            }
            // enum
            if let Some(en) = schema.get("enum").and_then(|v| v.as_array()) {
                if !en.iter().any(|e| e.as_str() == Some(s.as_str())) {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: value '{}' not in enum",
                        path, s
                    )));
                }
            }
        }
        Value::Number(n) => {
            if let Some(min) = schema.get("minimum").and_then(|v| v.as_f64()) {
                if n.as_f64().unwrap_or(0.0) < min {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: minimum {} not met",
                        path, min
                    )));
                }
            }
            if let Some(max) = schema.get("maximum").and_then(|v| v.as_f64()) {
                if n.as_f64().unwrap_or(0.0) > max {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: maximum {} exceeded",
                        path, max
                    )));
                }
            }
        }
        Value::Array(arr) => {
            if let Some(items_schema) = schema.get("items") {
                for (i, item) in arr.iter().enumerate() {
                    validate_at(items_schema, item, &format!("{}[{}]", path, i))?;
                }
            }
            if let Some(min) = schema.get("minItems").and_then(|v| v.as_u64()) {
                if (arr.len() as u64) < min {
                    return Err(CrossplaneError::XrdValidation(format!(
                        "{}: minItems {} not met",
                        path, min
                    )));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn type_matches(t: &str, value: &Value) -> bool {
    match t {
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some()
            || value
                .as_f64()
                .map(|f| f.fract() == 0.0)
                .unwrap_or(false),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn value_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn type_string_ok() {
        assert!(validate_spec(&json!({"type":"string"}), &json!("x")).is_ok());
    }

    #[test]
    fn type_mismatch_errors() {
        assert!(validate_spec(&json!({"type":"string"}), &json!(1)).is_err());
    }

    #[test]
    fn required_present_ok() {
        let s = json!({"type":"object","required":["a"],"properties":{"a":{"type":"string"}}});
        assert!(validate_spec(&s, &json!({"a":"hi"})).is_ok());
    }

    #[test]
    fn required_absent_errors() {
        let s = json!({"type":"object","required":["a"]});
        assert!(validate_spec(&s, &json!({})).is_err());
    }

    #[test]
    fn min_length() {
        let s = json!({"type":"string","minLength":3});
        assert!(validate_spec(&s, &json!("hi")).is_err());
        assert!(validate_spec(&s, &json!("hii")).is_ok());
    }

    #[test]
    fn max_length() {
        let s = json!({"type":"string","maxLength":2});
        assert!(validate_spec(&s, &json!("hi")).is_ok());
        assert!(validate_spec(&s, &json!("hii")).is_err());
    }

    #[test]
    fn pattern_match() {
        let s = json!({"type":"string","pattern":"^[a-z]+$"});
        assert!(validate_spec(&s, &json!("abc")).is_ok());
        assert!(validate_spec(&s, &json!("Abc")).is_err());
    }

    #[test]
    fn enum_check() {
        let s = json!({"type":"string","enum":["a","b","c"]});
        assert!(validate_spec(&s, &json!("a")).is_ok());
        assert!(validate_spec(&s, &json!("d")).is_err());
    }

    #[test]
    fn min_max_number() {
        let s = json!({"type":"number","minimum":0,"maximum":10});
        assert!(validate_spec(&s, &json!(5)).is_ok());
        assert!(validate_spec(&s, &json!(-1)).is_err());
        assert!(validate_spec(&s, &json!(11)).is_err());
    }

    #[test]
    fn nested_object() {
        let s = json!({
            "type":"object",
            "properties":{"db":{"type":"object","required":["size"],"properties":{"size":{"type":"integer"}}}}
        });
        assert!(validate_spec(&s, &json!({"db":{"size":10}})).is_ok());
        assert!(validate_spec(&s, &json!({"db":{}})).is_err());
    }

    #[test]
    fn array_items() {
        let s = json!({"type":"array","items":{"type":"integer"},"minItems":1});
        assert!(validate_spec(&s, &json!([1,2,3])).is_ok());
        assert!(validate_spec(&s, &json!([])).is_err());
        assert!(validate_spec(&s, &json!(["a"])).is_err());
    }

    #[test]
    fn invalid_pattern_errors() {
        let s = json!({"type":"string","pattern":"["});
        assert!(validate_spec(&s, &json!("x")).is_err());
    }

    #[test]
    fn boolean_type() {
        assert!(validate_spec(&json!({"type":"boolean"}), &json!(true)).is_ok());
        assert!(validate_spec(&json!({"type":"boolean"}), &json!("true")).is_err());
    }
}
