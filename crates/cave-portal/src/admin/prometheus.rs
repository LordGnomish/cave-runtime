//! `/admin/prometheus` — Prometheus targets + alerts upstream-UI parity
//! scaffold.
//!
//! Distinct from `admin/metrics.rs` (cave-metrics catalog view). This
//! page mirrors the **upstream-UI** shape of Prometheus's
//! `/targets` and `/alerts` pages — a list of scrape targets and the
//! rules that fire against them.
//!
//! Upstream UI: <https://prometheus.io/docs/>
//!
//! Status: scaffold. The 5 tests pin list/render contracts so the
//! port can grow without breaking call sites.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrometheusViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrometheusTargetRow {
    pub series: String,
    pub scraper: String,
    pub sample_count: u64,
    pub retention_days: u32,
}

pub fn list_targets(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<PrometheusTargetRow>, PrometheusViewError> {
    ctx.authorise(Permission::PrometheusRead)?;
    let series = state.metric_series.read().unwrap();
    let mut rows: Vec<PrometheusTargetRow> = series
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| PrometheusTargetRow {
            series: r.name.clone(),
            scraper: r.scraper.clone(),
            sample_count: r.sample_count,
            retention_days: r.retention_days,
        })
        .collect();
    rows.sort_by(|a, b| a.scraper.cmp(&b.scraper).then(a.series.cmp(&b.series)));
    Ok(rows)
}

/// Group targets by `scraper` so the UI can render one panel per
/// Prometheus scrape job (matches `/targets` page layout — each
/// scrape job has its own collapsed section).
pub fn group_by_scraper(
    rows: &[PrometheusTargetRow],
) -> Vec<(String, Vec<PrometheusTargetRow>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<PrometheusTargetRow>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.scraper.clone()).or_default().push(r.clone());
    }
    let mut out: Vec<(String, Vec<PrometheusTargetRow>)> = acc.into_iter().collect();
    for (_, v) in &mut out {
        v.sort_by(|a, b| a.series.cmp(&b.series));
    }
    out
}

/// Total samples scraped across all targets. Mirrors the
/// `prometheus_tsdb_head_samples_appended_total` counter in the
/// upstream UI's summary header.
pub fn total_samples(rows: &[PrometheusTargetRow]) -> u64 {
    rows.iter().map(|r| r.sample_count).sum()
}

/// Find one target row by series name + scraper (composite key —
/// Prometheus permits the same metric name across scrapers).
pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    scraper: &str,
    series: &str,
) -> Result<Option<PrometheusTargetRow>, PrometheusViewError> {
    let rows = list_targets(state, ctx)?;
    Ok(rows
        .into_iter()
        .find(|r| r.scraper == scraper && r.series == series))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PrometheusViewError> {
    let rows = list_targets(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.series),
                escape(&r.scraper),
                r.sample_count.to_string(),
                r.retention_days.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Prometheus targets + alerts scaffold (cave-metrics).
    Upstream: <a class="text-blue-700 underline" href="https://prometheus.io/docs/">prometheus.io/docs</a>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Targets ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["series", "scraper", "samples", "retention_days"], &table_rows),
    );
    Ok(page_shell(
        &format!("prometheus · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/prometheus/src/components/TargetsList.tsx", "TargetsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_targets_filters_to_caller_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetsList.tsx",
            "TenantFilter",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_targets(&state, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(!rows.is_empty());
        // No evil_metric in acme view.
        assert!(rows.iter().all(|r| !r.series.contains("evil")));
    }

    #[test]
    fn list_targets_refuses_without_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_targets(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_lists_count_in_heading() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetsList.tsx",
            "RenderCount",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(html.contains("Targets ("));
    }

    #[test]
    fn group_by_scraper_returns_one_entry_per_job() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetsList.tsx",
            "GroupByScraper",
            "acme"
        );
        let rows = list_targets(&AdminState::seeded(), &ctx(&[Permission::PrometheusRead])).unwrap();
        let groups = group_by_scraper(&rows);
        let names: Vec<&str> = groups.iter().map(|(n, _)| n.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, rows.len());
    }

    #[test]
    fn total_samples_sums_all_series() {
        let rows = vec![
            PrometheusTargetRow { series: "a".into(), scraper: "s".into(), sample_count: 100, retention_days: 30 },
            PrometheusTargetRow { series: "b".into(), scraper: "s".into(), sample_count: 250, retention_days: 30 },
        ];
        assert_eq!(total_samples(&rows), 350);
        assert_eq!(total_samples(&[]), 0);
    }

    #[test]
    fn detail_returns_target_by_composite_key() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetDetail.tsx",
            "Detail",
            "acme"
        );
        let s = AdminState::seeded();
        let d = detail(
            &s,
            &ctx(&[Permission::PrometheusRead]),
            "prometheus-prod",
            "http_requests_total",
        )
        .unwrap();
        assert!(d.is_some());
        let none = detail(
            &s,
            &ctx(&[Permission::PrometheusRead]),
            "no-scraper",
            "no-series",
        )
        .unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn list_targets_returns_sorted_by_scraper_then_series() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetsList.tsx",
            "SortedOrder",
            "acme"
        );
        let rows = list_targets(&AdminState::seeded(), &ctx(&[Permission::PrometheusRead])).unwrap();
        for w in rows.windows(2) {
            let a = (&w[0].scraper, &w[0].series);
            let b = (&w[1].scraper, &w[1].series);
            assert!(a <= b);
        }
    }

    #[test]
    fn render_links_prometheus_docs() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetsList.tsx",
            "RenderUpstreamLink",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(html.contains("prometheus.io/docs"));
    }

    #[test]
    fn render_shows_series_name() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TargetsList.tsx",
            "RenderSeries",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(html.contains("http_requests_total"));
        assert!(!html.contains("evil_metric"));
    }
}
