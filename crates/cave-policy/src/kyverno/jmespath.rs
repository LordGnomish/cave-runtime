// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JMESPath evaluator — subset implementation for Kyverno variable substitution.
//!
//! Supports: field access, array index, array/object projections, pipe,
//! filter expressions, function calls (length, keys, values, contains, starts_with,
//! ends_with, to_string, to_number, to_array, type, sort, sort_by, reverse,
//! min, max, min_by, max_by, sum, avg, merge, zip, items, group_by, map, find_in, wildcard).

use crate::error::PolicyError;
use serde_json::Value;

/// Evaluate a JMESPath expression against a JSON value.
pub fn evaluate(expression: &str, value: &Value) -> Result<Value, PolicyError> {
    let expr = expression.trim();
    if expr.is_empty() || expr == "@" {
        return Ok(value.clone());
    }
    eval_expr(expr, value)
}

fn eval_expr(expr: &str, ctx: &Value) -> Result<Value, PolicyError> {
    let expr = expr.trim();

    // Pipe: `a | b`
    if let Some(pipe_pos) = find_pipe(expr) {
        let left_result = eval_expr(&expr[..pipe_pos], ctx)?;
        return eval_expr(&expr[pipe_pos + 1..], &left_result);
    }

    // Or expression: `a || b`
    if let Some(or_pos) = find_or(expr) {
        let left_result = eval_expr(&expr[..or_pos], ctx)?;
        if !is_falsy(&left_result) {
            return Ok(left_result);
        }
        return eval_expr(&expr[or_pos + 2..], ctx);
    }

    // And expression: `a && b`
    if let Some(and_pos) = find_and(expr) {
        let left_result = eval_expr(&expr[..and_pos], ctx)?;
        if is_falsy(&left_result) {
            return Ok(left_result);
        }
        return eval_expr(&expr[and_pos + 2..], ctx);
    }

    // Comparisons
    for op in &[">=", "<=", "!=", "==", ">", "<"] {
        if let Some(pos) = find_operator(expr, op) {
            let left = eval_expr(&expr[..pos], ctx)?;
            let right = eval_expr(&expr[pos + op.len()..], ctx)?;
            return eval_comparison(&left, &right, op);
        }
    }

    // Not expression: `!expr`
    if let Some(rest) = expr.strip_prefix('!') {
        let val = eval_expr(rest, ctx)?;
        return Ok(Value::Bool(is_falsy(&val)));
    }

    // Function call: `func(args)`
    if let Some(paren_pos) = find_function_call(expr) {
        let func_name = &expr[..paren_pos];
        let args_str = &expr[paren_pos + 1..expr.len() - 1];
        return eval_function(func_name, args_str, ctx);
    }

    // Multi-select list: `[expr, expr, ...]`
    if expr.starts_with('[') && !expr.starts_with("[?") && expr.ends_with(']') {
        let inner = &expr[1..expr.len() - 1];
        let items: Vec<&str> = split_top_level(inner, ',');
        let mut results = Vec::new();
        for item in items {
            results.push(eval_expr(item.trim(), ctx)?);
        }
        return Ok(Value::Array(results));
    }

    // Multi-select hash / object projection: `{key: expr, key: expr}`
    if expr.starts_with('{') && expr.ends_with('}') {
        let inner = &expr[1..expr.len() - 1];
        let pairs: Vec<&str> = split_top_level(inner, ',');
        let mut m = serde_json::Map::new();
        for pair in pairs {
            let colon = pair.find(':').ok_or_else(|| PolicyError::Eval(format!("invalid object projection: {pair}")))?;
            let key = pair[..colon].trim().trim_matches('"').to_string();
            let val = eval_expr(pair[colon + 1..].trim(), ctx)?;
            m.insert(key, val);
        }
        return Ok(Value::Object(m));
    }

    // Flatten operator: `[]`
    if let Some(rest) = expr.strip_suffix("[]") {
        let val = eval_expr(rest, ctx)?;
        return Ok(flatten_array(&val));
    }

    // Sub-expression chain: `a.b.c` or `a[0].b`
    if let Some((head, tail)) = split_first_segment(expr) {
        let head_val = eval_segment(head, ctx)?;
        if tail.is_empty() {
            return Ok(head_val);
        }
        return eval_expr(&tail, &head_val);
    }

    // Literal values
    if let Ok(v) = serde_json::from_str::<Value>(expr) {
        return Ok(v);
    }

    // Backtick literals: `foo`
    if expr.starts_with('`') && expr.ends_with('`') {
        let inner = &expr[1..expr.len() - 1];
        return serde_json::from_str(inner)
            .map_err(|e| PolicyError::Eval(format!("invalid literal: {e}")));
    }

    // Simple identifier / field access
    eval_segment(expr, ctx)
}

