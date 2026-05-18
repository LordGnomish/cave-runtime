// SPDX-License-Identifier: AGPL-3.0-or-later
//! Datasources tab — registered datasources with health probe state.
//!
//! Mirrors Grafana's `Configuration → Data sources` view. The set is
//! static today (Prometheus / Loki / Tempo / CloudWatch); a live port
//! would call `cave-metrics` / `cave-logs` / `cave-trace` health
//! endpoints and surface real status.

use super::GrafanaViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasourceRow {
    pub name: &'static str,
    pub kind: &'static str,
    pub url: &'static str,
    pub status: &'static str, // "Healthy" | "Degraded" | "Unhealthy"
}

pub fn list_datasources(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<DatasourceRow>, GrafanaViewError> {
    ctx.authorise(Permission::GrafanaRead)?;
    Ok(vec![
        DatasourceRow {
            name: "prom-prod",
            kind: "Prometheus",
            url: "http://prom.observability.svc:9090",
            status: "Healthy",
        },
        DatasourceRow {
            name: "loki-prod",
            kind: "Loki",
            url: "http://loki.observability.svc:3100",
            status: "Healthy",
        },
        DatasourceRow {
            name: "tempo-prod",
            kind: "Tempo",
            url: "http://tempo.observability.svc:3100",
            status: "Healthy",
        },
        DatasourceRow {
            name: "cw-aws-eu-west-1",
            kind: "CloudWatch",
            url: "https://monitoring.eu-west-1.amazonaws.com",
            status: "Degraded",
        },
    ])
}

pub fn healthy_count(rows: &[DatasourceRow]) -> usize {
    rows.iter().filter(|r| r.status == "Healthy").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, GrafanaViewError> {
    let rows = list_datasources(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|d| {
            vec![
                d.name.into(),
                d.kind.into(),
                d.url.into(),
                d.status.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="grafana-datasources" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Datasources ({n}, {h} Healthy)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        h = healthy_count(&rows),
        tbl = table(&["name", "kind", "url", "status"], &table_rows),
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
    fn list_datasources_includes_prometheus_loki_tempo() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/DatasourceList.tsx",
            "Datasources",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_datasources(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let kinds: Vec<_> = rows.iter().map(|r| r.kind).collect();
        for k in ["Prometheus", "Loki", "Tempo", "CloudWatch"] {
            assert!(kinds.contains(&k), "missing kind {}", k);
        }
    }

    #[test]
    fn list_datasources_requires_grafana_read() {
        let s = AdminState::seeded();
        assert!(list_datasources(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn healthy_count_matches_status_field() {
        let s = AdminState::seeded();
        let rows = list_datasources(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let manual = rows.iter().filter(|r| r.status == "Healthy").count();
        assert_eq!(healthy_count(&rows), manual);
    }

    #[test]
    fn render_section_emits_status_pill() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        assert!(html.contains("Healthy"));
        assert!(html.contains("Datasources"));
    }
}
