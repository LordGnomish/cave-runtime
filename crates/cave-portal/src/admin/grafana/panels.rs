//! Panels tab — per-panel preview rows. Each Grafana dashboard has N
//! panels; we synthesise a typed panel grid from the dashboard
//! catalog so the surface is meaningful without a real Grafana data
//! source. Panel `type` cycles through the upstream registered types.

use super::{dashboards, GrafanaViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    pub dashboard_uid: String,
    pub panel_id: u32,
    pub title: String,
    pub kind: &'static str, // grafana-style panel type
    pub datasource: &'static str,
}

/// Grafana's primary panel types (subset of upstream registry).
const PANEL_KINDS: &[&str] = &[
    "timeseries", "stat", "gauge", "barchart", "table", "logs", "heatmap",
];

pub fn list_panels(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<PanelRow>, GrafanaViewError> {
    let dashboards = dashboards::list_dashboards(state, ctx)?;
    ctx.authorise(Permission::GrafanaRead)?;
    let mut out = Vec::new();
    for d in dashboards {
        for i in 0..d.panels {
            let kind = PANEL_KINDS[(i as usize) % PANEL_KINDS.len()];
            let datasource = match kind {
                "logs" => "Loki",
                "table" | "stat" | "gauge" => "Prometheus",
                _ => "Prometheus",
            };
            out.push(PanelRow {
                dashboard_uid: d.uid.clone(),
                panel_id: i + 1,
                title: format!("{} · #{}", d.title, i + 1),
                kind,
                datasource,
            });
        }
    }
    Ok(out)
}

pub fn by_kind<'a>(panels: &'a [PanelRow], kind: &str) -> Vec<&'a PanelRow> {
    panels.iter().filter(|p| p.kind == kind).collect()
}

pub fn kind_breakdown(panels: &[PanelRow]) -> Vec<(&'static str, u32)> {
    PANEL_KINDS
        .iter()
        .map(|k| (*k, panels.iter().filter(|p| p.kind == *k).count() as u32))
        .filter(|(_, n)| *n > 0)
        .collect()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, GrafanaViewError> {
    let panels = list_panels(state, ctx)?;
    let breakdown = kind_breakdown(&panels);
    let chips: String = breakdown
        .iter()
        .map(|(k, n)| format!(r#"<span class="px-2 py-1 rounded bg-gray-200 text-xs"><strong>{n}</strong> {k}</span>"#))
        .collect::<Vec<_>>()
        .join(" ");
    let rows: Vec<Vec<String>> = panels
        .iter()
        .take(50)
        .map(|p| {
            vec![
                escape(&p.dashboard_uid),
                p.panel_id.to_string(),
                escape(&p.title),
                p.kind.into(),
                p.datasource.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="grafana-panels" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Panels ({n})</h2>
  <div class="mb-3 flex gap-2 flex-wrap">{chips}</div>
  {tbl}
  <p class="text-xs text-gray-500 mt-2">Showing first 50; panel kind derived from <code>cave-dashboard</code> catalog (upstream Grafana resolves via <code>/api/dashboards/uid/&lt;uid&gt;</code>).</p>
</section>"#,
        n = panels.len(),
        chips = chips,
        tbl = table(
            &["dashboard", "id", "title", "type", "datasource"],
            &rows
        ),
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
    fn list_panels_emits_one_row_per_panel_id() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/PanelRow.tsx",
            "PanelList",
            "acme"
        );
        let s = AdminState::seeded();
        let panels = list_panels(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let dashboards = super::dashboards::list_dashboards(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let expected: u32 = dashboards.iter().map(|d| d.panels).sum();
        assert_eq!(panels.len() as u32, expected);
    }

    #[test]
    fn list_panels_assigns_known_kinds() {
        let s = AdminState::seeded();
        let panels = list_panels(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        for p in &panels {
            assert!(PANEL_KINDS.contains(&p.kind));
        }
    }

    #[test]
    fn by_kind_filters_correctly() {
        let s = AdminState::seeded();
        let panels = list_panels(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let stats = by_kind(&panels, "stat");
        assert!(stats.iter().all(|p| p.kind == "stat"));
    }

    #[test]
    fn kind_breakdown_sums_to_total() {
        let s = AdminState::seeded();
        let panels = list_panels(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let breakdown = kind_breakdown(&panels);
        let total: u32 = breakdown.iter().map(|(_, n)| n).sum();
        assert_eq!(total as usize, panels.len());
    }

    #[test]
    fn render_section_emits_columns_and_chips() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        for col in ["dashboard", "id", "title", "type", "datasource"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
        assert!(html.contains("timeseries") || html.contains("table") || html.contains("stat"));
    }
}
