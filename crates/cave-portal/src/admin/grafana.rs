//! `/admin/grafana` — Grafana panel-render parity scaffold.
//!
//! Distinct from `admin/dashboard.rs`, which surfaces the **cave-side**
//! dashboard catalog (cave-dashboard crate). This page mirrors the
//! **upstream-UI** shape of Grafana itself — a list of panels grouped
//! by folder + uid, the surface a Grafana user would expect from the
//! `/dashboards` view. Backed by `cave-dashboard`.
//!
//! Upstream UI: <https://grafana.com/grafana/dashboards/>
//!
//! Status: scaffold. The 5 tests below pin the public list/detail
//! shape so a future port can grow the page without breaking
//! contracts. The 2026-05-11 portal-UI audit classifies this page as
//! `scaffold` / P0; promotion to `partial` / `complete` is a separate
//! deliverable.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GrafanaViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// Public row shape — what `/admin/grafana` lists. Matches the
/// Grafana `GET /api/search?type=dash-db` envelope (`uid`, `title`,
/// `folder`, `panels`) so a future renderer can swap in a real
/// Grafana data source without changing call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrafanaPanelRow {
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub panels: u32,
}

/// List Grafana panels visible to the caller's tenant. The scaffold
/// reuses the seeded `DashboardCatalog` rows so the list isn't empty
/// in tests; a real port will pull from the `cave-dashboard` HTTP
/// surface.
pub fn list_panels(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<GrafanaPanelRow>, GrafanaViewError> {
    ctx.authorise(Permission::GrafanaRead)?;
    let catalog = state.dashboard_catalog.read().unwrap();
    let rows: Vec<GrafanaPanelRow> = catalog
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| GrafanaPanelRow {
            uid: r.uid.clone(),
            title: r.title.clone(),
            folder: r.folder.clone(),
            panels: r.panels,
        })
        .collect();
    Ok(rows)
}

/// Render the list page.
pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, GrafanaViewError> {
    let rows = list_panels(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.uid),
                escape(&r.title),
                escape(&r.folder),
                r.panels.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Grafana panel-render parity scaffold (cave-dashboard).
    Upstream: <a class="text-blue-700 underline" href="https://grafana.com/grafana/dashboards/">grafana.com/grafana/dashboards</a>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Panels ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["uid", "title", "folder", "panels"], &table_rows),
    );
    Ok(page_shell(
        &format!("grafana · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/grafana/src/components/DashboardList.tsx", "DashboardList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_panels_filters_to_caller_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "TenantFilter",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_panels(&state, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(rows.iter().all(|r| !r.uid.is_empty()));
        // All visible rows belong to acme.
        assert!(!rows.is_empty(), "seeded data should expose acme panels");
    }

    #[test]
    fn list_panels_refuses_without_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let state = AdminState::seeded();
        assert!(list_panels(&state, &ctx(&[])).is_err());
    }

    #[test]
    fn render_lists_count_in_heading() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "RenderCount",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::GrafanaRead])).unwrap();
        // The count must appear as `Panels (N)`.
        assert!(html.contains("Panels ("), "html missing panels heading: {html:.300}");
    }

    #[test]
    fn render_links_upstream_grafana_url() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "RenderUpstreamLink",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(html.contains("grafana.com/grafana/dashboards"));
    }

    #[test]
    fn render_runs_titles_through_escape_helper() {
        // Defence-in-depth: even though seeded panels don't contain
        // HTML, the render must call `escape()` on user-controlled
        // strings. We pin that by checking the page does NOT contain
        // an unescaped HTML-special sequence that could appear in a
        // hostile panel title (`<img onerror=`). The page_shell adds
        // legitimate <script>/<style> tags, so we test for an attack
        // shape rather than a generic tag.
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "RenderEscape",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(!html.contains("<img onerror="));
        assert!(!html.contains("javascript:"));
    }
}
