// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/connect` view — Kafka Connect runtime
//! console. Mirrors `connect/runtime/rest/resources/` from
//! upstream `apache/kafka`. Four tab anchors:
//!
//! * `#workers`    — Worker roster (worker id, host, state, owns)
//! * `#connectors` — Connector list with lifecycle buttons
//!   (pause / resume / restart / delete). Buttons render only
//!   when the request ctx carries [`Permission::StreamsAdmin`].
//! * `#tasks`      — Per-task state + failure trace + restart
//!   button (admin-gated).
//! * `#configs`    — Connector configuration dump (incl. the
//!   canonical `connector.class` key).
//!
//! Mutators are tenant-scoped — every lookup runs through
//! [`super::super::state::scope`] so a tenant_admin in tenant
//! `acme` cannot touch a connector / task in tenant `evil`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table, table_html};
use crate::admin::state::{
    AdminState, StreamsConnectTask, StreamsConnectWorker, StreamsConnector, scope,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConnectViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("connector {0} not found in this tenant")]
    ConnectorNotFound(String),
    #[error("task {connector}:{task} not found in this tenant")]
    TaskNotFound { connector: String, task: u32 },
}

// ── readers ──────────────────────────────────────────────────────────────────

pub fn list_connectors(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsConnector>, ConnectViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    Ok(scope(
        &state.streams_connectors.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect())
}

pub fn list_workers(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsConnectWorker>, ConnectViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    Ok(scope(
        &state.streams_connect_workers.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect())
}

pub fn list_tasks(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsConnectTask>, ConnectViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    Ok(scope(
        &state.streams_connect_tasks.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect())
}

pub fn inspect_connector(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<StreamsConnector, ConnectViewError> {
    list_connectors(state, ctx)?
        .into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| ConnectViewError::ConnectorNotFound(name.into()))
}

// ── mutators ─────────────────────────────────────────────────────────────────

fn update_connector_state(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    next: &'static str,
) -> Result<(), ConnectViewError> {
    ctx.authorise(Permission::StreamsAdmin)?;
    let mut g = state.streams_connectors.write().unwrap();
    let target = g
        .iter_mut()
        .find(|c| c.tenant == ctx.tenant && c.name == name)
        .ok_or_else(|| ConnectViewError::ConnectorNotFound(name.into()))?;
    target.state = next;
    // Drag tasks with the connector — Paused/Stopped propagate
    // to all owned tasks, Restart resets them to Running.
    let mut tasks = state.streams_connect_tasks.write().unwrap();
    for t in tasks
        .iter_mut()
        .filter(|t| t.tenant == ctx.tenant && t.connector == name)
    {
        match next {
            "Paused" => t.state = "Paused",
            "Stopped" => t.state = "Stopped",
            "Running" => {
                t.state = "Running";
                t.failure_trace = None;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn pause_connector(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), ConnectViewError> {
    update_connector_state(state, ctx, name, "Paused")
}

pub fn resume_connector(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), ConnectViewError> {
    update_connector_state(state, ctx, name, "Running")
}

pub fn restart_connector(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), ConnectViewError> {
    update_connector_state(state, ctx, name, "Running")
}

pub fn delete_connector(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), ConnectViewError> {
    ctx.authorise(Permission::StreamsAdmin)?;
    let mut g = state.streams_connectors.write().unwrap();
    let pos = g
        .iter()
        .position(|c| c.tenant == ctx.tenant && c.name == name)
        .ok_or_else(|| ConnectViewError::ConnectorNotFound(name.into()))?;
    g.remove(pos);
    let mut tasks = state.streams_connect_tasks.write().unwrap();
    tasks.retain(|t| !(t.tenant == ctx.tenant && t.connector == name));
    Ok(())
}

pub fn restart_task(
    state: &AdminState,
    ctx: &RequestCtx,
    connector: &str,
    task: u32,
) -> Result<(), ConnectViewError> {
    ctx.authorise(Permission::StreamsAdmin)?;
    let mut g = state.streams_connect_tasks.write().unwrap();
    let target = g
        .iter_mut()
        .find(|t| t.tenant == ctx.tenant && t.connector == connector && t.task == task)
        .ok_or_else(|| ConnectViewError::TaskNotFound {
            connector: connector.into(),
            task,
        })?;
    target.state = "Running";
    target.failure_trace = None;
    Ok(())
}

// ── renderers ────────────────────────────────────────────────────────────────

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ConnectViewError> {
    let workers = list_workers(state, ctx)?;
    let connectors = list_connectors(state, ctx)?;
    let tasks = list_tasks(state, ctx)?;
    let body = format!(
        r##"<nav aria-label="Connect sub-views" class="mb-4 flex gap-3">
  <a href="#workers" class="underline">workers ({n_w})</a>
  <a href="#connectors" class="underline">connectors ({n_c})</a>
  <a href="#tasks" class="underline">tasks ({n_t})</a>
  <a href="#configs" class="underline">configs</a>
</nav>
<section id="workers" class="mb-6">{w}</section>
<section id="connectors" class="mb-6">{c}</section>
<section id="tasks" class="mb-6">{t}</section>
<section id="configs">{cfg}</section>
"##,
        n_w = workers.len(),
        n_c = connectors.len(),
        n_t = tasks.len(),
        w = render_workers(state, ctx)?,
        c = render_connectors(state, ctx)?,
        t = render_tasks(state, ctx)?,
        cfg = render_configs(state, ctx)?,
    );
    Ok(body)
}

pub fn render_workers(state: &AdminState, ctx: &RequestCtx) -> Result<String, ConnectViewError> {
    let workers = list_workers(state, ctx)?;
    let rows: Vec<Vec<String>> = workers
        .iter()
        .map(|w| {
            vec![
                escape(&w.id),
                escape(&w.host),
                badge(w.state),
                w.connectors_owned.to_string(),
                w.tasks_owned.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<h2 class="text-lg font-semibold mb-2">Workers</h2>{t}"#,
        t = table_html(&["id", "host", "state", "connectors", "tasks"], &rows,),
    ))
}

pub fn render_connectors(state: &AdminState, ctx: &RequestCtx) -> Result<String, ConnectViewError> {
    let connectors = list_connectors(state, ctx)?;
    let admin = ctx.authorise(Permission::StreamsAdmin).is_ok();
    let rows: Vec<Vec<String>> = connectors
        .iter()
        .map(|c| {
            let actions = if admin {
                connector_actions(&c.name)
            } else {
                String::new()
            };
            vec![
                escape(&c.name),
                c.kind.into(),
                badge(c.state),
                c.tasks_max.to_string(),
                escape(&c.topics),
                actions,
            ]
        })
        .collect();
    Ok(format!(
        r#"<h2 class="text-lg font-semibold mb-2">Connectors</h2>{t}"#,
        t = table_html(
            &["name", "kind", "state", "tasks.max", "topics", "actions"],
            &rows,
        ),
    ))
}

pub fn render_tasks(state: &AdminState, ctx: &RequestCtx) -> Result<String, ConnectViewError> {
    let tasks = list_tasks(state, ctx)?;
    let admin = ctx.authorise(Permission::StreamsAdmin).is_ok();
    let rows: Vec<Vec<String>> = tasks
        .iter()
        .map(|t| {
            let trace = match &t.failure_trace {
                Some(s) => escape(s),
                None => "—".into(),
            };
            let actions = if admin {
                task_actions(&t.connector, t.task)
            } else {
                String::new()
            };
            vec![
                escape(&t.connector),
                t.task.to_string(),
                badge(t.state),
                trace,
                actions,
            ]
        })
        .collect();
    Ok(format!(
        r#"<h2 class="text-lg font-semibold mb-2">Tasks</h2>{t}"#,
        t = table_html(
            &["connector", "task", "state", "failure_trace", "actions"],
            &rows,
        ),
    ))
}

pub fn render_configs(state: &AdminState, ctx: &RequestCtx) -> Result<String, ConnectViewError> {
    let connectors = list_connectors(state, ctx)?;
    let rows: Vec<Vec<String>> = connectors
        .iter()
        .map(|c| {
            vec![
                escape(&c.name),
                "connector.class".to_string(),
                escape(&c.class),
            ]
        })
        .collect();
    Ok(format!(
        r#"<h2 class="text-lg font-semibold mb-2">Configs</h2>{t}"#,
        t = table_html(&["connector", "key", "value"], &rows),
    ))
}

// ── private helpers ──────────────────────────────────────────────────────────

fn badge(state: &str) -> String {
    let cls = match state {
        "Running" => "bg-green-700",
        "Paused" => "bg-yellow-700",
        "Failed" => "bg-red-700",
        "Stopped" => "bg-gray-700",
        "Dead" => "bg-red-900",
        _ => "bg-gray-700",
    };
    format!(
        r#"<span role="status" class="px-2 py-0.5 rounded text-white text-xs {cls}">{label}</span>"#,
        label = escape(state),
    )
}

fn connector_actions(name: &str) -> String {
    let esc = escape(name);
    format!(
        r#"<form method="post" action="/admin/streams/connect/{esc}/pause" class="inline">
  <button type="submit" aria-label="Pause connector {esc}" class="underline">pause</button>
</form>
<form method="post" action="/admin/streams/connect/{esc}/resume" class="inline">
  <button type="submit" aria-label="Resume connector {esc}" class="underline">resume</button>
</form>
<form method="post" action="/admin/streams/connect/{esc}/restart" class="inline">
  <button type="submit" aria-label="Restart connector {esc}" class="underline">restart</button>
</form>
<form method="post" action="/admin/streams/connect/{esc}/delete" class="inline">
  <button type="submit" aria-label="Delete connector {esc}" class="underline text-red-600">delete</button>
</form>"#,
    )
}

fn task_actions(connector: &str, task: u32) -> String {
    let esc = escape(connector);
    format!(
        r#"<form method="post" action="/admin/streams/connect/{esc}/tasks/{task}/restart" class="inline">
  <button type="submit" aria-label="Restart task {esc}/{task}" class="underline">restart</button>
</form>"#,
    )
}
