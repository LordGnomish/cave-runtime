// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Runtime value type for Rego evaluation.

use std::collections::HashMap;

/// Runtime value produced by Rego evaluation.
#[derive(Debug, Clone)]
pub enum Value {
    /// JSON-compatible scalar/compound value.
    Json(serde_json::Value),
    /// Rego set (not representable in plain JSON).
    Set(Vec<serde_json::Value>),
    /// Undefined — rule/ref did not produce a value.
    Undefined,
}

impl Value {
    pub fn null() -> Self {
        Value::Json(serde_json::Value::Null)
    }
    pub fn bool(b: bool) -> Self {
        Value::Json(serde_json::Value::Bool(b))
    }
    pub fn number_f64(f: f64) -> Self {
        Value::Json(serde_json::json!(f))
    }
    pub fn number_i64(i: i64) -> Self {
        Value::Json(serde_json::json!(i))
    }
    pub fn string(s: impl Into<String>) -> Self {
        Value::Json(serde_json::Value::String(s.into()))
    }
    pub fn array(items: Vec<serde_json::Value>) -> Self {
        Value::Json(serde_json::Value::Array(items))
    }
    pub fn object(m: serde_json::Map<String, serde_json::Value>) -> Self {
        Value::Json(serde_json::Value::Object(m))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Undefined => false,
            Value::Json(v) => match v {
                serde_json::Value::Bool(false) => false,
                serde_json::Value::Null => false,
                _ => true,
            },
            Value::Set(_) => true,
        }
    }

    pub fn as_json(&self) -> Option<&serde_json::Value> {
        match self {
            Value::Json(v) => Some(v),
            _ => None,
        }
    }

    pub fn into_json(self) -> Option<serde_json::Value> {
        match self {
            Value::Json(v) => Some(v),
            Value::Set(items) => Some(serde_json::Value::Array(items)),
            Value::Undefined => None,
        }
    }

    pub fn to_json_lossy(&self) -> serde_json::Value {
        match self {
            Value::Json(v) => v.clone(),
            Value::Set(items) => serde_json::Value::Array(items.clone()),
            Value::Undefined => serde_json::Value::Null,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Json(serde_json::Value::String(s)) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Json(serde_json::Value::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Json(v) => v.as_f64(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Json(v) => v.as_i64(),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<serde_json::Value>> {
        match self {
            Value::Json(serde_json::Value::Array(a)) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        match self {
            Value::Json(serde_json::Value::Object(m)) => Some(m),
            _ => None,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Json(a), Value::Json(b)) => a == b,
            (Value::Set(a), Value::Set(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                let mut sa = a.clone();
                let mut sb = b.clone();
                sa.sort_by(json_cmp);
                sb.sort_by(json_cmp);
                sa == sb
            }
            (Value::Undefined, Value::Undefined) => true,
            _ => false,
        }
    }
}

/// Total ordering for JSON values (for set comparison/sorting).
pub fn json_cmp(a: &serde_json::Value, b: &serde_json::Value) -> std::cmp::Ordering {
    use serde_json::Value as J;
    use std::cmp::Ordering;
    match (a, b) {
        (J::Null, J::Null) => Ordering::Equal,
        (J::Null, _) => Ordering::Less,
        (_, J::Null) => Ordering::Greater,
        (J::Bool(x), J::Bool(y)) => x.cmp(y),
        (J::Bool(_), _) => Ordering::Less,
        (_, J::Bool(_)) => Ordering::Greater,
        (J::Number(x), J::Number(y)) => {
            let xf = x.as_f64().unwrap_or(f64::NAN);
            let yf = y.as_f64().unwrap_or(f64::NAN);
            xf.partial_cmp(&yf).unwrap_or(Ordering::Equal)
        }
        (J::Number(_), _) => Ordering::Less,
        (_, J::Number(_)) => Ordering::Greater,
        (J::String(x), J::String(y)) => x.cmp(y),
        (J::String(_), _) => Ordering::Less,
        (_, J::String(_)) => Ordering::Greater,
        (J::Array(x), J::Array(y)) => {
            for (xi, yi) in x.iter().zip(y.iter()) {
                let c = json_cmp(xi, yi);
                if c != Ordering::Equal {
                    return c;
                }
            }
            x.len().cmp(&y.len())
        }
        (J::Array(_), _) => Ordering::Less,
        (_, J::Array(_)) => Ordering::Greater,
        (J::Object(x), J::Object(y)) => {
            // Compare serialized for simplicity
            let xs = serde_json::to_string(x).unwrap_or_default();
            let ys = serde_json::to_string(y).unwrap_or_default();
            xs.cmp(&ys)
        }
    }
}

/// Variable bindings used during evaluation.
pub type Bindings = HashMap<String, Value>;

/// Helper to look up a nested path in a JSON value.
pub fn json_get_path<'a>(
    mut v: &'a serde_json::Value,
    path: &[String],
) -> Option<&'a serde_json::Value> {
    for key in path {
        v = v.get(key)?;
    }
    Some(v)
}

/// Apply a JSON Patch (RFC 6902) operation to a JSON value.
pub fn apply_json_patch(
    target: &mut serde_json::Value,
    op: &str,
    path: &str,
    value: Option<&serde_json::Value>,
    from: Option<&str>,
) -> Result<(), String> {
    let parts: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    match op {
        "add" => {
            let val = value.ok_or("add requires value")?.clone();
            json_set(target, &parts, val)?;
        }
        "remove" => {
            json_remove(target, &parts)?;
        }
        "replace" => {
            let val = value.ok_or("replace requires value")?.clone();
            json_set(target, &parts, val)?;
        }
        "move" => {
            let from_path = from.ok_or("move requires from")?;
            let from_parts: Vec<&str> = from_path
                .trim_start_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            let moved = json_get(target, &from_parts)
                .ok_or("move: source not found")?
                .clone();
            json_remove(target, &from_parts)?;
            json_set(target, &parts, moved)?;
        }
        "copy" => {
            let from_path = from.ok_or("copy requires from")?;
            let from_parts: Vec<&str> = from_path
                .trim_start_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            let copied = json_get(target, &from_parts)
                .ok_or("copy: source not found")?
                .clone();
            json_set(target, &parts, copied)?;
        }
        "test" => {
            let expected = value.ok_or("test requires value")?;
            let actual = json_get(target, &parts).ok_or("test: path not found")?;
            if actual != expected {
                return Err(format!("test failed at {path}: {actual} != {expected}"));
            }
        }
        other => return Err(format!("unknown patch op: {other}")),
    }
    Ok(())
}

fn json_get<'a>(v: &'a serde_json::Value, parts: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cur = v;
    for part in parts {
        cur = cur.get(part)?;
    }
    Some(cur)
}

fn json_set(
    v: &mut serde_json::Value,
    parts: &[&str],
    val: serde_json::Value,
) -> Result<(), String> {
    if parts.is_empty() {
        *v = val;
        return Ok(());
    }
    let (head, tail) = parts.split_at(1);
    let key = head[0];
    match v {
        serde_json::Value::Object(m) => {
            if tail.is_empty() {
                m.insert(key.to_string(), val);
            } else {
                let entry = m
                    .entry(key.to_string())
                    .or_insert(serde_json::Value::Object(Default::default()));
                json_set(entry, tail, val)?;
            }
        }
        serde_json::Value::Array(arr) => {
            let idx: usize = if key == "-" {
                arr.len()
            } else {
                key.parse()
                    .map_err(|_| format!("invalid array index: {key}"))?
            };
            if tail.is_empty() {
                if idx == arr.len() {
                    arr.push(val);
                } else if idx < arr.len() {
                    arr[idx] = val;
                } else {
                    return Err(format!("array index out of bounds: {idx}"));
                }
            } else if idx < arr.len() {
                json_set(&mut arr[idx], tail, val)?;
            } else {
                return Err(format!("array index out of bounds: {idx}"));
            }
        }
        _ => return Err(format!("cannot set on non-object/array at {key}")),
    }
    Ok(())
}

fn json_remove(v: &mut serde_json::Value, parts: &[&str]) -> Result<(), String> {
    if parts.is_empty() {
        return Err("cannot remove root".into());
    }
    let (head, tail) = parts.split_at(parts.len() - 1);
    let last = parts.last().unwrap();
    let parent = if head.is_empty() {
        v
    } else {
        json_get_mut(v, head).ok_or("remove: path not found")?
    };
    match parent {
        serde_json::Value::Object(m) => {
            m.remove(*last);
        }
        serde_json::Value::Array(arr) => {
            let idx: usize = last.parse().map_err(|_| format!("invalid index: {last}"))?;
            if idx < arr.len() {
                arr.remove(idx);
            }
        }
        _ => return Err("cannot remove from non-object/array".into()),
    }
    Ok(())
}

fn json_get_mut<'a>(
    v: &'a mut serde_json::Value,
    parts: &[&str],
) -> Option<&'a mut serde_json::Value> {
    let mut cur = v;
    for part in parts {
        cur = cur.get_mut(*part)?;
    }
    Some(cur)
}

/// Set a nested value in a JSON document by path (used by PolicyEngine::set_data).
pub fn set_nested_data(target: &mut serde_json::Value, path: &[String], value: serde_json::Value) {
    if path.is_empty() {
        *target = value;
        return;
    }
    if !target.is_object() {
        *target = serde_json::Value::Object(Default::default());
    }
    if let serde_json::Value::Object(m) = target {
        if path.len() == 1 {
            m.insert(path[0].clone(), value);
        } else {
            let entry = m
                .entry(path[0].clone())
                .or_insert(serde_json::Value::Object(Default::default()));
            set_nested_data(entry, &path[1..], value);
        }
    }
}
