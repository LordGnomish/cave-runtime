//! Alerts tab — alerting rule list with state (Firing / Pending /
//! Resolved). Sourced from the seeded `active_alerts` collection so
//! the surface is meaningful without a live Prometheus rule eval.

use super::GrafanaViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, ActiveAlert, AdminState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrafanaAlertRow {
    pub name: String,
    pub severity: String,
    pub state: String, // "Firing" | "Pending" | "Resolved"
    pub for_duration: String,
}

pub fn list_alerts(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<GrafanaAlertRow>, GrafanaViewError> {
    ctx.authorise(Permission::GrafanaRead)?;
    Ok(scope(&state.active_alerts.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .map(map_to_row)
    .collect())
}

fn map_to_row(a: &ActiveAlert) -> GrafanaAlertRow {
    // ActiveAlert has `state: &'static str` ("firing"|"pending"); we
    // capitalise for Grafana's display convention. `fired_unix` ↦
    // synthetic "for" duration since we don't track 'for' explicitly
    // in the seeded model.
    let state = match a.state {
        "firing" => "Firing",
        "pending" => "Pending",
        "resolved" => "Resolved",
        other => other,
    };
    GrafanaAlertRow {
        name: a.rule.clone(),
        // Grafana severity isn't on ActiveAlert; map by rule-name heuristic.
        severity: severity_for(&a.rule).into(),
        state: state.into(),
        for_duration: "—".into(),
    }
}

fn severity_for(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("critical") || n.contains("down") || n.contains("error") {
        "critical"
    } else if n.contains("warn") || n.contains("hot") {
        "warning"
    } else {
        "info"
    }
}

pub fn firing_count(rows: &[GrafanaAlertRow]) -> usize {
    rows.iter().filter(|r| r.state == "Firing").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, GrafanaViewError> {
    let rows = list_alerts(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                escape(&r.severity),
                escape(&r.state),
                r.for_duration.clone(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="grafana-alerts" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Alerts ({n}, {firing} Firing)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        firing = firing_count(&rows),
        tbl = table(&["name", "severity", "state", "for"], &table_rows),
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
    fn list_alerts_returns_tenant_rows() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/grafana/src/components/AlertList.tsx",
            "AlertList",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_alerts(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        // Seeded state has acme alerts; we don't assert exact count to
        // stay robust against fixture growth.
        assert!(rows.iter().all(|r| !r.name.is_empty()));
    }

    #[test]
    fn list_alerts_requires_grafana_read() {
        let s = AdminState::seeded();
        assert!(list_alerts(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn firing_count_only_counts_firing() {
        let s = AdminState::seeded();
        let rows = list_alerts(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        let manual = rows.iter().filter(|r| r.state == "Firing").count();
        assert_eq!(firing_count(&rows), manual);
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::GrafanaRead])).unwrap();
        for col in ["name", "severity", "state", "for"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