fn eval_segment(seg: &str, ctx: &Value) -> Result<Value, PolicyError> {
    let seg = seg.trim();

    // Wildcard: `*`
    if seg == "*" {
        return match ctx {
            Value::Object(m) => Ok(Value::Array(m.values().cloned().collect())),
            Value::Array(a) => Ok(Value::Array(a.clone())),
            _ => Ok(Value::Null),
        };
    }

    // Array index: `[0]` or `[-1]`
    if seg.starts_with('[') && seg.ends_with(']') {
        let inner = &seg[1..seg.len() - 1];

        // Filter projection: `[?expr]`
        if let Some(filter_expr) = inner.strip_prefix('?') {
            return eval_filter(filter_expr, ctx);
        }

        // Slice: `[start:end]` or `[start:end:step]`
        if inner.contains(':') {
            return eval_slice(inner, ctx);
        }

        // Numeric index
        let idx: i64 = inner.parse()
            .map_err(|_| PolicyError::Eval(format!("invalid index: {inner}")))?;
        return Ok(match ctx {
            Value::Array(a) => {
                let len = a.len() as i64;
                let actual = if idx < 0 { (len + idx).max(0) as usize } else { idx as usize };
                a.get(actual).cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        });
    }

    // Field access with quoted key: `"foo"`
    if seg.starts_with('"') && seg.ends_with('"') {
        let key = &seg[1..seg.len() - 1];
        return Ok(ctx.get(key).cloned().unwrap_or(Value::Null));
    }

    // Unquoted identifier
    Ok(ctx.get(seg).cloned().unwrap_or(Value::Null))
}

fn eval_filter(filter_expr: &str, ctx: &Value) -> Result<Value, PolicyError> {
    let items = match ctx {
        Value::Array(a) => a.clone(),
        _ => return Ok(Value::Null),
    };
    let mut results = Vec::new();
    for item in &items {
        let result = eval_expr(filter_expr, item)?;
        if !is_falsy(&result) {
            results.push(item.clone());
        }
    }
    Ok(Value::Array(results))
}

fn eval_slice(inner: &str, ctx: &Value) -> Result<Value, PolicyError> {
    let arr = match ctx {
        Value::Array(a) => a,
        _ => return Ok(Value::Null),
    };
    let parts: Vec<&str> = inner.split(':').collect();
    let len = arr.len() as i64;
    let parse_idx = |s: &str, default: i64| -> i64 {
        if s.is_empty() { default } else { s.parse().unwrap_or(default) }
    };
    let start = parse_idx(parts.first().copied().unwrap_or(""), 0);
    let end = parse_idx(parts.get(1).copied().unwrap_or(""), len);
    let step = parse_idx(parts.get(2).copied().unwrap_or(""), 1);
    let normalize = |i: i64| -> usize {
        (if i < 0 { (len + i).max(0) } else { i.min(len) }) as usize
    };
    let start = normalize(start);
    let end = normalize(end);
    if step == 0 {
        return Err(PolicyError::Eval("slice step cannot be zero".into()));
    }
    let mut result = Vec::new();
    if step > 0 {
        let mut i = start;
        while i < end {
            if let Some(v) = arr.get(i) { result.push(v.clone()); }
            i += step as usize;
        }
    } else {
        let mut i = (end as i64 - 1).max(0) as usize;
        while i >= start {
            if let Some(v) = arr.get(i) { result.push(v.clone()); }
            if i == 0 { break; }
            i = i.saturating_sub((-step) as usize);
        }
    }
    Ok(Value::Array(result))
}

fn eval_comparison(left: &Value, right: &Value, op: &str) -> Result<Value, PolicyError> {
    let cmp = |a: &Value, b: &Value| -> std::cmp::Ordering {
        crate::rego::value::json_cmp(a, b)
    };
    let result = match op {
        "==" => left == right,
        "!=" => left != right,
        ">" => cmp(left, right) == std::cmp::Ordering::Greater,
        "<" => cmp(left, right) == std::cmp::Ordering::Less,
        ">=" => cmp(left, right) != std::cmp::Ordering::Less,
        "<=" => cmp(left, right) != std::cmp::Ordering::Greater,
        _ => false,
    };
    Ok(Value::Bool(result))
}

fn eval_function(name: &str, args_str: &str, ctx: &Value) -> Result<Value, PolicyError> {
    let args: Vec<Value> = if args_str.trim().is_empty() {
        vec![]
    } else {
        split_top_level(args_str, ',')
            .into_iter()
            .map(|a| eval_expr(a.trim(), ctx))
            .collect::<Result<Vec<_>, _>>()?
    };

    match name.trim() {
        "length" => {
            let v = args.first().unwrap_or(&Value::Null);
            Ok(Value::Number(serde_json::Number::from(match v {
                Value::Array(a) => a.len(),
                Value::Object(m) => m.len(),
                Value::String(s) => s.len(),
                Value::Null => 0,
                _ => 0,
            })))
        }
        "keys" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Object(m) => Ok(Value::Array(m.keys().map(|k| Value::String(k.clone())).collect())),
                _ => Ok(Value::Null),
            }
        }
        "values" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Object(m) => Ok(Value::Array(m.values().cloned().collect())),
                _ => Ok(Value::Null),
            }
        }
        "contains" => {
            let subject = args.first().unwrap_or(&Value::Null);
            let search = args.get(1).unwrap_or(&Value::Null);
            match subject {
                Value::Array(a) => Ok(Value::Bool(a.contains(search))),
                Value::String(s) => {
                    if let Value::String(sub) = search {
                        Ok(Value::Bool(s.contains(sub.as_str())))
                    } else {
                        Ok(Value::Bool(false))
                    }
                }
                _ => Ok(Value::Bool(false)),
            }
        }
        "starts_with" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let prefix = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::Bool(s.starts_with(prefix)))
        }
        "ends_with" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let suffix = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::Bool(s.ends_with(suffix)))
        }
        "to_string" => {
            let v = args.first().unwrap_or(&Value::Null);
            Ok(Value::String(match v {
                Value::String(s) => s.clone(),
                _ => serde_json::to_string(v).unwrap_or_default(),
            }))
        }
        "to_number" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Number(n) => Ok(Value::Number(n.clone())),
                Value::String(s) => {
                    if let Ok(i) = s.parse::<i64>() {
                        Ok(Value::Number(i.into()))
                    } else if let Ok(f) = s.parse::<f64>() {
                        Ok(serde_json::json!(f))
                    } else {
                        Ok(Value::Null)
                    }
                }
                _ => Ok(Value::Null),
            }
        }
        "to_array" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Array(_) => Ok(v.clone()),
                _ => Ok(Value::Array(vec![v.clone()])),
            }
        }
        "type" => {
            let v = args.first().unwrap_or(&Value::Null);
            Ok(Value::String(match v {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            }.into()))
        }
        "sort" => {
            let mut arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            arr.sort_by(crate::rego::value::json_cmp);
            Ok(Value::Array(arr))
        }
        "sort_by" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let key_expr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Ok(Value::Array(arr)),
            };
            let mut indexed: Vec<(Value, Value)> = arr.into_iter().map(|v| {
                let k = eval_expr(&key_expr, &v).unwrap_or(Value::Null);
                (k, v)
            }).collect();
            indexed.sort_by(|(a, _), (b, _)| crate::rego::value::json_cmp(a, b));
            Ok(Value::Array(indexed.into_iter().map(|(_, v)| v).collect()))
        }
        "reverse" => {
            let mut arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                Value::String(s) => return Ok(Value::String(s.chars().rev().collect())),
                _ => return Ok(Value::Null),
            };
            arr.reverse();
            Ok(Value::Array(arr))
        }
        "min" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) if !a.is_empty() => a,
                _ => return Ok(Value::Null),
            };
            Ok(arr.iter().min_by(|a, b| crate::rego::value::json_cmp(a, b)).cloned().unwrap_or(Value::Null))
        }
        "max" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) if !a.is_empty() => a,
                _ => return Ok(Value::Null),
            };
            Ok(arr.iter().max_by(|a, b| crate::rego::value::json_cmp(a, b)).cloned().unwrap_or(Value::Null))
        }
        "min_by" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) if !a.is_empty() => a.clone(),
                _ => return Ok(Value::Null),
            };
            let key_expr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Ok(Value::Null),
            };
            arr.into_iter().min_by(|a, b| {
                let ka = eval_expr(&key_expr, a).unwrap_or(Value::Null);
                let kb = eval_expr(&key_expr, b).unwrap_or(Value::Null);
                crate::rego::value::json_cmp(&ka, &kb)
            }).map(Ok).unwrap_or(Ok(Value::Null))
        }
        "max_by" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) if !a.is_empty() => a.clone(),
                _ => return Ok(Value::Null),
            };
            let key_expr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Ok(Value::Null),
            };
            arr.into_iter().max_by(|a, b| {
                let ka = eval_expr(&key_expr, a).unwrap_or(Value::Null);
                let kb = eval_expr(&key_expr, b).unwrap_or(Value::Null);
                crate::rego::value::json_cmp(&ka, &kb)
            }).map(Ok).unwrap_or(Ok(Value::Null))
        }
        "sum" | "avg" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
            if nums.is_empty() { return Ok(Value::Null); }
            let s: f64 = nums.iter().sum();
            let result = if name == "avg" { s / nums.len() as f64 } else { s };
            Ok(serde_json::json!(result))
        }
        "merge" => {
            let mut result = serde_json::Map::new();
            for arg in &args {
                if let Value::Object(m) = arg {
                    for (k, v) in m { result.insert(k.clone(), v.clone()); }
                }
            }
            Ok(Value::Object(result))
        }
        "zip" => {
            let a1 = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let a2 = match args.get(1).unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            Ok(Value::Array(a1.into_iter().zip(a2).map(|(a, b)| Value::Array(vec![a, b])).collect()))
        }
        "items" => {
            match args.first().unwrap_or(&Value::Null) {
                Value::Object(m) => {
                    let items: Vec<Value> = m.iter()
                        .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                        .collect();
                    Ok(Value::Array(items))
                }
                _ => Ok(Value::Null),
            }
        }
        "group_by" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let key_expr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Ok(Value::Null),
            };
            let mut groups: serde_json::Map<String, Value> = serde_json::Map::new();
            for item in arr {
                let k = eval_expr(&key_expr, &item).unwrap_or(Value::Null);
                let ks = match &k {
                    Value::String(s) => s.clone(),
                    _ => k.to_string(),
                };
                let entry = groups.entry(ks).or_insert(Value::Array(vec![]));
                if let Value::Array(arr) = entry {
                    arr.push(item);
                }
            }
            Ok(Value::Object(groups))
        }
        "map" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let expr = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Ok(Value::Null),
            };
            let results: Vec<Value> = arr.iter()
                .map(|item| eval_expr(&expr, item).unwrap_or(Value::Null))
                .collect();
            Ok(Value::Array(results))
        }
        "not_null" => {
            for arg in &args {
                if !matches!(arg, Value::Null) {
                    return Ok(arg.clone());
                }
            }
            Ok(Value::Null)
        }
        "flatten" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            Ok(Value::Array(arr.into_iter().flat_map(|v| match v {
                Value::Array(inner) => inner,
                other => vec![other],
            }).collect()))
        }
        "split" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let delim = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::Array(s.split(delim).map(|p| Value::String(p.into())).collect()))
        }
        "join" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let delim = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let parts: Vec<String> = arr.iter()
                .map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))
                .collect();
            Ok(Value::String(parts.join(delim)))
        }
        "trim" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::String(s.trim().to_string()))
        }
        "replace" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let old = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let new = args.get(2).and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::String(s.replace(old, new)))
        }
        "at" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a,
                _ => return Ok(Value::Null),
            };
            let idx = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as usize;
            Ok(arr.get(idx).cloned().unwrap_or(Value::Null))
        }
        "find_in" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let search = args.get(1).unwrap_or(&Value::Null);
            let idx = arr.iter().position(|v| v == search).map(|i| i as i64).unwrap_or(-1);
            Ok(serde_json::json!(idx))
        }
        "find_first" | "find_last" => {
            let arr = match args.first().unwrap_or(&Value::Null) {
                Value::Array(a) => a.clone(),
                _ => return Ok(Value::Null),
            };
            let search = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let found = if name == "find_first" {
                arr.iter().find(|v| v.as_str() == Some(search)).cloned()
            } else {
                arr.iter().rev().find(|v| v.as_str() == Some(search)).cloned()
            };
            Ok(found.unwrap_or(Value::Null))
        }
        "base64_decode" => {
            use base64::Engine as _;
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let bytes = base64::engine::general_purpose::STANDARD.decode(s)
                .map_err(|e| PolicyError::Eval(format!("base64_decode: {e}")))?;
            Ok(Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        }
        "base64_encode" => {
            use base64::Engine as _;
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::String(base64::engine::general_purpose::STANDARD.encode(s.as_bytes())))
        }
        "parse_json" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("null");
            Ok(serde_json::from_str(s).unwrap_or(Value::Null))
        }
        "parse_yaml" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("null");
            Ok(serde_yaml::from_str(s).unwrap_or(Value::Null))
        }
        "to_lower" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::String(s.to_lowercase()))
        }
        "to_upper" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::String(s.to_uppercase()))
        }
        "truncate" => {
            let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let n = args.get(1).and_then(|v| v.as_u64()).unwrap_or(u64::MAX) as usize;
            Ok(Value::String(s.chars().take(n).collect()))
        }
        "regex_match" => {
            let pattern = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let s = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let ok = regex::Regex::new(pattern).map(|re| re.is_match(s)).unwrap_or(false);
            Ok(Value::Bool(ok))
        }
        "pattern_match" => {
            let pattern = args.first().and_then(|v| v.as_str()).unwrap_or("");
            let s = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            Ok(Value::Bool(kyverno_pattern_match(pattern, s)))
        }
        _ => Err(PolicyError::Eval(format!("unknown JMESPath function: {name}"))),
    }
}

