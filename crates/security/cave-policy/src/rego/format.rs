// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Canonical Rego pretty-printer — faithful port of OPA's `format` package.
//!
//! Upstream: open-policy-agent/opa v1.16.2 `format/format.go`.
//!
//! The `opa fmt` *CLI* (file discovery, `-w` in-place rewrite, unified-diff
//! output) is scope-cut to cave-cli; this module is the **format-package
//! core**: it takes a parsed [`Module`](super::ast::Module) and renders the one
//! canonical source string for it.
//!
//! ## Canonical form
//!
//! ```text
//! package a.b.c
//!
//! import data.alpha
//! import data.zeta
//!
//! default allow := false
//!
//! allow {
//! \tinput.role == "admin"
//! }
//! ```
//!
//! * Sections (package / imports / each rule) are separated by exactly one
//!   blank line; imports are sorted ascending by dotted path.
//! * Rule bodies are wrapped in `{ … }` with one **tab**-indented expression
//!   per line — this braced form round-trips through the in-crate parser, which
//!   is the engine's source of truth, and is exactly what `opa fmt` emits when
//!   rendering multi-line rule bodies.
//! * Exactly one space surrounds every binary operator (`=`, `:=`, `==`, `!=`,
//!   `<`, `<=`, `>`, `>=`) and collection elements are `, `-separated.
//! * Output always terminates in a single trailing newline.
//!
//! The two defining `opa fmt` invariants — **idempotence** (`fmt(fmt(x)) ==
//! fmt(x)`) and **re-parseability** (output is always valid Rego) — hold by
//! construction and are covered by `tests/format_tdd.rs`.

use super::ast::{
    Body, CompareOp, ElseRule, Expr, Module, RefArg, Rule, RuleHead, Term, WithTarget,
};
use crate::error::PolicyError;

const INDENT: &str = "\t";

/// Parse `src` and render its canonical form. Returns a parse error if the
/// source is not valid Rego.
pub fn format_source(src: &str) -> Result<String, PolicyError> {
    let module = super::parser::parse_module(src)?;
    Ok(format_module(&module))
}

/// Render a parsed [`Module`] to its canonical source string.
pub fn format_module(module: &Module) -> String {
    let mut out = String::new();

    // ── package ───────────────────────────────────────────────────────────
    out.push_str("package ");
    out.push_str(&module.package.path.join("."));
    out.push('\n');

    // ── imports (sorted, own block) ───────────────────────────────────────
    if !module.imports.is_empty() {
        out.push('\n');
        let mut imports: Vec<String> = module
            .imports
            .iter()
            .map(|imp| {
                let path = imp.path.join(".");
                match &imp.alias {
                    Some(a) => format!("import {path} as {a}"),
                    None => format!("import {path}"),
                }
            })
            .collect();
        imports.sort();
        for line in imports {
            out.push_str(&line);
            out.push('\n');
        }
    }

    // ── rules (each its own blank-line-separated block) ───────────────────
    for rule in &module.rules {
        out.push('\n');
        fmt_rule(&mut out, rule);
    }

    out
}

fn fmt_rule(out: &mut String, rule: &Rule) {
    if rule.is_default {
        // `default <head> := <value>` — defaults never carry a body.
        out.push_str("default ");
        out.push_str(&fmt_head_sig(&rule.head));
        if let Some(v) = &rule.head.value {
            out.push_str(" := ");
            out.push_str(&fmt_term(v));
        }
        out.push('\n');
        return;
    }

    let head = fmt_head(&rule.head);

    // A rule with no body is a bare fact: `p := 1` or `p`.
    if rule.bodies.is_empty() {
        out.push_str(&head);
        out.push('\n');
    } else {
        // Each body is a separate clause sharing the same head (disjunction /
        // incremental definition), rendered as its own braced block.
        for (i, body) in rule.bodies.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            fmt_braced_clause(out, &head, body);
        }
    }

    // `else` clauses chain under the rule.
    for er in &rule.else_rules {
        fmt_else(out, er);
    }
}

fn fmt_else(out: &mut String, er: &ElseRule) {
    out.push_str("else");
    if let Some(v) = &er.value {
        out.push_str(" := ");
        out.push_str(&fmt_term(v));
    }
    fmt_braced_body(out, &er.body, " ");
    out.push('\n');
}

/// Render `<head> {\n\t<expr>\n}` (or omit braces when the body is empty).
fn fmt_braced_clause(out: &mut String, head: &str, body: &Body) {
    out.push_str(head);
    if body.is_empty() {
        out.push('\n');
        return;
    }
    fmt_braced_body(out, body, " ");
    out.push('\n');
}

/// Append ` {` + newline-separated indented exprs + `}` to `out`. `lead` is the
/// separator placed before the opening brace (a single space).
fn fmt_braced_body(out: &mut String, body: &Body, lead: &str) {
    out.push_str(lead);
    out.push('{');
    out.push('\n');
    for expr in body {
        out.push_str(INDENT);
        out.push_str(&fmt_expr(expr));
        out.push('\n');
    }
    out.push('}');
}

/// Full rule head including an assigned value (`p := 1`, `f(x) := y`).
fn fmt_head(head: &RuleHead) -> String {
    let mut s = fmt_head_sig(head);
    if let Some(v) = &head.value {
        s.push_str(" := ");
        s.push_str(&fmt_term(v));
    }
    s
}

