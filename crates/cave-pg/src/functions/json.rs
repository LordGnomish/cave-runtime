//! PostgreSQL JSON/JSONB functions.

use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{oid, PgValue};
use serde_json::{json, Value as JV};

fn to_jv(v: &PgValue) -> JV {
    match v {
        PgValue::Null => JV::Null,
        PgValue::Bool(b) => JV::Bool(*b),
        PgValue::Int2(n) => json!(n),
        PgValue::Int4(n) => json!(n),
        PgValue::Int8(n) => json!(n),
        PgValue::Float4(n) => json!(n),
        PgValue::Float8(n) => json!(n),
        PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => JV::String(s.clone()),
        PgValue::Json(j) | PgValue::Jsonb(j) => j.clone(),
        PgValue::Array { elements, .. } => {
            JV::Array(elements.iter().map(to_jv).collect())
        }
        other => JV::String(other.to_text()),
    }
}

fn from_jv(j: JV) -> PgValue {
    match j {
        JV::Null => PgValue::Null,
        JV::Bool(b) => PgValue::Bool(b),
        JV::Number(n) => {
            if let Some(i) = n.as_i64() { PgValue::Int8(i) }
            else { PgValue::Float8(n.as_f64().unwrap_or(0.0)) }
        }
        JV::String(s) => PgValue::Text(s),
        JV::Array(arr) => PgValue::Array {
            element_oid: oid::JSONB,
            elements: arr.into_iter().map(|v| PgValue::Jsonb(v)).collect(),
        },
        JV::Object(_) => PgValue::Jsonb(j),
    }
}

pub fn build_object(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() % 2 != 0 {
        return Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
            "argument list must have even number of elements")));
    }
    let mut map = serde_json::Map::new();
    let mut i = 0;
    while i + 1 < args.len() {
        let key = args[i].to_text();
        let val = to_jv(&args[i + 1]);
        map.insert(key, val);
        i += 2;
    }
    Ok(PgValue::Jsonb(JV::Object(map)))
}

pub fn build_array(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Jsonb(JV::Array(args.iter().map(to_jv).collect())))
}

pub fn json_object(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() { return Ok(PgValue::Jsonb(JV::Object(serde_json::Map::new()))); }
    match &args[0] {
        PgValue::Array { elements, .. } => {
            if args.len() == 2 {
                // json_object(keys, values)
                if let PgValue::Array { elements: vals, .. } = &args[1] {
                    let mut map = serde_json::Map::new();
                    for (k, v) in elements.iter().zip(vals.iter()) {
                        map.insert(k.to_text(), to_jv(v));
                    }
                    return Ok(PgValue::Jsonb(JV::Object(map)));
                }
            }
            // Single array of alternating key/value
            let mut map = serde_json::Map::new();
            let mut i = 0;
            while i + 1 < elements.len() {
                map.insert(elements[i].to_text(), to_jv(&elements[i + 1]));
                i += 2;
            }
            Ok(PgValue::Jsonb(JV::Object(map)))
        }
        _ => Ok(PgValue::Jsonb(JV::Object(serde_json::Map::new()))),
    }
}

pub fn array_length(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Json(j) | PgValue::Jsonb(j) => {
            match j {
                JV::Array(a) => Ok(PgValue::Int4(a.len() as i32)),
                _ => Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
                    "cannot get array length of a non-array"))),
            }
        }
        _ => Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "json_array_length requires json"))),
    }
}

pub fn typeof_fn(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Json(j) | PgValue::Jsonb(j) => {
            let t = match j {
                JV::Null => "null",
                JV::Bool(_) => "boolean",
                JV::Number(_) => "number",
                JV::String(_) => "string",
                JV::Array(_) => "array",
                JV::Object(_) => "object",
            };
            Ok(PgValue::Text(t.to_string()))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "json_typeof requires json"))),
    }
}

pub fn strip_nulls(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    fn strip(j: JV) -> JV {
        match j {
            JV::Object(mut map) => {
                map.retain(|_, v| !v.is_null());
                let stripped: serde_json::Map<_, _> = map.into_iter().map(|(k, v)| (k, strip(v))).collect();
                JV::Object(stripped)
            }
            JV::Array(arr) => JV::Array(arr.into_iter().map(strip).collect()),
            other => other,
        }
    }
    match args[0].clone() {
        PgValue::Json(j) => Ok(PgValue::Json(strip(j))),
        PgValue::Jsonb(j) => Ok(PgValue::Jsonb(strip(j))),
        _ => Ok(args[0].clone()),
    }
}