// ─── Pattern matching (Kyverno-style) ────────────────────────────────────────

/// Kyverno pattern matching: supports wildcards `*`, `?`, and ranges.
pub fn kyverno_pattern_match(pattern: &str, value: &str) -> bool {
    // Convert Kyverno pattern to regex
    let mut re = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '|' => re.push('|'),
            '\\' => {
                re.push('\\');
                if let Some(next) = chars.next() { re.push(next); }
            }
            c => {
                let escaped = regex::escape(&c.to_string());
                re.push_str(&escaped);
            }
        }
    }
    re.push('$');
    regex::Regex::new(&re).map(|r| r.is_match(value)).unwrap_or(false)
}

/// Variable substitution: replace `{{request.object.spec.replicas}}` style templates.
pub fn substitute_variables(template: &str, context: &Value) -> Result<String, PolicyError> {
    let mut result = template.to_string();
    let re = regex::Regex::new(r"\{\{([^}]+)\}\}").unwrap();
    let mut offset = 0i64;
    let binding = result.clone();
    let matches: Vec<_> = re.find_iter(&binding).collect();
    for m in matches {
        let expr = &m.as_str()[2..m.as_str().len() - 2].trim().to_string();
        let val = evaluate(expr, context)?;
        let replacement = match &val {
            Value::String(s) => s.clone(),
            _ => val.to_string(),
        };
        let start = (m.start() as i64 + offset) as usize;
        let end = (m.end() as i64 + offset) as usize;
        result.replace_range(start..end, &replacement);
        offset += replacement.len() as i64 - m.as_str().len() as i64;
    }
    Ok(result)
}

