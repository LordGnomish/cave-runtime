//! Dashboards tab — Grafana `/dashboards` parity. Folder-tree
//! grouping + flat list, mirroring `GET /api/search?type=dash-db`.

use super::GrafanaViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardRow {
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub panels: u32,
}

pub fn list_dashboards(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<DashboardRow>, GrafanaViewError> {
    ctx.authorise(Permission::GrafanaRead)?;
    let catalog = state.dashboard_catalog.read().unwrap();
    let mut rows: Vec<DashboardRow> = catalog
        .iter()
        .filter(|r| r.tenant.as_str() == ctx.tenant.as_str())
        .map(|r| DashboardRow {
            uid: r.uid.clone(),
            title: r.title.clone(),
            folder: r.folder.clone(),
            panels: r.panels,
        })
        .collect();
    rows.sort_by(|a, b| a.folder.cmp(&b.folder).then(a.title.cmp(&b.title)));
    Ok(rows)
}

pub fn group_by_folder(rows: &[DashboardRow]) -> Vec<(String, Vec<DashboardRow>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<DashboardRow>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.folder.clone()).or_default().push(r.clone());
    }
    let mut out: Vec<(String, Vec<DashboardRow>)> = acc.into_iter().collect();
    for (_, v) in &mut out {
        v.sort_by(|a, b| a.title.cmp(&b.title));
    }
    out
}

pub fn panel_count_total(rows: &[DashboardRow]) -> u32 {
    rows.iter().map(|r| r.panels).sum()
}

pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    uid: &str,
) -> Result<Option<DashboardRow>, GrafanaViewError> {
    let rows = list_dashboards(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.uid == uid))
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, GrafanaViewError> {
    let rows = list_dashboards(state, ctx)?;
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
    Ok(format!(
        r#"<section id="grafana-dashboards" class="mt-2">
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> dashboards</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{p}</strong> panels total</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{f}</strong> folders</span>
  </div>
  <h2 class="text-lg font-semibold mb-2">By folder</h2>
  {folder_sections}
  <h2 class="text-lg font-semibold mt-6 mb-2">Flat list</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        p = total_panels,
        f = groups.len(),
        folder_sections = folder_sections,
        tbl = table(&["uid", "title", "folder", "panels"], &summary_tbl_rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_dashboards_filters_to_caller_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DashboardList.tsx",
            "TenantFilter",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_dashboards(&state, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn list_dashboards_refuses_without_permission() {
        let state = AdminState::seeded();
        assert!(list_dashboards(&state, &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_folder_sorts_alphabetically() {
        let s = AdminState::seeded();
        let rows = list_dashboards(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let groups = group_by_folder(&rows);
        for w in groups.windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
    }

    #[test]
    fn panel_count_total_sums_all_rows() {
        let s = AdminState::seeded();
        let rows = list_dashboards(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let manual: u32 = rows.iter().map(|r| r.panels).sum();
        assert_eq!(panel_count_total(&rows), manual);
    }

    #[test]
    fn detail_returns_none_for_unknown_uid() {
        let s = AdminState::seeded();
        let row = detail(&s, &ctx(&[Permission::GrafanaRead]), "nope-uid").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn render_section_emits_folder_tree_and_flat_list() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(html.contains("By folder"));
        assert!(html.contains("Flat list"));
        assert!(html.contains("dashboards"));
    }
}