pub fn extract_path(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let j = match &args[0] {
        PgValue::Json(j) | PgValue::Jsonb(j) => j.clone(),
        _ => return Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "jsonb_extract_path requires jsonb"))),
    };
    let mut current = j;
    for path_elem in &args[1..] {
        let key = path_elem.to_text();
        current = match current {
            JV::Object(ref map) => map.get(&key).cloned().unwrap_or(JV::Null),
            JV::Array(ref arr) => {
                if let Ok(idx) = key.parse::<usize>() {
                    arr.get(idx).cloned().unwrap_or(JV::Null)
                } else {
                    JV::Null
                }
            }
            _ => JV::Null,
        };
    }
    if current.is_null() {
        Ok(PgValue::Null)
    } else {
        Ok(PgValue::Jsonb(current))
    }
}

pub fn extract_path_text(args: Vec<PgValue>) -> Result<PgValue> {
    extract_path(args).map(|v| match v {
        PgValue::Jsonb(JV::String(s)) | PgValue::Json(JV::String(s)) => PgValue::Text(s),
        PgValue::Jsonb(j) => PgValue::Text(j.to_string()),
        PgValue::Json(j) => PgValue::Text(j.to_string()),
        other => other,
    })
}

pub fn object_keys(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Json(JV::Object(m)) | PgValue::Jsonb(JV::Object(m)) => {
            Ok(PgValue::Array {
                element_oid: oid::TEXT,
                elements: m.keys().map(|k| PgValue::Text(k.clone())).collect(),
            })
        }
        _ => Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE, "json_object_keys requires a json object"))),
    }
}

pub fn each(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    // Returns array of (key, value) records
    match &args[0] {
        PgValue::Json(JV::Object(m)) | PgValue::Jsonb(JV::Object(m)) => {
            let rows: Vec<PgValue> = m.iter().map(|(k, v)| {
                PgValue::Record(vec![PgValue::Text(k.clone()), PgValue::Jsonb(v.clone())])
            }).collect();
            Ok(PgValue::Array { element_oid: oid::RECORD, elements: rows })
        }
        _ => Ok(PgValue::Array { element_oid: oid::RECORD, elements: vec![] }),
    }
}

pub fn each_text(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Json(JV::Object(m)) | PgValue::Jsonb(JV::Object(m)) => {
            let rows: Vec<PgValue> = m.iter().map(|(k, v)| {
                PgValue::Record(vec![
                    PgValue::Text(k.clone()),
                    PgValue::Text(match v { JV::String(s) => s.clone(), other => other.to_string() }),
                ])
            }).collect();
            Ok(PgValue::Array { element_oid: oid::RECORD, elements: rows })
        }
        _ => Ok(PgValue::Array { element_oid: oid::RECORD, elements: vec![] }),
    }
}

pub fn to_record(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(args[0].clone()) // Placeholder
}

pub fn jsonb_set(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let mut j = match &args[0] {
        PgValue::Jsonb(j) => j.clone(),
        _ => return Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "jsonb_set requires jsonb"))),
    };
    let path = match &args[1] {
        PgValue::Array { elements, .. } => elements.iter().map(|e| e.to_text()).collect::<Vec<_>>(),
        _ => return Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "jsonb_set path must be text[]"))),
    };
    let new_val = to_jv(&args[2]);
    let create_missing = args.get(3).map(|v| v.is_true()).unwrap_or(true);

    if !path.is_empty() {
        json_set_path(&mut j, &path, new_val, create_missing);
    }
    Ok(PgValue::Jsonb(j))
}

fn json_set_path(j: &mut JV, path: &[String], val: JV, create: bool) {
    if path.is_empty() { return; }
    if path.len() == 1 {
        match j {
            JV::Object(map) => { map.insert(path[0].clone(), val); }
            JV::Array(arr) => {
                if let Ok(idx) = path[0].parse::<usize>() {
                    if idx < arr.len() { arr[idx] = val; }
                    else if create { arr.push(val); }
                }
            }
            _ => {}
        }
        return;
    }
    match j {
        JV::Object(map) => {
            if let Some(child) = map.get_mut(&path[0]) {
                json_set_path(child, &path[1..], val, create);
            } else if create {
                let mut child = JV::Object(serde_json::Map::new());
                json_set_path(&mut child, &path[1..], val, create);
                map.insert(path[0].clone(), child);
            }
        }
        _ => {}
    }
}

