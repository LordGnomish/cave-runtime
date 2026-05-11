//! `/admin/metrics` view — metrics resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, MetricSeries};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MetricsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MetricSeries>, MetricsViewError> {
    ctx.authorise(Permission::MetricsRead)?;
    Ok(scope(&state.metric_series.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MetricsViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.name.clone(), r.scraper.clone(), r.sample_count.to_string(), r.retention_days.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Metrics ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["name", "scraper", "samples", "retention_days"], &table_rows),
    );
    Ok(page_shell(&format!("metrics · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/metrics/src/components/SeriesList.tsx", "SeriesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/metrics/src/components/SeriesList.tsx", "SeriesList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::MetricsRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!("plugins/metrics/src/components/SeriesList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert!(html.contains("http_requests_total"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/metrics/src/components/SeriesList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert!(!html.contains("evil_metric"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/metrics/src/components/SeriesList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::MetricsRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