/// Substitute variables in a JSON value recursively.
pub fn substitute_variables_json(value: &Value, context: &Value) -> Result<Value, PolicyError> {
    match value {
        Value::String(s) => {
            // If it's a pure expression {{...}}, evaluate to any type
            let trimmed = s.trim();
            if trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                let expr = &trimmed[2..trimmed.len() - 2].trim().to_string();
                return evaluate(expr, context);
            }
            Ok(Value::String(substitute_variables(s, context)?))
        }
        Value::Object(m) => {
            let mut new_m = serde_json::Map::new();
            for (k, v) in m {
                let new_k = substitute_variables(k, context)?;
                let new_v = substitute_variables_json(v, context)?;
                new_m.insert(new_k, new_v);
            }
            Ok(Value::Object(new_m))
        }
        Value::Array(a) => {
            let new_a: Result<Vec<_>, _> = a.iter().map(|v| substitute_variables_json(v, context)).collect();
            Ok(Value::Array(new_a?))
        }
        _ => Ok(value.clone()),
    }
}

// ─── Parsing helpers ──────────────────────────────────────────────────────────

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut depth = 0i32;
    let mut parts = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' { i += 1; }
                    i += 1;
                }
            }
            c if c == sep as u8 && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

fn split_first_segment(expr: &str) -> Option<(&str, String)> {
    let bytes = expr.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'.' if depth == 0 && i > 0 => {
                return Some((&expr[..i], expr[i + 1..].to_string()));
            }
            b'[' if depth == 0 && i > 0 => {
                // Find closing ]
                let seg_end = i;
                let mut j = i;
                let mut d = 0i32;
                while j < bytes.len() {
                    match bytes[j] {
                        b'[' => d += 1,
                        b']' => { d -= 1; if d == 0 { j += 1; break; } }
                        _ => {}
                    }
                    j += 1;
                }
                let head = &expr[..j];
                let tail = if j < bytes.len() && bytes[j] == b'.' {
                    expr[j + 1..].to_string()
                } else {
                    expr[j..].to_string()
                };
                let _ = seg_end;
                return Some((head, tail));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn find_pipe(expr: &str) -> Option<usize> {
    find_binary_op(expr, b'|', 1, |ahead| ahead.get(0) != Some(&b'|'))
}

fn find_or(expr: &str) -> Option<usize> {
    find_binary_op(expr, b'|', 2, |ahead| ahead.first() == Some(&b'|'))
}

fn find_and(expr: &str) -> Option<usize> {
    find_binary_op(expr, b'&', 2, |ahead| ahead.first() == Some(&b'&'))
}

fn find_binary_op(expr: &str, ch: u8, len: usize, check: impl Fn(&[u8]) -> bool) -> Option<usize> {
    let bytes = expr.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' { i += 1; }
            }
            c if c == ch && depth == 0 && i > 0 => {
                let ahead = &bytes[i + 1..];
                if check(ahead) {
                    return Some(i);
                }
                i += len - 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn find_operator(expr: &str, op: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i + op_bytes.len() <= bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && &bytes[i..i + op_bytes.len()] == op_bytes {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_function_call(expr: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    // Find a `(` that is at depth 0 and not at position 0
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' && i > 0 {
            // Check it's a proper function call (name before it)
            let name_part = &expr[..i];
            if name_part.chars().all(|c| c.is_alphanumeric() || c == '_') {
                // Check closing paren is the last char
                if bytes.last() == Some(&b')') {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

fn flatten_array(v: &Value) -> Value {
    match v {
        Value::Array(a) => {
            Value::Array(a.iter().flat_map(|item| match item {
                Value::Array(inner) => inner.clone(),
                _ => vec![item.clone()],
            }).collect())
        }
        _ => v.clone(),
    }
}

fn is_falsy(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Bool(false) => true,
        Value::Array(a) => a.is_empty(),
        Value::String(s) => s.is_empty(),
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}
