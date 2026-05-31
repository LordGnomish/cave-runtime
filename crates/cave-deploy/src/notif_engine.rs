// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-crate notification engine — the ArgoCD-Application-coupled half of the
//! notifications stack: a `text/template` subset renderer, trigger-condition
//! (`when` / `oncePer`) evaluation, and the per-notification dedup keying that
//! ArgoCD records under `notified.notifications.argoproj.io`.
//!
//! Upstream: `argoproj/notifications-engine`
//!   * templates  — Go `text/template` rendered against `{app, context, ...}`
//!   * `pkg/triggers/service.go` — `ConditionResult.Key = "[i]." + base64url(sha1(when))`,
//!      `when` → bool, `oncePer` → string (expr-lang).
//!   * `pkg/controller/state.go` — dedup key
//!      `{oncePer}:{trigger}:{conditionKey}:{service}:{recipient}`.
//!
//! Out of scope (stays scope_cut to cave-notify): the 30+ destination service
//! plugins (Slack/Teams/PagerDuty/… SDKs) and the cross-process delivery queue.

use crate::error::DeployError;
use serde_json::Value;

// ════════════════════════════════════════════════════════════════════════════
//  Template renderer — a faithful subset of Go text/template
// ════════════════════════════════════════════════════════════════════════════

/// Render a notification template against a JSON context.
///
/// Supported actions (Go `text/template` subset):
///   * `{{.a.b.c}}`        — dotted-path field substitution
///   * `{{.x | upper}}`    — pipelines through `upper`/`lower`/`title`/`trim`
///   * `{{if .c}}…{{else}}…{{end}}` — conditional with Go truthiness
///
/// A missing path renders as Go's `<no value>` sentinel (matching the default
/// `missingkey=invalid` behaviour the ArgoCD notification controller uses).
pub fn render_template(tmpl: &str, ctx: &Value) -> Result<String, DeployError> {
    let toks = lex(tmpl)?;
    let nodes = parse(&toks)?;
    let mut out = String::new();
    render_nodes(&nodes, ctx, &mut out);
    Ok(out)
}

enum Tok {
    Text(String),
    Action(String),
}

fn lex(tmpl: &str) -> Result<Vec<Tok>, DeployError> {
    let bytes = tmpl.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    let mut text_start = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if i > text_start {
                toks.push(Tok::Text(tmpl[text_start..i].to_string()));
            }
            // find closing }}
            let rest = &tmpl[i + 2..];
            let close = rest.find("}}").ok_or_else(|| {
                DeployError::Invalid("notification template: unterminated '{{' action".into())
            })?;
            let action = rest[..close].trim().to_string();
            toks.push(Tok::Action(action));
            i = i + 2 + close + 2;
            text_start = i;
        } else {
            i += 1;
        }
    }
    if text_start < bytes.len() {
        toks.push(Tok::Text(tmpl[text_start..].to_string()));
    }
    Ok(toks)
}

enum Node {
    Text(String),
    Output(Pipeline),
    If {
        cond: Pipeline,
        then: Vec<Node>,
        els: Vec<Node>,
    },
}

/// A pipeline: a leading term piped through zero or more named functions.
struct Pipeline {
    term: Term,
    funcs: Vec<String>,
}

enum Term {
    Path(Vec<String>),
    Literal(String),
}

fn parse(toks: &[Tok]) -> Result<Vec<Node>, DeployError> {
    let mut pos = 0usize;
    let (nodes, consumed_all) = parse_until(toks, &mut pos, &[])?;
    if !consumed_all {
        return Err(DeployError::Invalid(
            "notification template: dangling '{{end}}' or '{{else}}'".into(),
        ));
    }
    Ok(nodes)
}

