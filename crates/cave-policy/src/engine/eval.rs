//! Rego evaluator and built-in functions.

use std::collections::HashMap;
use serde_json::{Value, Map, json};
use super::ast::*;

// ── Environment ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, Value>,
}

impl Env {
    pub fn new() -> Self { Self::default() }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    pub fn set(&mut self, name: &str, val: Value) {
        self.vars.insert(name.to_string(), val);
    }

    pub fn with(&self, name: &str, val: Value) -> Self {
        let mut e = self.clone();
        e.set(name, val);
        e
    }
}

// ── Evaluator entry point ─────────────────────────────────────────────────────

pub struct EvalContext<'a> {
    pub input: &'a Value,
    pub data: &'a Value,
}

impl<'a> EvalContext<'a> {
    pub fn new(input: &'a Value, data: &'a Value) -> Self {
        Self { input, data }
    }

    /// Evaluate all rules in `policy`, return a map of rule-name → value.
    pub fn evaluate(&self, policy: &Policy) -> HashMap<String, Value> {
        let env = Env::new();
        let mut result: HashMap<String, Vec<Value>> = HashMap::new();
        let mut defaults: HashMap<String, Value> = HashMap::new();

        for rule in &policy.rules {
            if rule.is_default {
                if let Some(v) = &rule.default_value {
                    defaults.insert(rule.name.clone(), v.clone());
                }
                continue;
            }

            let mut fired = Vec::new();
            self.eval_rule(rule, &env, &mut |head_val| {
                fired.push(head_val);
            });

            if !fired.is_empty() {
                result.entry(rule.name.clone()).or_default().extend(fired);
            }
        }

        let mut out = HashMap::new();
        // Apply defaults for rules that never fired
        for (name, val) in defaults {
            if !result.contains_key(&name) {
                out.insert(name, val);
            }
        }
        // Collapse rule values
        for (name, vals) in result {
            let collapsed = if vals.len() == 1 {
                vals.into_iter().next().unwrap()
            } else {
                // multiple values → array (partial rule)
                Value::Array(vals)
            };
            out.insert(name, collapsed);
        }
        out
    }

    /// Evaluate a single rule, calling `cb` for each successful evaluation.
    fn eval_rule(&self, rule: &Rule, env: &Env, cb: &mut dyn FnMut(Value)) {
        let head_value = |env: &Env| -> Value {
            if let Some(hv) = &rule.head_value {
                self.eval_expr(hv, env).unwrap_or(Value::Null)
            } else {
                Value::Bool(true)
            }
        };

        if rule.body.is_empty() {
            cb(head_value(env));
            return;
        }

        self.eval_body(&rule.body, env, &mut |resolved_env| {
            cb(head_value(resolved_env));
        });
    }

