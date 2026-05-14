// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego evaluator — tree-walking interpreter with backtracking.
//!
//! Implements OPA's evaluation model:
//! - Complete rules (single value or undefined)
//! - Partial set rules (collect into set)
//! - Partial object rules (collect into object)
//! - Function rules (parametric)
//! - Comprehensions (array, set, object)
//! - Negation-as-failure (not)
//! - every / some
//! - with modifier

use super::ast::*;
use super::builtins::Builtins;
use super::value::{Bindings, Value, json_get_path};
use crate::error::PolicyError;
use std::collections::HashMap;
use std::sync::Arc;

/// Evaluation context — immutable per-query state.
#[derive(Clone)]
pub struct EvalCtx {
    pub data: serde_json::Value,
    pub input: serde_json::Value,
    pub modules: Arc<HashMap<String, Module>>,
    pub builtins: Arc<Builtins>,
    pub call_stack: Vec<String>,
}

impl EvalCtx {
    pub fn new(
        data: serde_json::Value,
        input: serde_json::Value,
        modules: Arc<HashMap<String, Module>>,
    ) -> Self {
        Self {
            data,
            input,
            modules,
            builtins: Arc::new(Builtins::new()),
            call_stack: Vec::new(),
        }
    }

    /// Create a new context with a path override (for `with` modifier).
    fn with_path_override(&self, path: &[String], value: serde_json::Value) -> Self {
        let mut ctx = self.clone();
        if path.len() >= 2 && path[0] == "data" {
            set_nested(&mut ctx.data, &path[1..], value);
        } else if path.len() >= 2 && path[0] == "input" {
            set_nested(&mut ctx.input, &path[1..], value);
        }
        ctx
    }
}

fn set_nested(target: &mut serde_json::Value, path: &[String], value: serde_json::Value) {
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
            let entry = m.entry(path[0].clone()).or_insert(serde_json::Value::Object(Default::default()));
            set_nested(entry, &path[1..], value);
        }
    }
}

/// Evaluate a Rego query against a set of loaded modules.
pub struct Evaluator {
    ctx: EvalCtx,
}

impl Evaluator {
    pub fn new(ctx: EvalCtx) -> Self {
        Self { ctx }
    }

    /// Evaluate a query body and return all possible binding sets.
    pub fn query(&self, body: &Body) -> Vec<Bindings> {
        eval_body(body, &HashMap::new(), &self.ctx)
    }

    /// Evaluate a data path (e.g., "data.authz.allow") with given input.
    pub fn query_path(&self, path: &[String], input: serde_json::Value) -> Value {
        let mut ctx = self.ctx.clone();
        ctx.input = input;
        // Build a term for the path
        let term = path_to_term(path);
        eval_term(&term, &HashMap::new(), &ctx)
    }
}

fn path_to_term(path: &[String]) -> Term {
    if path.is_empty() {
        return Term::Var("data".into());
    }
    let mut base = Term::Var(path[0].clone());
    let args: Vec<RefArg> = path[1..].iter().map(|s| RefArg::Field(s.clone())).collect();
    if args.is_empty() {
        base
    } else {
        Term::Ref(Box::new(base), args)
    }
}

// ─── Body evaluation ──────────────────────────────────────────────────────────

/// Evaluate a conjunction of expressions. Returns all satisfying binding sets.
pub fn eval_body(body: &[Expr], bindings: &Bindings, ctx: &EvalCtx) -> Vec<Bindings> {
    if body.is_empty() {
        return vec![bindings.clone()];
    }
    let mut results = Vec::new();
    let first = &body[0];
    let rest = &body[1..];

    for new_bindings in eval_expr(first, bindings, ctx) {
        results.extend(eval_body(rest, &new_bindings, ctx));
    }
    results
}

