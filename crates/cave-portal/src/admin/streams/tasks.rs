// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/tasks` — per-connector task table tab. One row
//! per `(connector, task)` pair with state, source/sink offset
//! position (synthesised from connector kind), last error trace.
//!
//! Persona gate: PlatformAdmin only — Connect task plane is
//! platform-wide.

use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, StreamsConnectTask};
use super::StreamsViewError;

/// Source vs sink offsets are surfaced as two distinct columns —
/// upstream Connect carries the same distinction (`source.offsets`
/// vs `sink.offsets` in the REST API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub connector: String,
    pub task: u32,
    pub state: &'static str,
    pub source_offset: Option<u64>,
    pub sink_offset: Option<u64>,
    pub failure_trace: Option<String>,
}

impl TaskRow {
    fn from_state(t: &StreamsConnectTask, kind: Option<&str>) -> Self {
        // Deterministic synth: source connectors carry `source_offset`,
        // sinks carry `sink_offset`. `Failed` tasks freeze at 0.
        let frozen = t.state == "Failed";
        let pos: u64 = if frozen {
            0
        } else {
            (t.connector.bytes().map(u64::from).sum::<u64>() + t.task as u64) * 11
        };
        let (src, snk) = match kind {
            Some("Source") => (Some(pos), None),
            Some("Sink") => (None, Some(pos)),
            // Unknown kind → expose neither offset (defensive).
            _ => (None, None),
        };
        Self {
            connector: t.connector.clone(),
            task: t.task,
            state: t.state,
            source_offset: src,
            sink_offset: snk,
            failure_trace: t.failure_trace.clone(),
        }
    }
}

pub fn list_tasks(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<TaskRow>, StreamsViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    ctx.authorise(Permission::StreamsRead)?;
    let connectors = state.streams_connectors.read().unwrap();
    let kind_for = |name: &str| {
        connectors
            .iter()
            .find(|c| c.tenant == ctx.tenant && c.name == name)
            .map(|c| c.kind)
    };
    let mut rows: Vec<TaskRow> = scope(
        &state.streams_connect_tasks.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .map(|t| TaskRow::from_state(t, kind_for(&t.connector)))
    .collect();
    rows.sort_by(|a, b| a.connector.cmp(&b.connector).then(a.task.cmp(&b.task)));
    Ok(rows)
}

pub fn failed_count(rows: &[TaskRow]) -> usize {
    rows.iter().filter(|t| t.state == "Failed").count()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_tasks(state, ctx)?;
    let failed = failed_count(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|t| {
            let trace = match &t.failure_trace {
                Some(s) => escape(s),
                None => "—".into(),
            };
            vec![
                escape(&t.connector),
                t.task.to_string(),
                t.state.to_string(),
                t.source_offset.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                t.sink_offset.map(|v| v.to_string()).unwrap_or_else(|| "—".into()),
                trace,
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Tasks ({n}) · failed {f}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-connector task plane. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#connect_rest">Connect REST</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        f = failed,
        tbl = table(
            &["connector", "task", "state", "source_offset", "sink_offset", "failure_trace"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/tasks",
        &format!("streams/tasks · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }
    fn ctx_as(persona: Persona, perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as("acme", perms, persona)
    }

    #[test]
    fn list_tasks_returns_seeded_for_tenant() {
        let rows = list_tasks(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        // jdbc-source has two tasks (0,1) + hdfs-sink has one (0) → 3.
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn list_tasks_sorted_by_connector_then_task() {
        let rows = list_tasks(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].connector < w[1].connector || (w[0].connector == w[1].connector && w[0].task <= w[1].task));
        }
    }

    #[test]
    fn failed_tasks_carry_trace_and_freeze_offset() {
        let rows = list_tasks(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let failed: Vec<_> = rows.iter().filter(|t| t.state == "Failed").collect();
        assert!(!failed.is_empty());
        for t in &failed {
            assert!(t.failure_trace.is_some());
            assert!(t.source_offset.map(|v| v == 0).unwrap_or(true));
            assert!(t.sink_offset.map(|v| v == 0).unwrap_or(true));
        }
    }

    #[test]
    fn list_tasks_rejects_tenant_admin_persona() {
        assert!(list_tasks(
            &AdminState::seeded(),
            &ctx_as(Persona::TenantAdmin, &[Permission::StreamsRead]),
        )
        .is_err());
    }

    #[test]
    fn list_tasks_accepts_platform_admin() {
        assert!(list_tasks(
            &AdminState::seeded(),
            &ctx_as(Persona::PlatformAdmin, &[Permission::StreamsRead]),
        )
        .is_ok());
    }

    #[test]
    fn list_tasks_rejects_without_permission() {
        assert!(list_tasks(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_excludes_evil_task() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Tasks ("));
        assert!(!html.contains("evil-connector"));
    }
}
