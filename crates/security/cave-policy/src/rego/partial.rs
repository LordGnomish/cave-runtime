// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Partial-evaluation support: residual-query analysis over a parsed body.
//!
//! Upstream: open-policy-agent/opa v1.16.2 — topdown/partial.go.
//!
//! These helpers classify each conjunct of a query body as either residual
//! (depends on an `unknown` root or a variable tainted by one) or ground, and
//! render the surviving conjuncts back to Rego-ish text for the residual query.
//!
//! Scope (kept honestly PARTIAL): residual generation for a top-level
//! conjunctive query. Full OPA PE — rule inlining into support modules,
//! negation residuals, and known-variable substitution into residual terms —
//! remains deferred.

use super::ast::{CompareOp, Expr, RefArg, Term};
use std::collections::HashSet;

/// Collect the leftmost root identifier of every variable/reference used by `expr`.
pub fn referenced_roots(expr: &Expr) -> HashSet<String> {
    let mut roots = HashSet::new();
    expr_roots(expr, &mut roots);
    roots
}

fn expr_roots(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Term(t) => term_roots(t, out),
        Expr::Unify(a, b) | Expr::Assign(a, b) => {
            term_roots(a, out);
            term_roots(b, out);
        }
        Expr::Compare { lhs, rhs, .. } => {
            term_roots(lhs, out);
            term_roots(rhs, out);
        }
        Expr::Not(e) => expr_roots(e, out),
        Expr::NotBody(body) => body.iter().for_each(|e| expr_roots(e, out)),
        Expr::Every { domain, body, .. } => {
            term_roots(domain, out);
            body.iter().for_each(|e| expr_roots(e, out));
        }
        Expr::Some(_) => {}
        Expr::SomeIn { key, value, domain } => {
            if let Some(k) = key {
                term_roots(k, out);
            }
            term_roots(value, out);
            term_roots(domain, out);
        }
        Expr::With { base, targets } => {
            expr_roots(base, out);
            targets.iter().for_each(|t| term_roots(&t.value, out));
        }
    }
}

fn term_roots(term: &Term, out: &mut HashSet<String>) {
    match term {
        Term::Var(n) => {
            out.insert(n.clone());
        }
        Term::Ref(base, args) => {
            term_roots(base, out);
            for a in args {
                if let RefArg::Index(t) = a {
                    term_roots(t, out);
                }
            }
        }
        Term::Array(items) | Term::Set(items) => items.iter().for_each(|t| term_roots(t, out)),
        Term::Object(kvs) => {
            for (k, v) in kvs {
                term_roots(k, out);
                term_roots(v, out);
            }
        }
        Term::Call { func, args } => {
            term_roots(func, out);
            args.iter().for_each(|t| term_roots(t, out));
        }
        Term::ArrayCompr { term, body } | Term::SetCompr { term, body } => {
            term_roots(term, out);
            body.iter().for_each(|e| expr_roots(e, out));
        }
        Term::ObjectCompr { key, value, body } => {
            term_roots(key, out);
            term_roots(value, out);
            body.iter().for_each(|e| expr_roots(e, out));
        }
        Term::Null | Term::Bool(_) | Term::Number(_) | Term::String(_) | Term::Wildcard => {}
    }
}

/// Does `expr` depend on an unknown root or an already-tainted variable?
pub fn is_residual(expr: &Expr, unknowns: &HashSet<String>, tainted: &HashSet<String>) -> bool {
    referenced_roots(expr)
        .iter()
        .any(|r| unknowns.contains(r) || tainted.contains(r))
}

/// Record variables bound (by `:=` / `=`) inside a residual conjunct so that
/// downstream conjuncts referencing them are also treated as residual.
pub fn mark_tainted(expr: &Expr, tainted: &mut HashSet<String>) {
    match expr {
        Expr::Assign(target, _) => {
            if let Term::Var(n) = target {
                tainted.insert(n.clone());
            }
        }
        Expr::Unify(a, b) => {
            if let Term::Var(n) = a {
                tainted.insert(n.clone());
            }
            if let Term::Var(n) = b {
                tainted.insert(n.clone());
            }
        }
        Expr::SomeIn { key, value, .. } => {
            if let Some(Term::Var(n)) = key {
                tainted.insert(n.clone());
            }
            if let Term::Var(n) = value {
                tainted.insert(n.clone());
            }
        }
        _ => {}
    }
}

// ─── Rendering ──────────────────────────────────────────────────────────────

/// Render a residual conjunct as a JSON string (Rego-ish source text).
pub fn expr_to_json(expr: &Expr) -> serde_json::Value {
    serde_json::Value::String(expr_to_string(expr))
}

pub fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::Term(t) => term_to_string(t),
        Expr::Unify(a, b) => format!("{} = {}", term_to_string(a), term_to_string(b)),
        Expr::Assign(a, b) => format!("{} := {}", term_to_string(a), term_to_string(b)),
        Expr::Compare { op, lhs, rhs } => {
            format!("{} {} {}", term_to_string(lhs), op_str(op), term_to_string(rhs))
        }
        Expr::Not(e) => format!("not {}", expr_to_string(e)),
        Expr::NotBody(_) => "not { ... }".into(),
        Expr::Every { value, domain, .. } => {
            format!("every {value} in {} {{ ... }}", term_to_string(domain))
        }
        Expr::Some(vars) => format!("some {}", vars.join(", ")),
        Expr::SomeIn { value, domain, .. } => {
            format!("some {} in {}", term_to_string(value), term_to_string(domain))
        }
        Expr::With { base, .. } => format!("{} with ...", expr_to_string(base)),
    }
}

fn op_str(op: &CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "==",
        CompareOp::Ne => "!=",
        CompareOp::Lt => "<",
        CompareOp::Le => "<=",
        CompareOp::Gt => ">",
        CompareOp::Ge => ">=",
    }
}

fn term_to_string(term: &Term) -> String {
    match term {
        Term::Null => "null".into(),
        Term::Bool(b) => b.to_string(),
        Term::Number(n) => n.clone(),
        Term::String(s) => format!("\"{s}\""),
        Term::Var(n) => n.clone(),
        Term::Wildcard => "_".into(),
        Term::Ref(base, args) => {
            let mut s = term_to_string(base);
            for a in args {
                match a {
                    RefArg::Field(f) => {
                        s.push('.');
                        s.push_str(f);
                    }
                    RefArg::Index(t) => {
                        s.push('[');
                        s.push_str(&term_to_string(t));
                        s.push(']');
                    }
                }
            }
            s
        }
        Term::Array(items) => format!(
            "[{}]",
            items.iter().map(term_to_string).collect::<Vec<_>>().join(", ")
        ),
        Term::Set(items) => format!(
            "{{{}}}",
            items.iter().map(term_to_string).collect::<Vec<_>>().join(", ")
        ),
        Term::Object(kvs) => format!(
            "{{{}}}",
            kvs.iter()
                .map(|(k, v)| format!("{}: {}", term_to_string(k), term_to_string(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Term::Call { func, args } => format!(
            "{}({})",
            term_to_string(func),
            args.iter().map(term_to_string).collect::<Vec<_>>().join(", ")
        ),
        Term::ArrayCompr { .. } => "[ ... | ... ]".into(),
        Term::SetCompr { .. } => "{ ... | ... }".into(),
        Term::ObjectCompr { .. } => "{ ...: ... | ... }".into(),
    }
}