/// Evaluate a single expression. Returns binding sets (0 = failure, ≥1 = success).
fn eval_expr(expr: &Expr, bindings: &Bindings, ctx: &EvalCtx) -> Vec<Bindings> {
    match expr {
        Expr::Term(term) => {
            let v = eval_term(term, bindings, ctx);
            if v.is_truthy() {
                vec![bindings.clone()]
            } else {
                vec![]
            }
        }

        Expr::Unify(left, right) => {
            unify(left, right, bindings.clone(), ctx)
        }

        Expr::Assign(left, right) => {
            // := always assigns to a new variable; the left must be a var or term
            let rval = eval_term(right, bindings, ctx);
            if rval.is_undefined() {
                return vec![];
            }
            // Bind left
            unify_value(left, rval, bindings.clone(), ctx)
        }

        Expr::Not(inner_expr) => {
            if eval_expr(inner_expr, bindings, ctx).is_empty() {
                vec![bindings.clone()]
            } else {
                vec![]
            }
        }

        Expr::NotBody(body) => {
            if eval_body(body, bindings, ctx).is_empty() {
                vec![bindings.clone()]
            } else {
                vec![]
            }
        }

        Expr::Every { key, value, domain, body } => {
            let domain_val = eval_term(domain, bindings, ctx);
            let items = match &domain_val {
                Value::Json(serde_json::Value::Array(a)) => {
                    a.iter().enumerate()
                        .map(|(i, v)| (serde_json::json!(i), v.clone()))
                        .collect::<Vec<_>>()
                }
                Value::Json(serde_json::Value::Object(m)) => {
                    m.iter().map(|(k, v)| (serde_json::json!(k), v.clone())).collect()
                }
                Value::Set(s) => {
                    s.iter().enumerate()
                        .map(|(i, v)| (serde_json::json!(i), v.clone()))
                        .collect()
                }
                _ => return vec![],
            };

            for (k, v) in &items {
                let mut b = bindings.clone();
                if let Some(kname) = key {
                    b.insert(kname.clone(), Value::Json(k.clone()));
                }
                b.insert(value.clone(), Value::Json(v.clone()));
                if eval_body(body, &b, ctx).is_empty() {
                    return vec![];
                }
            }
            vec![bindings.clone()]
        }

        Expr::Some(vars) => {
            // `some x` — just declare the variable; it stays unbound
            let mut b = bindings.clone();
            for var in vars {
                b.entry(var.clone()).or_insert(Value::Undefined);
            }
            vec![b]
        }

        Expr::SomeIn { key, value, domain } => {
            let domain_val = eval_term(domain, bindings, ctx);
            let items: Vec<(serde_json::Value, serde_json::Value)> = match &domain_val {
                Value::Json(serde_json::Value::Array(a)) => {
                    a.iter().enumerate().map(|(i, v)| (serde_json::json!(i), v.clone())).collect()
                }
                Value::Json(serde_json::Value::Object(m)) => {
                    m.iter().map(|(k, v)| (serde_json::json!(k), v.clone())).collect()
                }
                Value::Set(s) => {
                    s.iter().enumerate().map(|(i, v)| (serde_json::json!(i), v.clone())).collect()
                }
                _ => return vec![],
            };

            let mut results = Vec::new();
            for (k, v) in items {
                let mut b = bindings.clone();
                if let Some(key_term) = key {
                    let new_b = unify_value(key_term, Value::Json(k), b, ctx);
                    if new_b.is_empty() { continue; }
                    b = new_b.into_iter().next().unwrap();
                }
                let new_b = unify_value(value, Value::Json(v), b, ctx);
                results.extend(new_b);
            }
            results
        }

        Expr::With { base, targets } => {
            // Apply `with` overrides to the context, then eval base
            let mut new_ctx = ctx.clone();
            for target in targets {
                let val = eval_term(&target.value, bindings, ctx);
                let json_val = val.to_json_lossy();
                new_ctx = new_ctx.with_path_override(&target.path, json_val);
            }
            eval_expr(base, bindings, &new_ctx)
        }
    }
}

// ─── Term evaluation ──────────────────────────────────────────────────────────

