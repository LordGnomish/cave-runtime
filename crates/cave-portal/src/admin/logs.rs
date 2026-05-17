// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/logs` — Grafana Loki Explore parity for the cave-logs
//! catalog view. Distinct from `admin/loki.rs` (upstream-UI shape
//! for the LogQL editor itself) — this page renders the cave-side
//! catalog with sink grouping + total-ingest summary.
//!
//! Upstream UI: <https://grafana.com/docs/loki/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, LogStream};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LogsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LogStream>, LogsViewError> {
    ctx.authorise(Permission::LogsRead)?;
    let mut rows: Vec<LogStream> = scope(&state.log_streams.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.ingest_rate_per_sec.cmp(&a.ingest_rate_per_sec).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn group_by_sink(rows: &[LogStream]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows { *acc.entry(r.sink.clone()).or_insert(0) += 1; }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

pub fn total_ingest_per_sec(rows: &[LogStream]) -> u64 {
    rows.iter().map(|r| u64::from(r.ingest_rate_per_sec)).sum()
}

pub fn by_sink<'a>(rows: &'a [LogStream], sink: &str) -> Vec<&'a LogStream> {
    rows.iter().filter(|r| r.sink == sink).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LogsViewError> {
    let rows = list_records(state, ctx)?;
    let groups = group_by_sink(&rows);
    let total = total_ingest_per_sec(&rows);
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n
    )).collect();
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.name), escape(&r.sink), r.ingest_rate_per_sec.to_string(), r.retention_days.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Loki streams (cave-logs). Upstream: <a class="text-blue-700 underline" href="https://grafana.com/docs/loki/">grafana.com/docs/loki</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> streams</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{total}</strong>/s total ingest</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Streams ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total = total,
        chips = chips,
        tbl = table(&["name", "sink", "ingest/s", "retention_days"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/logs", &format!("logs · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/logs/src/components/StreamsList.tsx", "StreamsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_and_sorts_by_ingest_desc() {
        let (_c, _t) = portal_test_ctx!("plugins/logs/src/components/StreamsList.tsx", "StreamsList", "acme");
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].ingest_rate_per_sec >= w[1].ingest_rate_per_sec); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_sink_counts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        let g = group_by_sink(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn total_ingest_sums_rates() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        let expected: u64 = r.iter().map(|x| u64::from(x.ingest_rate_per_sec)).sum();
        assert_eq!(total_ingest_per_sec(&r), expected);
    }

    #[test]
    fn by_sink_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        let loki = by_sink(&r, "loki");
        assert!(loki.iter().all(|x| x.sink == "loki"));
        assert!(by_sink(&r, "no-such").is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        assert!(html.contains("web-stdout"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        assert!(!html.contains("evil-stream"));
    }

    #[test]
    fn render_includes_total_ingest_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::LogsRead])).unwrap();
        assert!(html.contains("total ingest"));
        assert!(html.contains("grafana.com/docs/loki"));
    }
}
