// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/metrics` — Prometheus expr browser parity for the
//! cave-metrics catalog view. Sister of `admin/prometheus.rs`
//! (upstream-UI parity); this page focuses on the cave-side catalog.
//!
//! Upstream UI: <https://prometheus.io/docs/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, MetricSeries};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MetricsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MetricSeries>, MetricsViewError> {
    ctx.authorise(Permission::MetricsRead)?;
    let mut rows: Vec<MetricSeries> = scope(&state.metric_series.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.sample_count.cmp(&a.sample_count).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn group_by_scraper(rows: &[MetricSeries]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows { *acc.entry(r.scraper.clone()).or_insert(0) += 1; }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

pub fn total_samples(rows: &[MetricSeries]) -> u64 {
    rows.iter().map(|r| r.sample_count).sum()
}

pub fn by_name<'a>(rows: &'a [MetricSeries], needle: &str) -> Vec<&'a MetricSeries> {
    let lc = needle.to_lowercase();
    rows.iter().filter(|r| r.name.to_lowercase().contains(&lc)).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MetricsViewError> {
    let rows = list_records(state, ctx)?;
    let total = total_samples(&rows);
    let groups = group_by_scraper(&rows);
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n)).collect();
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.name), escape(&r.scraper), r.sample_count.to_string(), r.retention_days.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Prometheus series catalog (cave-metrics). Upstream: <a class="text-blue-700 underline" href="https://prometheus.io/docs/">prometheus.io/docs</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> series</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{total}</strong> samples total</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Series ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total = total,
        chips = chips,
        tbl = table(&["name", "scraper", "samples", "retention_days"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/metrics", &format!("metrics · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/metrics/src/components/SeriesList.tsx", "SeriesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_and_sorts_by_samples_desc() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) { assert!(w[0].sample_count >= w[1].sample_count); }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_scraper_counts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        let g = group_by_scraper(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn total_samples_sums() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        let expected: u64 = r.iter().map(|x| x.sample_count).sum();
        assert_eq!(total_samples(&r), expected);
    }

    #[test]
    fn by_name_substring_filter() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        let hits = by_name(&r, "http");
        assert!(hits.iter().all(|x| x.name.contains("http")));
        assert!(by_name(&r, "no-such").is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert!(html.contains("http_requests_total"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert!(!html.contains("evil_metric"));
    }

    #[test]
    fn render_includes_total_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert!(html.contains("samples total"));
        assert!(html.contains("prometheus.io/docs"));
    }
}