pub fn eval_term(term: &Term, bindings: &Bindings, ctx: &EvalCtx) -> Value {
    match term {
        Term::Null => Value::null(),
        Term::Bool(b) => Value::bool(*b),
        Term::Number(n) => {
            if let Ok(i) = n.parse::<i64>() {
                Value::number_i64(i)
            } else if let Ok(f) = n.parse::<f64>() {
                Value::number_f64(f)
            } else {
                Value::Undefined
            }
        }
        Term::String(s) => Value::string(s.clone()),
        Term::Wildcard => Value::Undefined,

        Term::Var(name) => {
            match name.as_str() {
                "data" => Value::Json(ctx.data.clone()),
                "input" => Value::Json(ctx.input.clone()),
                "true" => Value::bool(true),
                "false" => Value::bool(false),
                "null" => Value::null(),
                _ => {
                    if let Some(v) = bindings.get(name) {
                        v.clone()
                    } else {
                        // Try to evaluate as a rule in the current module set
                        eval_rule_by_name(name, bindings, ctx)
                    }
                }
            }
        }

        Term::Ref(base, args) => {
            eval_ref(base, args, bindings, ctx)
        }

        Term::Array(items) => {
            let mut arr = Vec::new();
            for item in items {
                let v = eval_term(item, bindings, ctx);
                if v.is_undefined() { return Value::Undefined; }
                arr.push(v.to_json_lossy());
            }
            Value::array(arr)
        }

        Term::Object(kvs) => {
            let mut m = serde_json::Map::new();
            for (k, v) in kvs {
                let kv = eval_term(k, bindings, ctx);
                let vv = eval_term(v, bindings, ctx);
                let key = match kv {
                    Value::Json(serde_json::Value::String(s)) => s,
                    Value::Json(j) => j.to_string(),
                    _ => return Value::Undefined,
                };
                m.insert(key, vv.to_json_lossy());
            }
            Value::object(m)
        }

        Term::Set(items) => {
            let mut set = Vec::new();
            for item in items {
                let v = eval_term(item, bindings, ctx);
                if v.is_undefined() { return Value::Undefined; }
                let j = v.to_json_lossy();
                if !set.contains(&j) { set.push(j); }
            }
            Value::Set(set)
        }

        Term::ArrayCompr { term, body } => {
            let solutions = eval_body(body, bindings, ctx);
            let mut arr = Vec::new();
            for sol in solutions {
                let v = eval_term(term, &sol, ctx);
                if !v.is_undefined() {
                    arr.push(v.to_json_lossy());
                }
            }
            Value::array(arr)
        }

        Term::SetCompr { term, body } => {
            let solutions = eval_body(body, bindings, ctx);
            let mut set = Vec::new();
            for sol in solutions {
                let v = eval_term(term, &sol, ctx);
                if !v.is_undefined() {
                    let j = v.to_json_lossy();
                    if !set.contains(&j) { set.push(j); }
                }
            }
            Value::Set(set)
        }

        Term::ObjectCompr { key, value, body } => {
            let solutions = eval_body(body, bindings, ctx);
            let mut m = serde_json::Map::new();
            for sol in solutions {
                let kv = eval_term(key, &sol, ctx);
                let vv = eval_term(value, &sol, ctx);
                if !kv.is_undefined() && !vv.is_undefined() {
                    let ks = match kv {
                        Value::Json(serde_json::Value::String(s)) => s,
                        Value::Json(j) => j.to_string(),
                        _ => continue,
                    };
                    m.insert(ks, vv.to_json_lossy());
                }
            }
            Value::object(m)
        }

        Term::Call { func, args } => {
            eval_call(func, args, bindings, ctx)
        }
    }
}

