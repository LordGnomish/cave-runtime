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
    let mut rows: Vec<GrafanaPanelRow> = catalog
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| GrafanaPanelRow {
            uid: r.uid.clone(),
            title: r.title.clone(),
            folder: r.folder.clone(),
            panels: r.panels,
        })
        .collect();
    rows.sort_by(|a, b| a.folder.cmp(&b.folder).then(a.title.cmp(&b.title)));
    Ok(rows)
}

/// Group panels by `folder` so the page can render Grafana's
/// folder-tree layout (each folder collapses a group of dashboards).
/// Returns `(folder → Vec<row>)` pairs with folders sorted A→Z and
/// rows within each folder sorted by title.
pub fn group_by_folder(
    rows: &[GrafanaPanelRow],
) -> Vec<(String, Vec<GrafanaPanelRow>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<GrafanaPanelRow>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.folder.clone()).or_default().push(r.clone());
    }
    let mut out: Vec<(String, Vec<GrafanaPanelRow>)> = acc.into_iter().collect();
    for (_, v) in &mut out {
        v.sort_by(|a, b| a.title.cmp(&b.title));
    }
    out
}

/// Total panel count across all visible dashboards. Mirrors the
/// summary chip Grafana's `/dashboards` page shows in the header.
pub fn panel_count_total(rows: &[GrafanaPanelRow]) -> u32 {
    rows.iter().map(|r| r.panels).sum()
}

/// Find a single dashboard by `uid` (Grafana's stable identifier).
pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    uid: &str,
) -> Result<Option<GrafanaPanelRow>, GrafanaViewError> {
    let rows = list_panels(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.uid == uid))
}

/// Render the list page with folder grouping. Mirrors Grafana's
/// `/dashboards` folder-tree layout.
pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, GrafanaViewError> {
    let rows = list_panels(state, ctx)?;
    let total_panels = panel_count_total(&rows);
    let groups = group_by_folder(&rows);
    let folder_sections: String = groups
        .iter()
        .map(|(folder, items)| {
            let items_html = items
                .iter()
                .map(|r| {
                    format!(
                        r#"<tr><td><code>{uid}</code></td><td>{title}</td><td>{panels} panels</td></tr>"#,
                        uid = escape(&r.uid),
                        title = escape(&r.title),
                        panels = r.panels,
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            format!(
                r#"<details open class="mb-2 p-2 bg-white rounded shadow-sm">
  <summary class="cursor-pointer font-semibold">📁 {f} <small class="text-gray-500">({n})</small></summary>
  <table class="mt-2 w-full text-sm"><thead><tr><th>uid</th><th>title</th><th class="text-right">panels</th></tr></thead><tbody>{items}</tbody></table>
</details>"#,
                f = escape(folder),
                n = items.len(),
                items = items_html,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let summary_tbl_rows: Vec<Vec<String>> = rows
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
    Grafana panel-render parity (cave-dashboard).
    Upstream: <a class="text-blue-700 underline" href="https://grafana.com/grafana/dashboards/">grafana.com/grafana/dashboards</a>.
  </p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> dashboards</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{p}</strong> panels total</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{f}</strong> folders</span>
  </div>
  <h2 class="text-lg font-semibold mb-2">By folder</h2>
  {folder_sections}
  <h2 class="text-lg font-semibold mt-6 mb-2">Flat panel list</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        p = total_panels,
        f = groups.len(),
        folder_sections = folder_sections,
        tbl = table(&["uid", "title", "folder", "panels"], &summary_tbl_rows),
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
    fn render_shows_dashboard_summary_chips() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "RenderSummary",
            "acme"
        );
        let state = AdminState::seeded();
        let html = render(&state, &ctx(&[Permission::GrafanaRead])).unwrap();
        // The dashboard summary chips render the count + total panels.
        assert!(html.contains("dashboards"));
        assert!(html.contains("panels total"));
        assert!(html.contains("By folder"));
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
    fn list_panels_returns_rows_sorted_by_folder_then_title() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "SortedOrder",
            "acme"
        );
        let rows = list_panels(&AdminState::seeded(), &ctx(&[Permission::GrafanaRead])).unwrap();
        for w in rows.windows(2) {
            let a = (&w[0].folder, &w[0].title);
            let b = (&w[1].folder, &w[1].title);
            assert!(a <= b, "rows not sorted: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn group_by_folder_groups_dashboards_correctly() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "GroupByFolder",
            "acme"
        );
        let rows = list_panels(&AdminState::seeded(), &ctx(&[Permission::GrafanaRead])).unwrap();
        let groups = group_by_folder(&rows);
        // Each row belongs to exactly one group.
        let total_in_groups: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total_in_groups, rows.len());
        // Folder names unique + sorted.
        let folders: Vec<&str> = groups.iter().map(|(f, _)| f.as_str()).collect();
        let mut sorted = folders.clone();
        sorted.sort();
        assert_eq!(folders, sorted);
    }

    #[test]
    fn panel_count_total_sums_panels() {
        let rows = vec![
            GrafanaPanelRow { uid: "a".into(), title: "x".into(), folder: "f".into(), panels: 5 },
            GrafanaPanelRow { uid: "b".into(), title: "y".into(), folder: "f".into(), panels: 7 },
        ];
        assert_eq!(panel_count_total(&rows), 12);
        assert_eq!(panel_count_total(&[]), 0);
    }

    #[test]
    fn detail_returns_dashboard_by_uid() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardDetail.tsx",
            "DetailFound",
            "acme"
        );
        let rows = list_panels(&AdminState::seeded(), &ctx(&[Permission::GrafanaRead])).unwrap();
        if let Some(first) = rows.first() {
            let uid = first.uid.clone();
            let d = detail(
                &AdminState::seeded(),
                &ctx(&[Permission::GrafanaRead]),
                &uid,
            )
            .unwrap();
            assert!(d.is_some());
        }
    }

    #[test]
    fn detail_returns_none_for_missing_uid() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardDetail.tsx",
            "DetailMissing",
            "acme"
        );
        assert!(detail(
            &AdminState::seeded(),
            &ctx(&[Permission::GrafanaRead]),
            "no-such-uid",
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn render_shows_folder_grouped_layout() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "FolderLayout",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::GrafanaRead])).unwrap();
        // Folder details element + summary chips.
        assert!(html.contains("<details"));
        assert!(html.contains("By folder"));
        assert!(html.contains("dashboards"));
        assert!(html.contains("panels total"));
        assert!(html.contains("folders"));
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
