// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/workflows` — n8n editor parity (Argo Workflows / Temporal
//! sibling). Workflow run browser with status cards + duration aggregate.
//!
//! Upstream UI: <https://docs.n8n.io/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, WorkflowRun};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkflowsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_runs(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<WorkflowRun>, WorkflowsViewError> {
    ctx.authorise(Permission::WorkflowsRead)?;
    let mut runs: Vec<WorkflowRun> = scope(&state.workflow_runs.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    runs.sort_by(|a, b| b.started_unix.cmp(&a.started_unix));
    Ok(runs)
}

pub fn runs_for(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<Vec<WorkflowRun>, WorkflowsViewError> {
    Ok(list_runs(state, ctx)?.into_iter().filter(|r| r.name == name).collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkflowSummary {
    pub total: u32,
    pub running: u32,
    pub succeeded: u32,
    pub failed: u32,
}

pub fn workflow_summary(rows: &[WorkflowRun]) -> WorkflowSummary {
    let mut s = WorkflowSummary { total: rows.len() as u32, ..Default::default() };
    for r in rows {
        match r.status {
            "Running" | "Pending" => s.running += 1,
            "Succeeded" => s.succeeded += 1,
            "Failed" => s.failed += 1,
            _ => {}
        }
    }
    s
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, WorkflowsViewError> {
    let runs = list_runs(state, ctx)?;
    let summary = workflow_summary(&runs);
    let rows: Vec<Vec<String>> = runs.iter().map(|r| vec![
        r.name.clone(), r.run_id.clone(), r.status.into(),
        r.started_unix.to_string(),
        r.finished_unix.map(|f| f.to_string()).unwrap_or_else(|| "—".into()),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">n8n editor / Argo Workflows (cave-workflows). Upstream: <a class="text-blue-700 underline" href="https://docs.n8n.io/">docs.n8n.io</a>.</p>
  <div class="mb-4 grid grid-cols-4 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL</div><div class="text-2xl font-bold">{total}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNNING</div><div class="text-2xl font-bold text-blue-700">{running}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">SUCCEEDED</div><div class="text-2xl font-bold text-green-700">{succeeded}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">FAILED</div><div class="text-2xl font-bold text-red-700">{failed}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Workflow runs ({n})</h2>{tbl}
</section>"#,
        n = runs.len(),
        total = summary.total,
        running = summary.running,
        succeeded = summary.succeeded,
        failed = summary.failed,
        tbl = table(&["name", "run_id", "status", "started", "finished"], &rows),
    );
    Ok(page_shell_full(ctx, "/admin/workflows", &format!("workflows · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/workflows/src/components/RunsList.tsx", "RunsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_and_orders_newest_first() {
        let (_c, _t) = portal_test_ctx!("plugins/workflows/src/components/RunsList.tsx", "RunsList", "acme");
        let s = AdminState::seeded();
        let r = list_runs(&s, &ctx(&[Permission::WorkflowsRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r[0].started_unix >= r[1].started_unix);
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_runs(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn runs_for_filters_by_name() {
        let (_c, _t) = portal_test_ctx!("plugins/workflows/src/components/WorkflowDetail.tsx", "WorkflowDetail", "acme");
        let s = AdminState::seeded();
        let r = runs_for(&s, &ctx(&[Permission::WorkflowsRead]), "etl-orders").unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn runs_for_does_not_leak_evil_workflow() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-backend/src/PermissionsService.ts", "tenantScopeGuard", "acme");
        let s = AdminState::seeded();
        let r = runs_for(&s, &ctx(&[Permission::WorkflowsRead]), "evil-wf").unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn workflow_summary_counts_states() {
        let r = list_runs(&AdminState::seeded(), &ctx(&[Permission::WorkflowsRead])).unwrap();
        let s = workflow_summary(&r);
        assert_eq!(s.total, r.len() as u32);
        let total_counted = s.running + s.succeeded + s.failed;
        assert!(total_counted <= s.total);
    }

    #[test]
    fn render_includes_summary_cards_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::WorkflowsRead])).unwrap();
        assert!(html.contains("SUCCEEDED"));
        assert!(html.contains("docs.n8n.io"));
    }

    #[test]
    fn render_excludes_evil_run() {
        let (_c, _t) = portal_test_ctx!("plugins/workflows/src/components/RunsPage.tsx", "RunsPage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::WorkflowsRead])).unwrap();
        assert!(html.contains("Workflow runs (2)"));
        assert!(html.contains("etl-orders"));
        assert!(!html.contains("evil-wf"));
    }
}