/// Parse nodes until one of `stop` keywords (or end of input). Returns the
/// parsed nodes and whether input was fully consumed (vs. stopped on a keyword,
/// which the caller — an `if` frame — consumes).
fn parse_until(
    toks: &[Tok],
    pos: &mut usize,
    stop: &[&str],
) -> Result<(Vec<Node>, bool), DeployError> {
    let mut nodes = Vec::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Tok::Text(t) => {
                nodes.push(Node::Text(t.clone()));
                *pos += 1;
            }
            Tok::Action(a) => {
                let kw = a.split_whitespace().next().unwrap_or("");
                if stop.contains(&kw) {
                    // leave the stop keyword for the caller
                    return Ok((nodes, false));
                }
                if kw == "if" {
                    *pos += 1;
                    let cond_src = a[2..].trim();
                    let cond = parse_pipeline(cond_src)?;
                    let (then, _) = parse_until(toks, pos, &["else", "end"])?;
                    let mut els = Vec::new();
                    // current token is else or end
                    if let Some(Tok::Action(nx)) = toks.get(*pos) {
                        if nx.trim() == "else" {
                            *pos += 1;
                            let (e, _) = parse_until(toks, pos, &["end"])?;
                            els = e;
                        }
                    }
                    // expect end
                    match toks.get(*pos) {
                        Some(Tok::Action(nx)) if nx.trim() == "end" => *pos += 1,
                        _ => {
                            return Err(DeployError::Invalid(
                                "notification template: '{{if}}' without '{{end}}'".into(),
                            ));
                        }
                    }
                    nodes.push(Node::If { cond, then, els });
                } else if kw == "else" || kw == "end" {
                    // not in our stop set but a control keyword → dangling
                    return Err(DeployError::Invalid(format!(
                        "notification template: unexpected '{{{{{}}}}}'",
                        kw
                    )));
                } else {
                    nodes.push(Node::Output(parse_pipeline(a)?));
                    *pos += 1;
                }
            }
        }
    }
    Ok((nodes, true))
}

fn parse_pipeline(src: &str) -> Result<Pipeline, DeployError> {
    let mut parts = src.split('|').map(str::trim);
    let head = parts.next().unwrap_or("");
    if head.is_empty() {
        return Err(DeployError::Invalid(
            "notification template: empty pipeline".into(),
        ));
    }
    let term = if let Some(path) = head.strip_prefix('.') {
        Term::Path(path.split('.').filter(|s| !s.is_empty()).map(String::from).collect())
    } else if (head.starts_with('"') && head.ends_with('"'))
        || (head.starts_with('\'') && head.ends_with('\''))
    {
        Term::Literal(head[1..head.len() - 1].to_string())
    } else if head == "." {
        Term::Path(vec![])
    } else {
        // bare word — treat as literal text (rare in ArgoCD templates)
        Term::Literal(head.to_string())
    };
    let funcs = parts.map(String::from).collect();
    Ok(Pipeline { term, funcs })
}

fn render_nodes(nodes: &[Node], ctx: &Value, out: &mut String) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Output(p) => out.push_str(&eval_pipeline_string(p, ctx)),
            Node::If { cond, then, els } => {
                if truthy(&resolve_term(&cond.term, ctx)) {
                    render_nodes(then, ctx, out);
                } else {
                    render_nodes(els, ctx, out);
                }
            }
        }
    }
}

fn resolve_term(term: &Term, ctx: &Value) -> Option<Value> {
    match term {
        Term::Literal(s) => Some(Value::String(s.clone())),
        Term::Path(segs) => {
            let mut cur = ctx;
            for s in segs {
                cur = cur.get(s)?;
            }
            Some(cur.clone())
        }
    }
}

fn eval_pipeline_string(p: &Pipeline, ctx: &Value) -> String {
    let mut s = match resolve_term(&p.term, ctx) {
        Some(Value::Null) | None => return go_no_value_through(&p.funcs),
        Some(v) => value_to_string(&v),
    };
    for f in &p.funcs {
        s = apply_func(f, &s);
    }
    s
}

/// Missing/null still flows through string funcs in Go, but the base is the
/// `<no value>` sentinel.
fn go_no_value_through(funcs: &[String]) -> String {
    let mut s = "<no value>".to_string();
    for f in funcs {
        s = apply_func(f, &s);
    }
    s
}

fn apply_func(name: &str, s: &str) -> String {
    match name {
        "upper" => s.to_uppercase(),
        "lower" => s.to_lowercase(),
        "trim" => s.trim().to_string(),
        "title" => title_case(s),
        _ => s.to_string(),
    }
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "<no value>".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Go template truthiness: false, 0, "", nil, and empty collections are false.
fn truthy(v: &Option<Value>) -> bool {
    match v {
        None => false,
        Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truthiness_matches_go() {
        assert!(truthy(&Some(json!(true))));
        assert!(!truthy(&Some(json!(false))));
        assert!(!truthy(&Some(json!(0))));
        assert!(truthy(&Some(json!(1))));
        assert!(!truthy(&Some(json!(""))));
        assert!(truthy(&Some(json!("x"))));
        assert!(!truthy(&None));
        assert!(!truthy(&Some(Value::Null)));
    }

    #[test]
    fn title_case_capitalizes_words() {
        assert_eq!(title_case("hello world"), "Hello World");
    }
}
