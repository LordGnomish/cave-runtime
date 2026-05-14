// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/pipelines` — Tekton Dashboard parity. PipelineRun graph
//! grouped by pipeline + per-status summary cards.
//!
//! Upstream UI: <https://tekton.dev/docs/dashboard/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, PipelineRun};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PipelinesViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<PipelineRun>, PipelinesViewError> {
    ctx.authorise(Permission::PipelinesRead)?;
    let mut rows: Vec<PipelineRun> = scope(&state.pipeline_runs.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| a.pipeline.cmp(&b.pipeline).then(a.run_id.cmp(&b.run_id)));
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PipelineSummary {
    pub total: u32,
    pub running: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub total_duration_s: u64,
}

pub fn pipeline_summary(rows: &[PipelineRun]) -> PipelineSummary {
    let mut s = PipelineSummary { total: rows.len() as u32, ..Default::default() };
    for r in rows {
        match r.status {
            "Running" => s.running += 1,
            "Succeeded" => s.succeeded += 1,
            "Failed" => s.failed += 1,
            _ => {}
        }
        s.total_duration_s += u64::from(r.duration_seconds);
    }
    s
}

pub fn by_status<'a>(rows: &'a [PipelineRun], status: &str) -> Vec<&'a PipelineRun> {
    rows.iter().filter(|r| r.status == status).collect()
}

pub fn by_pipeline<'a>(rows: &'a [PipelineRun], pipeline: &str) -> Vec<&'a PipelineRun> {
    rows.iter().filter(|r| r.pipeline == pipeline).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PipelinesViewError> {
    let rows = list_records(state, ctx)?;
    let summary = pipeline_summary(&rows);
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.pipeline), escape(&r.run_id), r.status.into(), r.duration_seconds.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Tekton Dashboard parity (cave-pipelines). Upstream: <a class="text-blue-700 underline" href="https://tekton.dev/docs/dashboard/">tekton.dev/docs/dashboard</a>.</p>
  <div class="mb-4 grid grid-cols-4 gap-2 text-center text-sm">
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNS</div><div class="text-2xl font-bold">{total}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">RUNNING</div><div class="text-2xl font-bold text-blue-700">{running}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">SUCCEEDED</div><div class="text-2xl font-bold text-green-700">{succeeded}</div></div>
    <div class="p-3 bg-white rounded shadow"><div class="text-xs text-gray-500">FAILED</div><div class="text-2xl font-bold text-red-700">{failed}</div></div>
  </div>
  <h2 class="text-lg font-semibold mb-2">Runs ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total = summary.total,
        running = summary.running,
        succeeded = summary.succeeded,
        failed = summary.failed,
        tbl = table(&["pipeline", "run_id", "status", "duration_s"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/pipelines", &format!("pipelines · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/pipelines/src/components/RunsList.tsx", "RunsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_and_sorts_by_pipeline() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].pipeline <= w[1].pipeline); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn pipeline_summary_counts_states() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        let s = pipeline_summary(&r);
        assert_eq!(s.total, r.len() as u32);
        let total_dur: u64 = r.iter().map(|x| u64::from(x.duration_seconds)).sum();
        assert_eq!(s.total_duration_s, total_dur);
    }

    #[test]
    fn by_status_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        let succ = by_status(&r, "Succeeded");
        assert!(succ.iter().all(|x| x.status == "Succeeded"));
    }

    #[test]
    fn by_pipeline_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        if let Some(f) = r.first() {
            let n = &f.pipeline.clone();
            assert!(by_pipeline(&r, n).iter().all(|x| &x.pipeline == n));
        }
        assert!(by_pipeline(&r, "no-such").is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        assert!(html.contains("build-web"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        assert!(!html.contains("evil-pl"));
    }

    #[test]
    fn render_includes_summary_cards_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PipelinesRead])).unwrap();
        assert!(html.contains("RUNS"));
        assert!(html.contains("tekton.dev"));
    }
}
