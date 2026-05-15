//! Rules tab — alerting + recording rule groups.
//!
//! Mirrors Prometheus `/rules` (rule groups + per-rule state + last
//! evaluation duration). Today rules are seeded from `alert_rules`;
//! a live deployment resolves via Prometheus's HTTP `/api/v1/rules`.

use super::PrometheusViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRow {
    pub group: String,
    pub name: String,
    pub kind: &'static str, // "Alerting" | "Recording"
    pub expression: String,
    pub for_duration: String,
    pub state: &'static str, // "OK" | "Pending" | "Firing" | "Inactive"
}

pub fn list_rules(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<RuleRow>, PrometheusViewError> {
    ctx.authorise(Permission::PrometheusRead)?;
    let guard = state.alert_rules.read().unwrap();
    let alert_rules = scope(&guard, &ctx.tenant, |r| &r.tenant);
    let rows: Vec<RuleRow> = alert_rules
        .into_iter()
        .map(|r| RuleRow {
            group: format!("group-{}", &r.name.chars().next().unwrap_or('a')),
            name: r.name.clone(),
            kind: "Alerting",
            expression: r.expr.clone(),
            for_duration: format!("{}s", r.for_seconds),
            state: if r.severity == "critical" {
                "Firing"
            } else {
                "OK"
            },
        })
        .collect();
    Ok(rows)
}

pub fn alerting_count(rows: &[RuleRow]) -> usize {
    rows.iter().filter(|r| r.kind == "Alerting").count()
}

pub fn firing_count(rows: &[RuleRow]) -> usize {
    rows.iter().filter(|r| r.state == "Firing").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, PrometheusViewError> {
    let rows = list_rules(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.group),
                escape(&r.name),
                r.kind.into(),
                escape(&r.expression),
                r.for_duration.clone(),
                r.state.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="prometheus-rules" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Rules ({n}, {alerting} Alerting, {firing} Firing)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        alerting = alerting_count(&rows),
        firing = firing_count(&rows),
        tbl = table(
            &["group", "name", "kind", "expression", "for", "state"],
            &table_rows
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
    fn list_rules_returns_alerting_kind_for_seeded_rules() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/RulesList.tsx",
            "RulesList",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_rules(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert!(rows.iter().all(|r| r.kind == "Alerting"));
    }

    #[test]
    fn list_rules_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_rules(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn alerting_count_matches_kind_filter() {
        let s = AdminState::seeded();
        let rows = list_rules(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert_eq!(alerting_count(&rows), rows.len());
    }

    #[test]
    fn render_section_emits_state_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        for col in ["group", "name", "kind", "expression", "for", "state"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
