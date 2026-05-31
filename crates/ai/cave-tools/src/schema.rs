// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! A self-contained JSON Schema validator, scoped to the subset that tool
//! `inputSchema` objects use in practice (MCP / OpenAI function calling /
//! LangChain all express parameters with the same Draft-2020-12 subset).
//!
//! Supported keywords:
//! * `type` (single or array; `integer` distinguished from `number`)
//! * `properties`, `required`, `additionalProperties` (bool)
//! * `enum`
//! * `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`
//! * `minLength`, `maxLength`, `pattern`
//! * `items`, `minItems`, `maxItems`
//!
//! Validation collects *all* violations (not fail-fast) so callers can show
//! the model every problem at once. Keywords outside the subset are ignored
//! (permissive), matching how lightweight validators behave on tool args.

use serde_json::Value;

use crate::error::{Result, ToolError};

/// List of human-readable validation messages. Empty == valid.
pub type ValidationErrors = Vec<String>;

/// Validate `instance` against `schema`. Returns `Ok(())` when valid, or the
/// collected list of violations.
pub fn validate(schema: &Value, instance: &Value) -> std::result::Result<(), ValidationErrors> {
    let mut errs = Vec::new();
    validate_at(schema, instance, "$", &mut errs);
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Validate tool `args` against a tool's schema, mapping any failure onto a
/// [`ToolError::InvalidArguments`] addressed to `tool`.
pub fn validate_args(tool: &str, schema: &Value, args: &Value) -> Result<()> {
    validate(schema, args).map_err(|errs| ToolError::InvalidArguments {
        tool: tool.to_string(),
        reason: errs.join("; "),
    })
}

fn validate_at(schema: &Value, inst: &Value, path: &str, errs: &mut ValidationErrors) {
    let Some(obj) = schema.as_object() else {
        // A non-object schema (e.g. `true`) accepts anything.
        return;
    };

    // ── type ────────────────────────────────────────────────────────────
    if let Some(t) = obj.get("type") {
        check_type(t, inst, path, errs);
    }

    // ── enum ────────────────────────────────────────────────────────────
    if let Some(Value::Array(choices)) = obj.get("enum")
        && !choices.iter().any(|c| c == inst)
    {
        errs.push(format!("{path}: value not in enum {choices:?}"));
    }

    // ── numeric constraints ──────────────────────────────────────────────
    if let Some(n) = inst.as_f64() {
        if let Some(min) = obj.get("minimum").and_then(Value::as_f64)
            && n < min
        {
            errs.push(format!("{path}: {n} < minimum {min}"));
        }
        if let Some(max) = obj.get("maximum").and_then(Value::as_f64)
            && n > max
        {
            errs.push(format!("{path}: {n} > maximum {max}"));
        }
        if let Some(emin) = obj.get("exclusiveMinimum").and_then(Value::as_f64)
            && n <= emin
        {
            errs.push(format!("{path}: {n} <= exclusiveMinimum {emin}"));
        }
        if let Some(emax) = obj.get("exclusiveMaximum").and_then(Value::as_f64)
            && n >= emax
        {
            errs.push(format!("{path}: {n} >= exclusiveMaximum {emax}"));
        }
    }

    // ── string constraints ───────────────────────────────────────────────
    if let Some(s) = inst.as_str() {
        let len = s.chars().count() as u64;
        if let Some(min) = obj.get("minLength").and_then(Value::as_u64)
            && len < min
        {
            errs.push(format!("{path}: length {len} < minLength {min}"));
        }
        if let Some(max) = obj.get("maxLength").and_then(Value::as_u64)
            && len > max
        {
            errs.push(format!("{path}: length {len} > maxLength {max}"));
        }
        if let Some(pat) = obj.get("pattern").and_then(Value::as_str) {
            match regex::Regex::new(pat) {
                Ok(re) if !re.is_match(s) => {
                    errs.push(format!("{path}: does not match pattern /{pat}/"));
                }
                Err(e) => errs.push(format!("{path}: invalid pattern /{pat}/: {e}")),
                _ => {}
            }
        }
    }

    // ── object constraints ───────────────────────────────────────────────
    if let Some(map) = inst.as_object() {
        if let Some(Value::Array(req)) = obj.get("required") {
            for r in req {
                if let Some(name) = r.as_str()
                    && !map.contains_key(name)
                {
                    errs.push(format!("{path}: missing required property `{name}`"));
                }
            }
        }
        let props = obj.get("properties").and_then(Value::as_object);
        if let Some(props) = props {
            for (k, sub_schema) in props {
                if let Some(v) = map.get(k) {
                    validate_at(sub_schema, v, &format!("{path}.{k}"), errs);
                }
            }
        }
        if obj.get("additionalProperties") == Some(&Value::Bool(false)) {
            let allowed = props;
            for k in map.keys() {
                let known = allowed.map(|p| p.contains_key(k)).unwrap_or(false);
                if !known {
                    errs.push(format!("{path}: additional property `{k}` not allowed"));
                }
            }
        }
    }

    // ── array constraints ────────────────────────────────────────────────
    if let Some(arr) = inst.as_array() {
        let n = arr.len() as u64;
        if let Some(min) = obj.get("minItems").and_then(Value::as_u64)
            && n < min
        {
            errs.push(format!("{path}: {n} items < minItems {min}"));
        }
        if let Some(max) = obj.get("maxItems").and_then(Value::as_u64)
            && n > max
        {
            errs.push(format!("{path}: {n} items > maxItems {max}"));
        }
        if let Some(items_schema) = obj.get("items") {
            for (i, v) in arr.iter().enumerate() {
                validate_at(items_schema, v, &format!("{path}[{i}]"), errs);
            }
        }
    }
}

fn check_type(t: &Value, inst: &Value, path: &str, errs: &mut ValidationErrors) {
    let types: Vec<&str> = match t {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return,
    };
    if !types.iter().any(|ty| type_matches(ty, inst)) {
        errs.push(format!(
            "{path}: expected type {}, got {}",
            types.join("|"),
            json_type_name(inst)
        ));
    }
}

fn type_matches(ty: &str, inst: &Value) -> bool {
    match ty {
        "object" => inst.is_object(),
        "array" => inst.is_array(),
        "string" => inst.is_string(),
        "boolean" => inst.is_boolean(),
        "null" => inst.is_null(),
        "number" => inst.is_number(),
        // JSON Schema integer: a number with no fractional part. serde_json
        // surfaces this directly for i64/u64-backed values; for f64-backed
        // values we additionally accept those with a zero fraction.
        "integer" => inst.is_i64() || inst.is_u64() || inst.as_f64().is_some_and(|f| f.fract() == 0.0),
        _ => true, // unknown type keyword: be permissive
    }
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