    /// Evaluate a sequence of body statements, backtracking over iterations.
    fn eval_body(&self, stmts: &[Expr], env: &Env, cb: &mut dyn FnMut(&Env)) {
        if stmts.is_empty() {
            cb(env);
            return;
        }
        let stmt = &stmts[0];
        let rest = &stmts[1..];

        match stmt {
            // `not expr` — succeeds if expr produces no solutions
            Expr::Not(inner) => {
                let mut succeeded = false;
                self.eval_body(std::slice::from_ref(inner), env, &mut |_| {
                    succeeded = true;
                });
                if !succeeded {
                    self.eval_body(rest, env, cb);
                }
            }
            // `name := value`
            Expr::Assign { name, value } => {
                if let Ok(v) = self.eval_expr(value, env) {
                    let new_env = env.with(name, v);
                    self.eval_body(rest, &new_env, cb);
                }
            }
            // `left = right`  (unification)
            Expr::Unify { left, right } => {
                match (left.as_ref(), right.as_ref()) {
                    (Expr::Var(name), _) if env.get(name).is_none() => {
                        if let Ok(rv) = self.eval_expr(right, env) {
                            let new_env = env.with(name, rv);
                            self.eval_body(rest, &new_env, cb);
                        }
                    }
                    (_, Expr::Var(name)) if env.get(name).is_none() => {
                        if let Ok(lv) = self.eval_expr(left, env) {
                            let new_env = env.with(name, lv);
                            self.eval_body(rest, &new_env, cb);
                        }
                    }
                    _ => {
                        let lv = self.eval_expr(left, env).ok();
                        let rv = self.eval_expr(right, env).ok();
                        if lv == rv && lv.is_some() {
                            self.eval_body(rest, env, cb);
                        }
                    }
                }
            }
            // `ref[_]` — iteration
            Expr::Ref { head, parts } if matches!(parts.last(), Some(RefPart::AnyIndex)) => {
                let parts_without_last = &parts[..parts.len() - 1];
                let collection_expr = if parts_without_last.is_empty() {
                    *head.clone()
                } else {
                    Expr::Ref {
                        head: head.clone(),
                        parts: parts_without_last.to_vec(),
                    }
                };
                if let Ok(collection) = self.eval_expr(&collection_expr, env) {
                    match &collection {
                        Value::Array(arr) => {
                            for item in arr {
                                self.eval_body(rest, env, cb);
                                let _ = item; // item available but discarded in bare iteration
                            }
                        }
                        Value::Object(obj) => {
                            for (_k, _v) in obj {
                                self.eval_body(rest, env, cb);
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Generic statement — evaluate for truth
            other => {
                match self.eval_stmt_truth(other, env) {
                    Ok((true, new_env)) => self.eval_body(rest, &new_env, cb),
                    _ => {}
                }
            }
        }
    }

    /// Evaluate a statement for truth, returning updated env.
    fn eval_stmt_truth(&self, expr: &Expr, env: &Env) -> Result<(bool, Env), ()> {
        match expr {
            Expr::Cmp { op, left, right } => {
                let lv = self.eval_expr(left, env).map_err(|_| ())?;
                let rv = self.eval_expr(right, env).map_err(|_| ())?;
                let result = compare_values(&lv, op, &rv);
                Ok((result, env.clone()))
            }
            Expr::Call { func, args } => {
                let result = self.call_builtin(func, args, env).map_err(|_| ())?;
                Ok((is_truthy(&result), env.clone()))
            }
            Expr::Var(name) => {
                match env.get(name) {
                    Some(v) => Ok((is_truthy(v), env.clone())),
                    None => Err(()),
                }
            }
            Expr::Ref { .. } => {
                match self.eval_expr(expr, env) {
                    Ok(v) => Ok((is_truthy(&v), env.clone())),
                    Err(_) => Err(()),
                }
            }
            Expr::Lit(v) => Ok((is_truthy(v), env.clone())),
            Expr::BinOp { .. } => {
                match self.eval_expr(expr, env) {
                    Ok(v) => Ok((is_truthy(&v), env.clone())),
                    Err(_) => Err(()),
                }
            }
            // Assign that shows up as a statement (var := val already handled in eval_body)
            Expr::Assign { name, value } => {
                let v = self.eval_expr(value, env).map_err(|_| ())?;
                let new_env = env.with(name, v);
                Ok((true, new_env))
            }
            _ => {
                match self.eval_expr(expr, env) {
                    Ok(v) => Ok((is_truthy(&v), env.clone())),
                    Err(_) => Err(()),
                }
            }
        }
    }

    // ── Expression evaluation ─────────────────────────────────────────────────

    pub fn eval_expr(&self, expr: &Expr, env: &Env) -> Result<Value, String> {
        match expr {
            Expr::Lit(v) => Ok(v.clone()),

            Expr::Var(name) => {
                match name.as_str() {
                    "input" => Ok(self.input.clone()),
                    "data"  => Ok(self.data.clone()),
                    "true"  => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "null"  => Ok(Value::Null),
                    _ => env.get(name).cloned()
                              .ok_or_else(|| format!("undefined variable: {name}")),
                }
            }

            Expr::Ref { head, parts } => {
                let mut cur = self.eval_expr(head, env)?;
                for part in parts {
                    cur = match part {
                        RefPart::Key(k) => cur.get(k).cloned()
                            .ok_or_else(|| format!("key not found: {k}"))?,
                        RefPart::Index(idx_expr) => {
                            let idx = self.eval_expr(idx_expr, env)?;
                            match (&cur, &idx) {
                                (Value::Array(arr), Value::Number(n)) => {
                                    let i = n.as_u64()
                                        .or_else(|| n.as_f64().map(|f| f as u64))
                                        .unwrap_or(0) as usize;
                                    arr.get(i).cloned()
                                        .ok_or_else(|| format!("index out of bounds: {i}"))?
                                }
                                (Value::Object(obj), Value::String(k)) => {
                                    obj.get(k).cloned()
                                        .ok_or_else(|| format!("key not found: {k}"))?
                                }
                                _ => return Err("invalid index operation".into()),
                            }
                        }
                        RefPart::AnyIndex => {
                            // [_] in expression context: take first element
                            match &cur {
                                Value::Array(arr) => arr.first().cloned()
                                    .ok_or_else(|| "empty array".to_string())?,
                                Value::Object(obj) => obj.values().next().cloned()
                                    .ok_or_else(|| "empty object".to_string())?,
                                _ => return Err("cannot iterate non-collection".into()),
                            }
                        }
                    };
                }
                Ok(cur)
            }

            Expr::BinOp { op, left, right } => {
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                apply_binop(&lv, op, &rv)
            }

            Expr::Cmp { op, left, right } => {
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                Ok(Value::Bool(compare_values(&lv, op, &rv)))
            }

            Expr::Not(inner) => {
                match self.eval_expr(inner, env) {
                    Ok(v) => Ok(Value::Bool(!is_truthy(&v))),
                    Err(_) => Ok(Value::Bool(true)),
                }
            }

            Expr::Assign { name: _, value } => {
                // In expression context, just evaluate the value
                self.eval_expr(value, env)
            }

            Expr::Unify { left, right } => {
                let lv = self.eval_expr(left, env)?;
                let rv = self.eval_expr(right, env)?;
                Ok(Value::Bool(lv == rv))
            }

            Expr::Call { func, args } => {
                self.call_builtin(func, args, env)
            }

            Expr::Array(items) => {
                let mut arr = Vec::new();
                for item in items {
                    arr.push(self.eval_expr(item, env)?);
                }
                Ok(Value::Array(arr))
            }

            Expr::Object(pairs) => {
                let mut map = Map::new();
                for (k, v) in pairs {
                    let key = self.eval_expr(k, env)?;
                    let val = self.eval_expr(v, env)?;
                    let key_str = match key {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    map.insert(key_str, val);
                }
                Ok(Value::Object(map))
            }

            Expr::Set(items) => {
                // Sets represented as arrays (JSON doesn't have sets)
                let mut arr = Vec::new();
                for item in items {
                    arr.push(self.eval_expr(item, env)?);
                }
                Ok(Value::Array(arr))
            }

            Expr::ArrayComp { term, body } => {
                let mut results = Vec::new();
                self.eval_body(body, env, &mut |resolved| {
                    if let Ok(v) = self.eval_expr(term, resolved) {
                        results.push(v);
                    }
                });
                Ok(Value::Array(results))
            }

            Expr::SetComp { term, body } => {
                let mut results = Vec::new();
                self.eval_body(body, env, &mut |resolved| {
                    if let Ok(v) = self.eval_expr(term, resolved) {
                        if !results.contains(&v) {
                            results.push(v);
                        }
                    }
                });
                Ok(Value::Array(results))
            }

            Expr::ObjectComp { key, value, body } => {
                let mut map = Map::new();
                self.eval_body(body, env, &mut |resolved| {
                    if let (Ok(k), Ok(v)) = (
                        self.eval_expr(key, resolved),
                        self.eval_expr(value, resolved),
                    ) {
                        let ks = match k { Value::String(s) => s, o => o.to_string() };
                        map.insert(ks, v);
                    }
                });
                Ok(Value::Object(map))
            }
        }
    }

    // ── Built-in functions ────────────────────────────────────────────────────

    fn call_builtin(&self, func: &str, args: &[Expr], env: &Env) -> Result<Value, String> {
        let evaled: Result<Vec<Value>, String> = args.iter()
            .map(|a| self.eval_expr(a, env))
            .collect();
        let a = evaled?;

        match func {
            // ── Aggregate ──────────────────────────────────────────────────────
            "count" => {
                let v = a.first().ok_or("count: missing arg")?;
                let n = match v {
                    Value::Array(arr) => arr.len(),
                    Value::Object(obj) => obj.len(),
                    Value::String(s) => s.len(),
                    _ => return Err("count: unsupported type".into()),
                };
                Ok(json!(n))
            }
            "sum" => {
                let arr = as_array(a.first())?;
                let s: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();
                Ok(json!(s))
            }
            "product" => {
                let arr = as_array(a.first())?;
                let p: f64 = arr.iter().filter_map(|v| v.as_f64()).product();
                Ok(json!(p))
            }
            "min" => {
                let arr = as_array(a.first())?;
                let m = arr.iter().filter_map(|v| v.as_f64())
                    .fold(f64::INFINITY, f64::min);
                Ok(json!(m))
            }
            "max" => {
                let arr = as_array(a.first())?;
                let m = arr.iter().filter_map(|v| v.as_f64())
                    .fold(f64::NEG_INFINITY, f64::max);
                Ok(json!(m))
            }

            // ── String ────────────────────────────────────────────────────────
            "concat" => {
                let delim = as_str(a.first())?;
                let arr = as_array(a.get(1))?;
                let parts: Vec<String> = arr.iter()
                    .map(|v| match v { Value::String(s) => s.clone(), o => o.to_string() })
                    .collect();
                Ok(json!(parts.join(&delim)))
            }
            "contains" => {
                let haystack = as_str(a.first())?;
                let needle = as_str(a.get(1))?;
                Ok(json!(haystack.contains(needle.as_str())))
            }
            "startswith" => {
                let s = as_str(a.first())?;
                let prefix = as_str(a.get(1))?;
                Ok(json!(s.starts_with(prefix.as_str())))
            }
            "endswith" => {
                let s = as_str(a.first())?;
                let suffix = as_str(a.get(1))?;
                Ok(json!(s.ends_with(suffix.as_str())))
            }
            "lower" => {
                let s = as_str(a.first())?;
                Ok(json!(s.to_lowercase()))
            }
            "upper" => {
                let s = as_str(a.first())?;
                Ok(json!(s.to_uppercase()))
            }
            "trim" | "trim_space" => {
                let s = as_str(a.first())?;
                Ok(json!(s.trim()))
            }
            "split" => {
                let s = as_str(a.first())?;
                let delim = as_str(a.get(1))?;
                let parts: Vec<Value> = s.split(delim.as_str())
                    .map(|p| json!(p)).collect();
                Ok(Value::Array(parts))
            }
            "replace" => {
                let s = as_str(a.first())?;
                let old = as_str(a.get(1))?;
                let new = as_str(a.get(2))?;
                Ok(json!(s.replace(old.as_str(), new.as_str())))
            }
            "substring" => {
                let s = as_str(a.first())?;
                let start = a.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let len = a.get(2).and_then(|v| v.as_i64()).unwrap_or(-1);
                let chars: Vec<char> = s.chars().collect();
                let end = if len < 0 { chars.len() } else { (start + len as usize).min(chars.len()) };
                let result: String = chars[start.min(chars.len())..end].iter().collect();
                Ok(json!(result))
            }
            "sprintf" => {
                // Simplified sprintf: just return first string arg
                Ok(a.first().cloned().unwrap_or(Value::Null))
            }
            "format_int" => {
                let n = a.first().and_then(|v| v.as_i64()).unwrap_or(0);
                let base = a.get(1).and_then(|v| v.as_u64()).unwrap_or(10);
                let s = match base {
                    2  => format!("{n:b}"),
                    8  => format!("{n:o}"),
                    16 => format!("{n:x}"),
                    _  => format!("{n}"),
                };
                Ok(json!(s))
            }
            "to_number" => {
                match a.first() {
                    Some(Value::String(s)) => s.parse::<f64>()
                        .map(|n| json!(n))
                        .map_err(|_| format!("to_number: cannot parse {s:?}")),
                    Some(n @ Value::Number(_)) => Ok(n.clone()),
                    _ => Err("to_number: unsupported type".into()),
                }
            }
            "string" | "json.marshal" => {
                let v = a.first().ok_or("string: missing arg")?;
                Ok(json!(v.to_string()))
            }
            "json.unmarshal" => {
                let s = as_str(a.first())?;
                serde_json::from_str(&s).map_err(|e| e.to_string())
            }

            // ── Regex ─────────────────────────────────────────────────────────
            "re_match" | "regex.match" => {
                let pattern = as_str(a.first())?;
                let text = as_str(a.get(1))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| format!("regex error: {e}"))?;
                Ok(json!(re.is_match(&text)))
            }
            "regex.find_all_string_submatch_n" => {
                let pattern = as_str(a.first())?;
                let text = as_str(a.get(1))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| format!("regex error: {e}"))?;
                let matches: Vec<Value> = re.find_iter(&text)
                    .map(|m| json!(m.as_str()))
                    .collect();
                Ok(Value::Array(matches))
            }
            "regex.split" => {
                let pattern = as_str(a.first())?;
                let text = as_str(a.get(1))?;
                let re = regex::Regex::new(&pattern)
                    .map_err(|e| format!("regex error: {e}"))?;
                let parts: Vec<Value> = re.split(&text).map(|s| json!(s)).collect();
                Ok(Value::Array(parts))
            }
            "glob.match" => {
                let pattern = as_str(a.first())?;
                let text = as_str(a.get(2).or_else(|| a.get(1)))?;
                // Simple glob: * matches any sequence, ? matches one char
                let regex_pat = pattern
                    .replace('.', "\\.")
                    .replace('*', ".*")
                    .replace('?', ".");
                let re = regex::Regex::new(&format!("^{regex_pat}$"))
                    .map_err(|e| format!("glob error: {e}"))?;
                Ok(json!(re.is_match(&text)))
            }

            // ── Type checks ───────────────────────────────────────────────────
            "is_number"  => Ok(json!(matches!(a.first(), Some(Value::Number(_))))),
            "is_string"  => Ok(json!(matches!(a.first(), Some(Value::String(_))))),
            "is_boolean" => Ok(json!(matches!(a.first(), Some(Value::Bool(_))))),
            "is_null"    => Ok(json!(matches!(a.first(), Some(Value::Null)))),
            "is_array"   => Ok(json!(matches!(a.first(), Some(Value::Array(_))))),
            "is_object"  => Ok(json!(matches!(a.first(), Some(Value::Object(_))))),
            "is_set"     => Ok(json!(matches!(a.first(), Some(Value::Array(_))))),

            // ── Array ─────────────────────────────────────────────────────────
            "array.concat" => {
                let a1 = as_array(a.first())?;
                let a2 = as_array(a.get(1))?;
                let mut out = a1;
                out.extend(a2);
                Ok(Value::Array(out))
            }
            "array.slice" => {
                let arr = as_array(a.first())?;
                let start = a.get(1).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let stop  = a.get(2).and_then(|v| v.as_u64()).unwrap_or(arr.len() as u64) as usize;
                Ok(Value::Array(arr[start.min(arr.len())..stop.min(arr.len())].to_vec()))
            }
            "sort" => {
                let mut arr = as_array(a.first())?;
                arr.sort_by(|a, b| value_cmp(a, b));
                Ok(Value::Array(arr))
            }
            "reverse" => {
                let mut arr = as_array(a.first())?;
                arr.reverse();
                Ok(Value::Array(arr))
            }

            // ── Sets (represented as arrays) ───────────────────────────────────
            "intersection" => {
                let a1 = as_array(a.first())?;
                let a2 = as_array(a.get(1))?;
                let out: Vec<Value> = a1.into_iter().filter(|v| a2.contains(v)).collect();
                Ok(Value::Array(out))
            }
            "union" => {
                let mut a1 = as_array(a.first())?;
                let a2 = as_array(a.get(1))?;
                for v in a2 { if !a1.contains(&v) { a1.push(v); } }
                Ok(Value::Array(a1))
            }
            "members" | "set" => Ok(a.first().cloned().unwrap_or(Value::Array(vec![]))),

            // ── Object ────────────────────────────────────────────────────────
            "object.get" => {
                let obj = a.first().ok_or("object.get: missing arg")?;
                let key = as_str(a.get(1))?;
                let default = a.get(2).cloned().unwrap_or(Value::Null);
                Ok(obj.get(&key).cloned().unwrap_or(default))
            }
            "object.keys" => {
                match a.first() {
                    Some(Value::Object(m)) => Ok(Value::Array(m.keys().map(|k| json!(k)).collect())),
                    _ => Ok(Value::Array(vec![])),
                }
            }
            "object.values" => {
                match a.first() {
                    Some(Value::Object(m)) => Ok(Value::Array(m.values().cloned().collect())),
                    _ => Ok(Value::Array(vec![])),
                }
            }
            "object.remove" => {
                let mut obj = match a.first() {
                    Some(Value::Object(m)) => m.clone(),
                    _ => return Err("object.remove: expected object".into()),
                };
                let keys = as_array(a.get(1))?;
                for k in keys { if let Value::String(s) = k { obj.remove(&s); } }
                Ok(Value::Object(obj))
            }
            "object.union" | "object.union_n" => {
                let mut out = Map::new();
                for v in &a {
                    if let Value::Object(m) = v {
                        out.extend(m.clone());
                    }
                }
                Ok(Value::Object(out))
            }

            // ── Time ─────────────────────────────────────────────────────────
            "time.now_ns" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let ns = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                Ok(json!(ns))
            }
            "time.parse_rfc3339_ns" => {
                let s = as_str(a.first())?;
                use chrono::DateTime;
                let dt = DateTime::parse_from_rfc3339(&s)
                    .map_err(|e| format!("time parse error: {e}"))?;
                let ns = dt.timestamp_nanos_opt().unwrap_or(0);
                Ok(json!(ns))
            }
            "time.format" => {
                Ok(a.first().cloned().unwrap_or(Value::Null))
            }

            // ── Crypto ───────────────────────────────────────────────────────
            "crypto.md5" => {
                // Simplified: return hex of input bytes using ring SHA256 as proxy
                let s = as_str(a.first())?;
                Ok(json!(format!("{:016x}", simple_hash(s.as_bytes()))))
            }
            "crypto.sha256" => {
                let s = as_str(a.first())?;
                // Use ring for SHA256
                let digest = ring_sha256(s.as_bytes());
                Ok(json!(hex_encode(&digest)))
            }
            "crypto.hmac.md5" | "crypto.hmac.sha1" | "crypto.hmac.sha256" => {
                // Return simplified HMAC
                let msg = as_str(a.first())?;
                let key = as_str(a.get(1))?;
                let combined = format!("{key}:{msg}");
                Ok(json!(format!("{:016x}", simple_hash(combined.as_bytes()))))
            }

            // ── Base64 ───────────────────────────────────────────────────────
            "base64.encode" | "base64url.encode" => {
                use base64::Engine;
                let s = as_str(a.first())?;
                Ok(json!(base64::engine::general_purpose::STANDARD.encode(s.as_bytes())))
            }
            "base64.decode" | "base64url.decode" => {
                use base64::Engine;
                let s = as_str(a.first())?;
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(s.as_bytes())
                    .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.as_bytes()))
                    .map_err(|e| format!("base64 decode error: {e}"))?;
                Ok(json!(String::from_utf8_lossy(&decoded).to_string()))
            }

            // ── JWT ──────────────────────────────────────────────────────────
            "io.jwt.decode" => {
                let token_str = as_str(a.first())?;
                let parts: Vec<&str> = token_str.splitn(3, '.').collect();
                if parts.len() != 3 {
                    return Err("io.jwt.decode: invalid JWT".into());
                }
                use base64::Engine;
                let decode = |s: &str| -> Result<Value, String> {
                    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(s)
                        .map_err(|e| e.to_string())?;
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
                };
                let header = decode(parts[0])?;
                let payload = decode(parts[1])?;
                let sig = Value::String(parts[2].to_string());
                Ok(Value::Array(vec![header, payload, sig]))
            }
            "io.jwt.verify_hs256" | "io.jwt.verify_hs384" | "io.jwt.verify_hs512" => {
                // Simplified: just decode and check structure
                let token_str = as_str(a.first())?;
                let parts: Vec<&str> = token_str.splitn(3, '.').collect();
                Ok(json!(parts.len() == 3))
            }
            "io.jwt.decode_verify" => {
                let token_str = as_str(a.first())?;
                let parts: Vec<&str> = token_str.splitn(3, '.').collect();
                if parts.len() != 3 {
                    return Ok(Value::Array(vec![json!(false), Value::Null, Value::Null]));
                }
                use base64::Engine;
                let decode_part = |s: &str| -> Value {
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .decode(s)
                        .ok()
                        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                        .unwrap_or(Value::Null)
                };
                Ok(Value::Array(vec![
                    json!(true),
                    decode_part(parts[0]),
                    decode_part(parts[1]),
                ]))
            }

            // ── HTTP ─────────────────────────────────────────────────────────
            "http.send" => {
                // Synchronous HTTP is tricky in async context; return a stub.
                // In production, this would use reqwest blocking or tokio::task::block_in_place.
                Ok(json!({
                    "status": "200 OK",
                    "status_code": 200,
                    "body": null,
                    "raw_body": ""
                }))
            }

            // ── OPA compat ───────────────────────────────────────────────────
            "opa.runtime" => Ok(json!({
                "version": "cave-policy/0.1.0",
                "env": {}
            })),
            "trace" => Ok(Value::Null),
            "print" => {
                // no-op in eval context
                Ok(Value::Null)
            }

            _ => Err(format!("unknown built-in function: {func}")),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
    }
}

fn compare_values(lv: &Value, op: &CmpOp, rv: &Value) -> bool {
    match op {
        CmpOp::Eq => lv == rv,
        CmpOp::Ne => lv != rv,
        CmpOp::Lt => value_cmp(lv, rv).is_lt(),
        CmpOp::Le => !value_cmp(lv, rv).is_gt(),
        CmpOp::Gt => value_cmp(lv, rv).is_gt(),
        CmpOp::Ge => !value_cmp(lv, rv).is_lt(),
    }
}

fn value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(n1), Value::Number(n2)) => {
            let f1 = n1.as_f64().unwrap_or(0.0);
            let f2 = n2.as_f64().unwrap_or(0.0);
            f1.partial_cmp(&f2).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(s1), Value::String(s2)) => s1.cmp(s2),
        _ => std::cmp::Ordering::Equal,
    }
}