fn eval_ref(base: &Term, ref_args: &[RefArg], bindings: &Bindings, ctx: &EvalCtx) -> Value {
    let mut current = eval_term(base, bindings, ctx);

    for ref_arg in ref_args {
        current = match ref_arg {
            RefArg::Field(name) => {
                match &current {
                    Value::Json(serde_json::Value::Object(m)) => {
                        if let Some(v) = m.get(name) {
                            Value::Json(v.clone())
                        } else {
                            // Try to evaluate as a rule
                            Value::Undefined
                        }
                    }
                    _ => Value::Undefined,
                }
            }
            RefArg::Index(idx_term) => {
                let idx = eval_term(idx_term, bindings, ctx);
                match (&current, idx) {
                    (Value::Json(serde_json::Value::Array(a)), Value::Json(serde_json::Value::Number(n))) => {
                        if let Some(i) = n.as_u64() {
                            a.get(i as usize).cloned().map(Value::Json).unwrap_or(Value::Undefined)
                        } else {
                            Value::Undefined
                        }
                    }
                    (Value::Json(serde_json::Value::Object(m)), Value::Json(serde_json::Value::String(k))) => {
                        m.get(&k).cloned().map(Value::Json).unwrap_or(Value::Undefined)
                    }
                    (Value::Json(serde_json::Value::Object(m)), Value::Json(serde_json::Value::Number(n))) => {
                        let k = n.to_string();
                        m.get(&k).cloned().map(Value::Json).unwrap_or(Value::Undefined)
                    }
                    (_, Value::Undefined) => Value::Undefined, // wildcard iteration
                    _ => Value::Undefined,
                }
            }
        };
        if current.is_undefined() { break; }
    }
    current
}

fn eval_call(func: &Term, args: &[Term], bindings: &Bindings, ctx: &EvalCtx) -> Value {
    // Collect function name path
    let func_path = term_to_path(func);
    let func_name = func_path.join(".");

    // Evaluate arguments
    let arg_vals: Vec<Value> = args.iter().map(|a| eval_term(a, bindings, ctx)).collect();

    // Check arithmetic/comparison operators expressed as function calls
    match func_name.as_str() {
        "plus" | "+" => return arith_op(&arg_vals, |a, b| a + b),
        "minus" | "-" => return arith_op(&arg_vals, |a, b| a - b),
        "mul" | "*" => return arith_op(&arg_vals, |a, b| a * b),
        "div" | "/" => return arith_op(&arg_vals, |a, b| if b != 0.0 { a / b } else { f64::INFINITY }),
        "rem" | "%" => return arith_op(&arg_vals, |a, b| a % b),
        "lt" | "<" => return compare_op(&arg_vals, std::cmp::Ordering::Less),
        "gt" | ">" => return compare_op(&arg_vals, std::cmp::Ordering::Greater),
        "lte" | "<=" => return compare_op_le(&arg_vals),
        "gte" | ">=" => return compare_op_ge(&arg_vals),
        "equal" | "==" => {
            let eq = arg_vals.get(0) == arg_vals.get(1);
            return Value::bool(eq);
        }
        "neq" | "!=" => {
            let neq = arg_vals.get(0) != arg_vals.get(1);
            return Value::bool(neq);
        }
        _ => {}
    }

    // Check registered builtins
    if let Some(result) = ctx.builtins.call(&func_name, &arg_vals) {
        return result.unwrap_or(Value::Undefined);
    }

    // Try to find user-defined function rule
    eval_function_rule(&func_path, &arg_vals, bindings, ctx)
}

fn arith_op(args: &[Value], op: impl Fn(f64, f64) -> f64) -> Value {
    let a = args.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
    let b = args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let result = op(a, b);
    if result.fract() == 0.0 && result.abs() < 9.007e15 {
        Value::number_i64(result as i64)
    } else {
        Value::number_f64(result)
    }
}

fn compare_op(args: &[Value], expected: std::cmp::Ordering) -> Value {
    let a = args.first().and_then(|v| v.as_json()).cloned().unwrap_or(serde_json::Value::Null);
    let b = args.get(1).and_then(|v| v.as_json()).cloned().unwrap_or(serde_json::Value::Null);
    Value::bool(super::value::json_cmp(&a, &b) == expected)
}

