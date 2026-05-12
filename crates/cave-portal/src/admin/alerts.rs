//! `/admin/alerts` — Alertmanager UI parity. Rules + active alerts
//! with severity-grouped pills and the existing ack mutator.
//!
//! Upstream UI: <https://prometheus.io/docs/alerting/latest/clients/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ActiveAlert, AlertRule};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AlertsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("active alert for rule {0} not found")]
    AlertNotFound(String),
}

pub fn list_rules(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<AlertRule>, AlertsViewError> {
    ctx.authorise(Permission::AlertsRead)?;
    Ok(scope(&state.alert_rules.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn list_active(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ActiveAlert>, AlertsViewError> {
    ctx.authorise(Permission::AlertsRead)?;
    Ok(scope(&state.active_alerts.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn ack_alert(state: &AdminState, ctx: &RequestCtx, rule: &str) -> Result<(), AlertsViewError> {
    ctx.authorise(Permission::AlertsAck)?;
    let mut alerts = state.active_alerts.write().unwrap();
    let before = alerts.len();
    alerts.retain(|a| !(a.tenant == ctx.tenant && a.rule == rule));
    if alerts.len() == before {
        return Err(AlertsViewError::AlertNotFound(rule.into()));
    }
    Ok(())
}

/// Group rules by severity ("critical", "warning", "info" — same
/// vocabulary Alertmanager uses).
pub fn group_by_severity(rules: &[AlertRule]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rules { *acc.entry(r.severity.to_string()).or_insert(0) += 1; }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    // Standard severity order: critical > warning > info.
    out.sort_by(|a, b| severity_rank(&b.0).cmp(&severity_rank(&a.0)));
    out
}

fn severity_rank(s: &str) -> u32 {
    match s {
        "critical" => 3,
        "warning" => 2,
        "info" => 1,
        _ => 0,
    }
}

pub fn rules_by_severity<'a>(rules: &'a [AlertRule], severity: &str) -> Vec<&'a AlertRule> {
    rules.iter().filter(|r| r.severity == severity).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AlertsViewError> {
    let rules = list_rules(state, ctx)?;
    let active = list_active(state, ctx)?;
    let groups = group_by_severity(&rules);
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n)).collect();
    let r_rows: Vec<Vec<String>> = rules.iter().map(|r| vec![
        r.name.clone(), r.severity.into(), r.expr.clone(), format!("{}s", r.for_seconds),
    ]).collect();
    let a_rows: Vec<Vec<String>> = active.iter().map(|a| vec![
        a.rule.clone(), a.state.into(), a.fired_unix.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Alertmanager UI parity (cave-alerts). Upstream: <a class="text-blue-700 underline" href="https://prometheus.io/docs/alerting/latest/clients/">prometheus.io/docs/alerting</a>.</p>
  <div class="mb-4">{chips}</div>
  <section><h2 class="text-lg font-semibold mb-2">Rules ({n_r})</h2>{r_tbl}</section>
  <section class="mt-6"><h2 class="text-lg font-semibold mb-2">Active ({n_a})</h2>{a_tbl}</section>
</section>"#,
        chips = chips,
        n_r = rules.len(), n_a = active.len(),
        r_tbl = table(&["name", "severity", "expr", "for"], &r_rows),
        a_tbl = table(&["rule", "state", "fired"], &a_rows),
    );
    Ok(page_shell(&format!("alerts · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/alerts/src/components/AlertsList.tsx", "AlertsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_rules_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/alerts/src/components/AlertsList.tsx", "AlertsList", "acme");
        let s = AdminState::seeded();
        let r = list_rules(&s, &ctx(&[Permission::AlertsRead])).unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn list_active_excludes_evil() {
        let (_c, _t) = portal_test_ctx!("plugins/alerts/src/components/ActiveAlerts.tsx", "ActiveAlerts", "acme");
        let s = AdminState::seeded();
        let a = list_active(&s, &ctx(&[Permission::AlertsRead])).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].rule, "HighErrorRate");
    }

    #[test]
    fn ack_alert_removes_active_and_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!("plugins/alerts/src/components/AckButton.tsx", "AckButton", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::AlertsRead, Permission::AlertsAck]);
        ack_alert(&s, &c, "HighErrorRate").unwrap();
        assert_eq!(list_active(&s, &c).unwrap().len(), 0);
        assert!(matches!(ack_alert(&s, &c, "EvilNoiseAlert").unwrap_err(), AlertsViewError::AlertNotFound(_)));
    }

    #[test]
    fn ack_alert_requires_ack_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-backend/src/PermissionsService.ts", "ackPerm", "acme");
        assert!(ack_alert(&AdminState::seeded(), &ctx(&[Permission::AlertsRead]), "HighErrorRate").is_err());
    }

    #[test]
    fn group_by_severity_orders_critical_first() {
        let r = list_rules(&AdminState::seeded(), &ctx(&[Permission::AlertsRead])).unwrap();
        let g = group_by_severity(&r);
        // Critical (if any) must precede warning (if any).
        let crit_pos = g.iter().position(|(s, _)| s == "critical");
        let warn_pos = g.iter().position(|(s, _)| s == "warning");
        if let (Some(c), Some(w)) = (crit_pos, warn_pos) { assert!(c < w); }
    }

    #[test]
    fn rules_by_severity_filters() {
        let r = list_rules(&AdminState::seeded(), &ctx(&[Permission::AlertsRead])).unwrap();
        if let Some(f) = r.first() {
            let sev = f.severity;
            assert!(rules_by_severity(&r, sev).iter().all(|x| x.severity == sev));
        }
        assert!(rules_by_severity(&r, "no-such").is_empty());
    }

    #[test]
    fn render_includes_severity_chips_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AlertsRead])).unwrap();
        assert!(html.contains("prometheus.io/docs/alerting"));
    }

    #[test]
    fn render_excludes_evil_rule() {
        let (_c, _t) = portal_test_ctx!("plugins/alerts/src/components/AlertsPage.tsx", "AlertsPage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AlertsRead])).unwrap();
        assert!(html.contains("Rules (2)"));
        assert!(html.contains("HighErrorRate"));
        assert!(!html.contains("EvilNoiseAlert"));
    }
}
