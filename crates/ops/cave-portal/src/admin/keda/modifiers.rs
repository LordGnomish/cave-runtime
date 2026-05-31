// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/keda/modifiers` — ScalingModifiers formula playground.
//!
//! KEDA's `spec.advanced.scalingModifiers.formula` is evaluated with the
//! `github.com/expr-lang/expr` engine over a `map[string]float64` of
//! trigger-name → metric value (pkg/scaling/modifiers/formula.go). This
//! page lets an operator paste a formula plus a set of trigger values and
//! see the composite metric the controller would compute — backed by the
//! real `cave_keda::eval_formula` port, so the playground and the runtime
//! share one evaluator.

use std::collections::BTreeMap;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// Parse a `name=value,name=value` trigger string into the float map the
/// formula engine expects. Malformed pairs are skipped so a partial input
/// still previews.
pub fn parse_triggers(s: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    for pair in s.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        if let Some((name, val)) = pair.split_once('=') {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if let Ok(v) = val.trim().parse::<f64>() {
                if v.is_finite() {
                    out.insert(name.to_string(), v);
                }
            }
        }
    }
    out
}

/// Evaluate `formula` against `triggers` using the shared cave-keda
/// engine. Returns the composite metric or the engine's error string.
pub fn evaluate(formula: &str, triggers: &BTreeMap<String, f64>) -> Result<f64, String> {
    cave_keda::eval_formula(formula, triggers).map_err(|e| e.to_string())
}

/// Render the playground. When `formula` is provided it is evaluated and
/// the result (or error) is shown; otherwise a worked example is shown.
pub fn render(
    ctx: &RequestCtx,
    formula: Option<&str>,
    triggers: Option<&str>,
) -> Result<String, Error> {
    ctx.authorise(Permission::KedaRead)?;

    let formula_in = formula.unwrap_or("(api + worker) / 2").trim().to_string();
    let triggers_in = triggers.unwrap_or("api=30, worker=10").trim().to_string();
    let parsed = parse_triggers(&triggers_in);

    let result_html = match evaluate(&formula_in, &parsed) {
        Ok(v) => format!(
            r#"<div class="mt-4 p-3 rounded bg-green-50 border border-green-300">
<span class="text-sm text-gray-600">composite metric</span>
<div class="text-2xl font-mono font-semibold text-green-800">{v}</div></div>"#,
            v = v
        ),
        Err(e) => format!(
            r#"<div class="mt-4 p-3 rounded bg-red-50 border border-red-300">
<span class="text-sm text-gray-600">formula error</span>
<div class="font-mono text-red-800">{e}</div></div>"#,
            e = escape(&e)
        ),
    };

    let trigger_rows: String = parsed
        .iter()
        .map(|(k, v)| format!("<li><code>{}</code> = {}</li>", escape(k), v))
        .collect();

    let body = format!(
        r#"<h2 class="text-lg font-semibold mb-2">ScalingModifiers formula playground</h2>
<p class="text-sm text-gray-600 mb-3">Backed by the real <code>cave_keda::eval_formula</code>
port of <code>pkg/scaling/modifiers/formula.go</code> (github.com/expr-lang/expr). Supports
arithmetic, comparison, logical <code>&amp;&amp; || !</code>, ternary <code>?:</code>, array
literals and the builtins <code>sum avg min max len abs ceil floor round</code> plus
<code>count(arr, {{# &gt; k}})</code>.</p>
<form method="get" action="/admin/keda/modifiers" class="grid gap-2 max-w-2xl">
  <input type="hidden" name="tenant_id" value="{tenant}">
  <label class="text-sm">formula
    <input class="w-full border rounded px-2 py-1 font-mono" name="formula" value="{formula}">
  </label>
  <label class="text-sm">triggers (name=value, comma-separated)
    <input class="w-full border rounded px-2 py-1 font-mono" name="triggers" value="{triggers}">
  </label>
  <button class="px-3 py-1 rounded bg-blue-600 text-white w-32" type="submit">Evaluate</button>
</form>
{result}
<div class="mt-3 text-sm text-gray-600">parsed triggers:<ul class="list-disc ml-5">{rows}</ul></div>"#,
        tenant = escape(ctx.tenant.as_str()),
        formula = escape(&formula_in),
        triggers = escape(&triggers_in),
        result = result_html,
        rows = trigger_rows,
    );

    Ok(page_shell_full(
        ctx,
        "/admin/keda/modifiers",
        &format!("keda · modifiers · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::KedaRead])
    }

    #[test]
    fn parse_triggers_handles_spaces_and_junk() {
        let m = parse_triggers("api=30, worker = 10 , bad, =5, x=nan");
        assert_eq!(m.get("api"), Some(&30.0));
        assert_eq!(m.get("worker"), Some(&10.0));
        assert_eq!(m.len(), 2, "malformed pairs are skipped");
    }

    #[test]
    fn evaluate_runs_the_real_engine() {
        let m = parse_triggers("api=30,worker=10");
        assert_eq!(evaluate("(api + worker) / 2", &m), Ok(20.0));
    }

    #[test]
    fn evaluate_count_predicate_matches_keda_docs() {
        let m = parse_triggers("a=0,b=2,c=3");
        assert_eq!(evaluate("count([a, b, c], {# > 1})", &m), Ok(2.0));
    }

    #[test]
    fn evaluate_surfaces_engine_error() {
        let m = parse_triggers("a=1");
        assert!(evaluate("ghost + 1", &m).is_err());
    }

    #[test]
    fn render_shows_computed_result() {
        let html = render(&ctx(), Some("(api + worker) / 2"), Some("api=30,worker=10")).unwrap();
        assert!(html.contains("composite metric"));
        assert!(html.contains("20"));
    }

    #[test]
    fn render_requires_keda_read() {
        let denied = RequestCtx::developer("acme", &[]);
        assert!(render(&denied, None, None).is_err());
    }
}
