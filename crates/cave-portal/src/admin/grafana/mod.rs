// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/grafana` — Grafana Web UI parity surface.
//!
//! Tabs mirror Grafana's top-level navigation:
//! * **Dashboards** — folder-tree dashboard catalog (cave-dashboard backed).
//! * **Panels** — per-panel previews with type + datasource.
//! * **Datasources** — Prometheus / Loki / Tempo / CloudWatch with health.
//! * **Explore** — ad-hoc query editor (PromQL / LogQL) — read-only surface.
//! * **Alerts** — alerting rule list with state (Firing / Pending / Resolved).
//!
//! Upstream: <https://grafana.com/grafana/dashboards/>
//!
//! Each submodule owns its accessors + tests; mod.rs composes them
//! into one page and re-exports the legacy `list_panels` /
//! `GrafanaPanelRow` / `GrafanaViewError` / `group_by_folder` /
//! `panel_count_total` / `detail` from `dashboards.rs` so the existing
//! callers keep compiling.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use crate::admin::types::Cite;

pub mod alerts;
pub mod dashboards;
pub mod datasources;
pub mod explore;
pub mod panels;

// Legacy compatibility — old call sites used these names.
pub use dashboards::{
    detail, group_by_folder, list_dashboards as list_panels, panel_count_total, DashboardRow as GrafanaPanelRow,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GrafanaViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, GrafanaViewError> {
    ctx.authorise(Permission::GrafanaRead)?;
    let dashboards_html = dashboards::render_section(state, ctx)?;
    let panels_html = panels::render_section(state, ctx)?;
    let datasources_html = datasources::render_section(state, ctx)?;
    let explore_html = explore::render_section(state, ctx)?;
    let alerts_html = alerts::render_section(state, ctx)?;
    let body = format!(
        r##"<section class="mb-4 p-3 bg-blue-50 rounded text-sm text-blue-900">
  Grafana Web UI parity (cave-dashboard).
  Upstream: <a class="text-blue-700 underline" href="https://grafana.com/grafana/dashboards/">grafana.com/grafana/dashboards</a>.
</section>
<nav class="mb-4 flex gap-4 text-sm text-blue-700">
  <a href="#grafana-dashboards">Dashboards</a>
  <a href="#grafana-panels">Panels</a>
  <a href="#grafana-datasources">Datasources</a>
  <a href="#grafana-explore">Explore</a>
  <a href="#grafana-alerts">Alerts</a>
</nav>
{dashboards}
{panels}
{datasources}
{explore}
{alerts}"##,
        dashboards = dashboards_html,
        panels = panels_html,
        datasources = datasources_html,
        explore = explore_html,
        alerts = alerts_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/grafana",
        &format!("grafana · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/grafana/src/components/DashboardList.tsx",
    "DashboardList",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_grafana_read() {
        let s = AdminState::seeded();
        assert!(render(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_all_five_tabs() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::GrafanaRead])).unwrap();
        for anchor in [
            "#grafana-dashboards",
            "#grafana-panels",
            "#grafana-datasources",
            "#grafana-explore",
            "#grafana-alerts",
        ] {
            assert!(html.contains(anchor), "missing anchor {anchor}");
        }
        assert!(html.contains("grafana.com/grafana/dashboards"));
    }

    #[test]
    fn render_excludes_evil_tenant_dashboards() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::GrafanaRead])).unwrap();
        // The seed includes an evil-tenant dashboard; it must not leak.
        assert!(!html.contains("evil-dashboard"));
    }
}