fn compare_op_le(args: &[Value]) -> Value {
    let a = args.first().and_then(|v| v.as_json()).cloned().unwrap_or(serde_json::Value::Null);
    let b = args.get(1).and_then(|v| v.as_json()).cloned().unwrap_or(serde_json::Value::Null);
    let ord = super::value::json_cmp(&a, &b);
    Value::bool(ord != std::cmp::Ordering::Greater)
}

fn compare_op_ge(args: &[Value]) -> Value {
    let a = args.first().and_then(|v| v.as_json()).cloned().unwrap_or(serde_json::Value::Null);
    let b = args.get(1).and_then(|v| v.as_json()).cloned().unwrap_or(serde_json::Value::Null);
    let ord = super::value::json_cmp(&a, &b);
    Value::bool(ord != std::cmp::Ordering::Less)
}

fn term_to_path(term: &Term) -> Vec<String> {
    match term {
        Term::Var(s) => vec![s.clone()],
        Term::Ref(base, args) => {
            let mut path = term_to_path(base);
            for arg in args {
                match arg {
                    RefArg::Field(f) => path.push(f.clone()),
                    RefArg::Index(Term::String(s)) => path.push(s.clone()),
                    _ => {}
                }
            }
            path
        }
        _ => vec![],
    }
}

// ─── Rule evaluation ──────────────────────────────────────────────────────────

fn eval_rule_by_name(name: &str, bindings: &Bindings, ctx: &EvalCtx) -> Value {
    // Check all loaded modules for rules matching this name
    for module in ctx.modules.values() {
        let pkg_prefix = &module.package.path;
        for rule in &module.rules {
            if rule.head.name == name {
                if let Some(v) = eval_rule(rule, pkg_prefix, bindings, ctx) {
                    return v;
                }
            }
        }
    }
    Value::Undefined
}

fn eval_function_rule(
    func_path: &[String],
    arg_vals: &[Value],
    bindings: &Bindings,
    ctx: &EvalCtx,
) -> Value {
    // Check cycle detection
    let call_key = func_path.join(".");
    if ctx.call_stack.contains(&call_key) {
        return Value::Undefined;
    }

    for module in ctx.modules.values() {
        for rule in &module.rules {
            if rule.head.name != *func_path.last().unwrap_or(&String::new()) {
                continue;
            }
            if rule.head.args.is_empty() { continue; }
            if rule.head.args.len() != arg_vals.len() { continue; }

            // Unify function parameters with argument values
            let mut call_bindings = HashMap::new();
            let mut ok = true;
            for (param, arg_val) in rule.head.args.iter().zip(arg_vals.iter()) {
                let solutions = unify_value(param, arg_val.clone(), call_bindings.clone(), ctx);
                if solutions.is_empty() { ok = false; break; }
                call_bindings = solutions.into_iter().next().unwrap();
            }
            if !ok { continue; }

            let mut call_ctx = ctx.clone();
            call_ctx.call_stack.push(call_key.clone());

            // Evaluate function bodies
            for body in &rule.bodies {
                let solutions = eval_body(body, &call_bindings, &call_ctx);
                if !solutions.is_empty() {
                    // Function succeeded; return head value
                    let sol = &solutions[0];
                    if let Some(val_term) = &rule.head.value {
                        return eval_term(val_term, sol, &call_ctx);
                    }
                    return Value::bool(true);
                }
            }
        }
    }
    Value::Undefined
}