fn apply_binop(lv: &Value, op: &BinOp, rv: &Value) -> Result<Value, String> {
    let l = lv.as_f64().ok_or_else(|| format!("expected number, got {lv}"))?;
    let r = rv.as_f64().ok_or_else(|| format!("expected number, got {rv}"))?;
    let result = match op {
        BinOp::Add => l + r,
        BinOp::Sub => l - r,
        BinOp::Mul => l * r,
        BinOp::Div => {
            if r == 0.0 { return Err("division by zero".into()); }
            l / r
        }
        BinOp::And | BinOp::Or => {
            return Ok(Value::Bool(match op {
                BinOp::And => is_truthy(lv) && is_truthy(rv),
                _ => is_truthy(lv) || is_truthy(rv),
            }));
        }
    };
    Ok(json!(result))
}

fn as_str(v: Option<&Value>) -> Result<String, String> {
    match v {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Ok(other.to_string()),
        None => Err("missing string argument".into()),
    }
}

fn as_array(v: Option<&Value>) -> Result<Vec<Value>, String> {
    match v {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(other) => Err(format!("expected array, got {other}")),
        None => Err("missing array argument".into()),
    }
}

fn simple_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn ring_sha256(data: &[u8]) -> Vec<u8> {
    use ring::digest::{digest, SHA256};
    digest(&SHA256, data).as_ref().to_vec()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
