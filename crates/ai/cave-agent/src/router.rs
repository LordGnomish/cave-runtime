// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP + portal surface for cave-agent. Mounts `/api/agent/*` JSON endpoints
//! and a self-contained `/admin/agent` operator page. The handler bodies are
//! thin wrappers over the pure cores ([`tool_catalog_json`], [`invoke_json`],
//! [`plan_json`], [`admin_page_html`]) so the logic is unit-testable without a
//! live server.

use crate::plan::Planner;
use crate::Runtime;
use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;

/// Shared runtime state behind the router.
pub struct AgentState {
    runtime: Mutex<Runtime>,
}

impl AgentState {
    /// Wrap a runtime for sharing across handlers.
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime: Mutex::new(runtime) }
    }
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new(crate::default_runtime())
    }
}

/// The serializable tool catalog: `{ "tools": [ {name, description, schema} ] }`.
pub fn tool_catalog_json(rt: &Runtime) -> Value {
    json!({ "tools": rt.tools.catalog() })
}

/// Invoke a tool from a `{ "tool": name, "args": {...} }` request. Always
/// returns an object with an `ok` flag rather than surfacing a transport error.
pub fn invoke_json(rt: &Runtime, req: &Value) -> Value {
    let tool = req.get("tool").and_then(|v| v.as_str()).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or_else(|| json!({}));
    match rt.tools.invoke(tool, &args) {
        Ok(result) => json!({ "ok": true, "tool": tool, "result": result }),
        Err(e) => json!({ "ok": false, "tool": tool, "error": e.to_string() }),
    }
}

/// Decompose a goal into a JSON plan.
pub fn plan_json(goal: &str) -> Value {
    let plan = Planner::decompose(goal);
    serde_json::to_value(&plan).unwrap_or_else(|_| json!({ "goal": goal, "steps": [] }))
}

/// Render the self-contained `/admin/agent` operator page.
pub fn admin_page_html(rt: &Runtime) -> String {
    let rows: String = rt
        .tools
        .catalog()
        .iter()
        .map(|t| {
            format!(
                "<tr><td><code>{}</code></td><td>{}</td></tr>",
                html_escape(&t.name),
                html_escape(&t.description)
            )
        })
        .collect();
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>cave-agent · OpenJarvis primitives</title>
<style>
 body {{ font-family: system-ui, sans-serif; margin: 2rem; color: #1a1a2e; }}
 h1 {{ font-size: 1.4rem; }} h2 {{ font-size: 1.1rem; margin-top: 1.5rem; }}
 table {{ border-collapse: collapse; width: 100%; }}
 td, th {{ border-bottom: 1px solid #ddd; padding: .4rem .6rem; text-align: left; }}
 code {{ background: #f0f0f5; padding: .1rem .3rem; border-radius: 3px; }}
 .pill {{ display: inline-block; background: #2d6cdf; color: #fff; border-radius: 10px;
         padding: .1rem .6rem; font-size: .75rem; margin-right: .3rem; }}
</style>
</head>
<body data-route="/admin/agent">
<h1>cave-agent — OpenJarvis primitives</h1>
<p>On-device composable agent runtime · upstream OpenJarvis <code>{version}</code>
   · self-improvement mandate <code>ADR-SELF-IMPROVE-001</code>.</p>
<p>
 <span class="pill">tool registry</span><span class="pill">conversation memory</span>
 <span class="pill">plan &amp; execute</span><span class="pill">evaluation</span>
 <span class="pill">composable patterns</span><span class="pill">self-improve loop</span>
</p>
<h2>Registered tools ({n})</h2>
<table><thead><tr><th>name</th><th>description</th></tr></thead><tbody>{rows}</tbody></table>
<h2>Runtime knobs</h2>
<ul>
 <li>max_tokens: <code>{max_tokens}</code></li>
 <li>temperature: <code>{temperature}</code></li>
 <li>concurrency: <code>{concurrency}</code></li>
</ul>
<p><small>API: <code>GET /api/agent/tools</code> · <code>POST /api/agent/invoke</code> ·
   <code>POST /api/agent/plan</code></small></p>
</body>
</html>"#,
        version = html_escape(crate::UPSTREAM_VERSION),
        n = rt.tools.len(),
        rows = rows,
        max_tokens = rt.knobs.max_tokens,
        temperature = rt.knobs.temperature,
        concurrency = rt.knobs.concurrency,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ── axum handlers ───────────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "upstream": crate::UPSTREAM_VERSION }))
}

async fn tools(State(s): State<Arc<AgentState>>) -> impl IntoResponse {
    let rt = s.runtime.lock();
    Json(tool_catalog_json(&rt))
}

async fn invoke(State(s): State<Arc<AgentState>>, Json(req): Json<Value>) -> impl IntoResponse {
    let rt = s.runtime.lock();
    Json(invoke_json(&rt, &req))
}

async fn plan(Json(req): Json<Value>) -> impl IntoResponse {
    let goal = req.get("goal").and_then(|v| v.as_str()).unwrap_or("");
    Json(plan_json(goal))
}

async fn admin(State(s): State<Arc<AgentState>>) -> impl IntoResponse {
    let rt = s.runtime.lock();
    Html(admin_page_html(&rt))
}

/// Build the cave-agent router. Mount with `app.merge(cave_agent::router(state))`.
pub fn router(state: Arc<AgentState>) -> Router {
    Router::new()
        .route("/api/agent/health", get(health))
        .route("/api/agent/tools", get(tools))
        .route("/api/agent/invoke", post(invoke))
        .route("/api/agent/plan", post(plan))
        .route("/admin/agent", get(admin))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_neutralizes_markup() {
        assert_eq!(html_escape("<b>&</b>"), "&lt;b&gt;&amp;&lt;/b&gt;");
    }
}