fn eval_rule(
    rule: &Rule,
    _pkg_prefix: &[String],
    bindings: &Bindings,
    ctx: &EvalCtx,
) -> Option<Value> {
    if rule.is_default {
        // Only use default if no other rule produces a value
        if let Some(val_term) = &rule.head.value {
            return Some(eval_term(val_term, bindings, ctx));
        }
        return Some(Value::bool(false));
    }

    // Partial set rule: collect all values from all bodies
    if rule.head.key.is_some() && rule.head.value.is_none() {
        let mut set = Vec::new();
        for body in &rule.bodies {
            let solutions = eval_body(body, bindings, ctx);
            for sol in solutions {
                if let Some(key_term) = &rule.head.key {
                    let v = eval_term(key_term, &sol, ctx);
                    if !v.is_undefined() {
                        let j = v.to_json_lossy();
                        if !set.contains(&j) { set.push(j); }
                    }
                }
            }
        }
        return Some(Value::Set(set));
    }

    // Partial object rule: collect key-value pairs
    if rule.head.key.is_some() && rule.head.value.is_some() {
        let mut m = serde_json::Map::new();
        for body in &rule.bodies {
            let solutions = eval_body(body, bindings, ctx);
            for sol in solutions {
                let kv = eval_term(rule.head.key.as_ref().unwrap(), &sol, ctx);
                let vv = eval_term(rule.head.value.as_ref().unwrap(), &sol, ctx);
                if !kv.is_undefined() && !vv.is_undefined() {
                    let ks = match kv {
                        Value::Json(serde_json::Value::String(s)) => s,
                        Value::Json(j) => j.to_string(),
                        _ => continue,
                    };
                    m.insert(ks, vv.to_json_lossy());
                }
            }
        }
        return Some(Value::object(m));
    }

    // Complete rule: evaluate bodies, take first truthy result
    for body in &rule.bodies {
        let solutions = eval_body(body, bindings, ctx);
        if !solutions.is_empty() {
            let sol = &solutions[0];
            if let Some(val_term) = &rule.head.value {
                return Some(eval_term(val_term, sol, ctx));
            }
            return Some(Value::bool(true));
        }
    }

    // Else rules
    for else_rule in &rule.else_rules {
        let body = &else_rule.body;
        if body.is_empty() {
            if let Some(val_term) = &else_rule.value {
                return Some(eval_term(val_term, bindings, ctx));
            }
            return Some(Value::bool(true));
        }
        let solutions = eval_body(body, bindings, ctx);
        if !solutions.is_empty() {
            if let Some(val_term) = &else_rule.value {
                return Some(eval_term(val_term, &solutions[0], ctx));
            }
            return Some(Value::bool(true));
        }
    }

    None
}

// ─── Unification ──────────────────────────────────────────────────────────────

/// Unify two terms, extending bindings if successful.
pub fn unify(left: &Term, right: &Term, bindings: Bindings, ctx: &EvalCtx) -> Vec<Bindings> {
    // Evaluate both sides as much as possible
    let lv = eval_term(left, &bindings, ctx);
    let rv = eval_term(right, &bindings, ctx);

    match (left, right, &lv, &rv) {
        // Left is unbound variable
        (Term::Var(name), _, Value::Undefined, _) if name != "data" && name != "input" => {
            if rv.is_undefined() { return vec![]; }
            let mut b = bindings;
            b.insert(name.clone(), rv);
            vec![b]
        }
        // Right is unbound variable
        (_, Term::Var(name), _, Value::Undefined) if name != "data" && name != "input" => {
            if lv.is_undefined() { return vec![]; }
            let mut b = bindings;
            b.insert(name.clone(), lv);
            vec![b]
        }
        // Both concrete: check equality
        (_, _, lv, rv) => {
            if lv == rv && !lv.is_undefined() {
                vec![bindings]
            } else {
                vec![]
            }
        }
    }
}

/// Bind a term (typically a variable) to a value.
fn unify_value(term: &Term, value: Value, bindings: Bindings, ctx: &EvalCtx) -> Vec<Bindings> {
    match term {
        Term::Var(name) => {
            if name == "data" || name == "input" || name == "_" {
                return vec![bindings];
            }
            if let Some(existing) = bindings.get(name) {
                if existing == &value {
                    return vec![bindings];
                } else if existing.is_undefined() {
                    let mut b = bindings;
                    b.insert(name.clone(), value);
                    return vec![b];
                } else {
                    return vec![];
                }
            }
            let mut b = bindings;
            b.insert(name.clone(), value);
            vec![b]
        }
        Term::Wildcard => vec![bindings],
        _ => {
            let existing = eval_term(term, &bindings, ctx);
            if existing == value { vec![bindings] } else { vec![] }
        }
    }
}