pub fn jsonb_insert(args: Vec<PgValue>) -> Result<PgValue> {
    jsonb_set(args) // Simplified — full implementation would handle insert semantics
}

pub fn jsonb_pretty(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Json(j) | PgValue::Jsonb(j) => {
            Ok(PgValue::Text(serde_json::to_string_pretty(j)
                .unwrap_or_else(|_| j.to_string())))
        }
        _ => Ok(PgValue::Text(args[0].to_text())),
    }
}

pub fn jsonb_path_query(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    // Simplified jsonpath: just handle basic path expressions
    let j = match &args[0] {
        PgValue::Json(j) | PgValue::Jsonb(j) => j.clone(),
        _ => return Ok(PgValue::Null),
    };
    let path = args[1].to_text();
    // Simple dotted path: $.foo.bar
    let result = eval_jsonpath(&j, &path);
    Ok(PgValue::Jsonb(result))
}

pub fn jsonb_path_exists(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let j = match &args[0] {
        PgValue::Json(j) | PgValue::Jsonb(j) => j.clone(),
        _ => return Ok(PgValue::Bool(false)),
    };
    let path = args[1].to_text();
    let result = eval_jsonpath(&j, &path);
    Ok(PgValue::Bool(!result.is_null()))
}

fn eval_jsonpath(j: &JV, path: &str) -> JV {
    // Very basic jsonpath: $.key or $.key.subkey or $[0]
    let path = path.trim();
    if path == "$" { return j.clone(); }
    let path = path.strip_prefix("$.").or_else(|| path.strip_prefix("$")).unwrap_or(path);
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = j.clone();
    for part in parts {
        let part = part.trim();
        if part.is_empty() { continue; }
        // Handle array subscript: key[n]
        if let Some(bracket_pos) = part.find('[') {
            let key = &part[..bracket_pos];
            let idx_str = &part[bracket_pos + 1..part.len().saturating_sub(1)];
            if !key.is_empty() {
                current = match &current {
                    JV::Object(map) => map.get(key).cloned().unwrap_or(JV::Null),
                    _ => JV::Null,
                };
            }
            if let Ok(idx) = idx_str.parse::<usize>() {
                current = match &current {
                    JV::Array(arr) => arr.get(idx).cloned().unwrap_or(JV::Null),
                    _ => JV::Null,
                };
            }
        } else {
            current = match &current {
                JV::Object(map) => map.get(part).cloned().unwrap_or(JV::Null),
                JV::Array(arr) => {
                    if let Ok(idx) = part.parse::<usize>() {
                        arr.get(idx).cloned().unwrap_or(JV::Null)
                    } else {
                        JV::Null
                    }
                }
                _ => JV::Null,
            };
        }
    }
    current
}

pub fn row_to_json(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Json(to_jv(&args[0])))
}

pub fn to_json(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Jsonb(to_jv(&args[0])))
}

pub fn array_to_json(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Json(to_jv(&args[0])))
}

/// JSONB containment operator @>
pub fn jsonb_contains(left: &PgValue, right: &PgValue) -> bool {
    let l = match left { PgValue::Jsonb(j) | PgValue::Json(j) => j, _ => return false };
    let r = match right { PgValue::Jsonb(j) | PgValue::Json(j) => j, _ => return false };
    contains(l, r)
}

fn contains(haystack: &JV, needle: &JV) -> bool {
    match (haystack, needle) {
        (JV::Object(h), JV::Object(n)) => {
            n.iter().all(|(k, v)| h.get(k).map(|hv| contains(hv, v)).unwrap_or(false))
        }
        (JV::Array(h), JV::Array(n)) => {
            n.iter().all(|nv| h.iter().any(|hv| contains(hv, nv)))
        }
        (JV::Array(h), nv) => h.iter().any(|hv| hv == nv),
        (h, n) => h == n,
    }
}

/// JSONB key exists operator ?
pub fn jsonb_key_exists(j: &PgValue, key: &str) -> bool {
    match j {
        PgValue::Jsonb(JV::Object(map)) | PgValue::Json(JV::Object(map)) => map.contains_key(key),
        _ => false,
    }
}
