// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explore tab — ad-hoc PromQL/LogQL editor surface.
//!
//! Mirrors Grafana's Explore view header (datasource picker + query
//! editor textarea + run button). This is a *display* surface today;
//! the underlying query path lands when cave-metrics / cave-logs gain
//! a real HTTP-frontend `/query` adapter behind this page.

use super::GrafanaViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::escape;
use crate::admin::state::AdminState;

/// Pre-canned starter queries the Explore view exposes as quick
/// links. Mirrors Grafana's "Query history → Starred" feature.
pub const STARTER_PROMQL: &[(&str, &str)] = &[
    ("CPU per node (5m avg)", "avg by (instance) (rate(node_cpu_seconds_total{mode!='idle'}[5m]))"),
    ("Pod restarts (1h)", "increase(kube_pod_container_status_restarts_total[1h])"),
    ("HTTP 5xx rate", "sum(rate(http_requests_total{status=~'5..'}[5m]))"),
    ("Disk free %", "100 * (node_filesystem_avail_bytes / node_filesystem_size_bytes)"),
];

pub const STARTER_LOGQL: &[(&str, &str)] = &[
    ("Error log rate per service", "sum by (app) (rate({namespace='prod'} |= 'error' [5m]))"),
    ("Per-pod stderr tail", "{app='ingest'} != 'INFO'"),
];

pub fn starter_promql() -> &'static [(&'static str, &'static str)] {
    STARTER_PROMQL
}

pub fn starter_logql() -> &'static [(&'static str, &'static str)] {
    STARTER_LOGQL
}

pub(super) fn render_section(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, GrafanaViewError> {
    ctx.authorise(Permission::GrafanaRead)?;
    let promql_chips: String = STARTER_PROMQL
        .iter()
        .map(|(label, q)| {
            format!(
                r#"<li><span class="font-mono text-xs bg-gray-100 px-1 rounded">{}</span> — {}</li>"#,
                escape(q),
                escape(label),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let logql_chips: String = STARTER_LOGQL
        .iter()
        .map(|(label, q)| {
            format!(
                r#"<li><span class="font-mono text-xs bg-gray-100 px-1 rounded">{}</span> — {}</li>"#,
                escape(q),
                escape(label),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    Ok(format!(
        r#"<section id="grafana-explore" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Explore</h2>
  <p class="text-xs text-gray-500 mb-3">Read-only editor surface. Query execution lands when cave-metrics / cave-logs gain an HTTP-frontend <code>/query</code> adapter.</p>
  <h3 class="text-md font-semibold mt-3 mb-1">Starter PromQL</h3>
  <ul class="text-sm space-y-1">{promql}</ul>
  <h3 class="text-md font-semibold mt-3 mb-1">Starter LogQL</h3>
  <ul class="text-sm space-y-1">{logql}</ul>
  <form class="mt-3 flex flex-col gap-2 max-w-3xl">
    <select class="border rounded px-2 py-1 w-48" name="datasource">
      <option>prom-prod</option><option>loki-prod</option><option>tempo-prod</option>
    </select>
    <textarea class="border rounded px-2 py-2 font-mono text-sm h-24" placeholder="PromQL or LogQL…"></textarea>
    <button type="button" class="px-3 py-1 rounded bg-blue-600 text-white w-fit" disabled>Run (offline)</button>
  </form>
</section>"#,
        promql = promql_chips,
        logql = logql_chips,
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
    fn starter_lists_are_non_empty() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/ExplorePanel.tsx",
            "Starters",
            "acme"
        );
        assert!(!starter_promql().is_empty());
        assert!(!starter_logql().is_empty());
    }

    #[test]
    fn render_section_requires_grafana_read() {
        let s = AdminState::seeded();
        assert!(render_section(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_emits_datasource_picker_and_editor() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(html.contains("prom-prod"));
        assert!(html.contains("loki-prod"));
        assert!(html.contains("tempo-prod"));
        assert!(html.contains("<textarea"));
    }
}