/// Head signature without the assigned value: `name`, `name[key]`, `f(a, b)`.
fn fmt_head_sig(head: &RuleHead) -> String {
    let mut s = String::new();
    s.push_str(&head.name);
    if !head.args.is_empty() {
        let args: Vec<String> = head.args.iter().map(fmt_term).collect();
        s.push('(');
        s.push_str(&args.join(", "));
        s.push(')');
    }
    if let Some(key) = &head.key {
        s.push('[');
        s.push_str(&fmt_term(key));
        s.push(']');
    }
    s
}

fn cmp_str(op: CompareOp) -> &'static str {
    match op {
        CompareOp::Eq => "==",
        CompareOp::Ne => "!=",
        CompareOp::Lt => "<",
        CompareOp::Le => "<=",
        CompareOp::Gt => ">",
        CompareOp::Ge => ">=",
    }
}

fn fmt_expr(expr: &Expr) -> String {
    match expr {
        Expr::Term(t) => fmt_term(t),
        Expr::Unify(a, b) => format!("{} = {}", fmt_term(a), fmt_term(b)),
        Expr::Assign(a, b) => format!("{} := {}", fmt_term(a), fmt_term(b)),
        Expr::Compare { op, lhs, rhs } => {
            format!("{} {} {}", fmt_term(lhs), cmp_str(*op), fmt_term(rhs))
        }
        Expr::Not(inner) => format!("not {}", fmt_expr(inner)),
        Expr::NotBody(body) => {
            // Negation over a body — render its single line(s) inline.
            let parts: Vec<String> = body.iter().map(fmt_expr).collect();
            format!("not {}", parts.join("; "))
        }
        Expr::Every {
            key,
            value,
            domain,
            body,
        } => {
            let head = match key {
                Some(k) => format!("every {k}, {value} in {}", fmt_term(domain)),
                None => format!("every {value} in {}", fmt_term(domain)),
            };
            let mut s = head;
            let mut buf = String::new();
            fmt_braced_body(&mut buf, body, " ");
            s.push_str(&buf);
            s
        }
        Expr::Some(vars) => format!("some {}", vars.join(", ")),
        Expr::SomeIn { key, value, domain } => match key {
            Some(k) => format!(
                "some {}, {} in {}",
                fmt_term(k),
                fmt_term(value),
                fmt_term(domain)
            ),
            None => format!("some {} in {}", fmt_term(value), fmt_term(domain)),
        },
        Expr::With { base, targets } => {
            let mut s = fmt_expr(base);
            for t in targets {
                s.push_str(&fmt_with_target(t));
            }
            s
        }
    }
}

fn fmt_with_target(t: &WithTarget) -> String {
    format!(" with {} as {}", t.path.join("."), fmt_term(&t.value))
}

fn fmt_term(term: &Term) -> String {
    match term {
        Term::Null => "null".to_string(),
        Term::Bool(b) => b.to_string(),
        Term::Number(n) => n.clone(),
        Term::String(s) => fmt_string(s),
        Term::Var(v) => v.clone(),
        Term::Wildcard => "_".to_string(),
        Term::Ref(base, args) => {
            let mut s = fmt_term(base);
            for arg in args {
                match arg {
                    RefArg::Field(name) => {
                        s.push('.');
                        s.push_str(name);
                    }
                    RefArg::Index(t) => {
                        s.push('[');
                        s.push_str(&fmt_term(t));
                        s.push(']');
                    }
                }
            }
            s
        }
        Term::Array(items) => {
            let parts: Vec<String> = items.iter().map(fmt_term).collect();
            format!("[{}]", parts.join(", "))
        }
        Term::Set(items) => {
            if items.is_empty() {
                // The empty set literal must use `set()`; `{}` is the empty
                // object. This is exactly how `opa fmt` disambiguates.
                "set()".to_string()
            } else {
                let parts: Vec<String> = items.iter().map(fmt_term).collect();
                format!("{{{}}}", parts.join(", "))
            }
        }
        Term::Object(kvs) => {
            let parts: Vec<String> = kvs
                .iter()
                .map(|(k, v)| format!("{}: {}", fmt_term(k), fmt_term(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Term::Call { func, args } => {
            let parts: Vec<String> = args.iter().map(fmt_term).collect();
            format!("{}({})", fmt_term(func), parts.join(", "))
        }
        Term::ArrayCompr { term, body } => {
            format!("[{} | {}]", fmt_term(term), fmt_compr_body(body))
        }
        Term::SetCompr { term, body } => {
            format!("{{{} | {}}}", fmt_term(term), fmt_compr_body(body))
        }
        Term::ObjectCompr { key, value, body } => {
            format!(
                "{{{}: {} | {}}}",
                fmt_term(key),
                fmt_term(value),
                fmt_compr_body(body)
            )
        }
    }
}

/// Comprehension bodies render inline with `; ` between expressions.
fn fmt_compr_body(body: &Body) -> String {
    body.iter().map(fmt_expr).collect::<Vec<_>>().join("; ")
}

/// Render a Rego string literal with the standard JSON-style escapes.
fn fmt_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
