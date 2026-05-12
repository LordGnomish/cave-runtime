//! `/admin/incidents` — Grafana OnCall incidents parity. Severity
//! grouped pills + state-transition mutator (preserved).
//!
//! Upstream UI: <https://grafana.com/docs/oncall/latest/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, IncidentRecord};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IncidentsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("incident {0} not found in this tenant")]
    IncidentNotFound(String),
    #[error("invalid state {0}: must be Open, Investigating, or Resolved")]
    InvalidState(String),
}

pub fn list_incidents(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<IncidentRecord>, IncidentsViewError> {
    ctx.authorise(Permission::IncidentsRead)?;
    let mut rows: Vec<IncidentRecord> = scope(&state.incident_records.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| b.opened_unix.cmp(&a.opened_unix));
    Ok(rows)
}

pub fn transition(state: &AdminState, ctx: &RequestCtx, id: &str, new_state: &str) -> Result<(), IncidentsViewError> {
    ctx.authorise(Permission::IncidentsWrite)?;
    let normalised: &'static str = match new_state {
        "Open" => "Open",
        "Investigating" => "Investigating",
        "Resolved" => "Resolved",
        _ => return Err(IncidentsViewError::InvalidState(new_state.into())),
    };
    let mut incs = state.incident_records.write().unwrap();
    let target = incs.iter_mut().find(|i| i.tenant == ctx.tenant && i.id == id)
        .ok_or_else(|| IncidentsViewError::IncidentNotFound(id.into()))?;
    target.state = normalised;
    Ok(())
}

pub fn group_by_severity(rows: &[IncidentRecord]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows { *acc.entry(r.severity.to_string()).or_insert(0) += 1; }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn open_incidents<'a>(rows: &'a [IncidentRecord]) -> Vec<&'a IncidentRecord> {
    rows.iter().filter(|i| i.state != "Resolved").collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IncidentsViewError> {
    let incs = list_incidents(state, ctx)?;
    let open_n = open_incidents(&incs).len();
    let groups = group_by_severity(&incs);
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n)).collect();
    let rows: Vec<Vec<String>> = incs.iter().map(|i| vec![
        i.id.clone(), i.title.clone(), i.severity.into(), i.state.into(), i.opened_unix.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Grafana OnCall incidents (cave-incidents). Upstream: <a class="text-blue-700 underline" href="https://grafana.com/docs/oncall/latest/">grafana.com/docs/oncall</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> incidents</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{open}</strong> open</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Incidents ({n})</h2>{tbl}
</section>"#,
        n = incs.len(),
        open = open_n,
        chips = chips,
        tbl = table(&["id", "title", "severity", "state", "opened"], &rows),
    );
    Ok(page_shell(&format!("incidents · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/incidents/src/components/IncidentsList.tsx", "IncidentsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_incidents_filters_and_orders_newest_first() {
        let (_c, _t) = portal_test_ctx!("plugins/incidents/src/components/IncidentsList.tsx", "IncidentsList", "acme");
        let s = AdminState::seeded();
        let i = list_incidents(&s, &ctx(&[Permission::IncidentsRead])).unwrap();
        assert_eq!(i.len(), 2);
        assert!(i[0].opened_unix >= i[1].opened_unix);
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_incidents(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn transition_updates_state() {
        let (_c, _t) = portal_test_ctx!("plugins/incidents/src/components/StateSelect.tsx", "StateSelect", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::IncidentsRead, Permission::IncidentsWrite]);
        transition(&s, &c, "INC-2026-001", "Resolved").unwrap();
        let inc = list_incidents(&s, &c).unwrap();
        assert_eq!(inc.iter().find(|x| x.id == "INC-2026-001").unwrap().state, "Resolved");
    }

    #[test]
    fn transition_rejects_invalid_state_and_cross_tenant() {
        let (_c, _t) = portal_test_ctx!("plugins/incidents/src/components/StateSelect.tsx", "validateState", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::IncidentsRead, Permission::IncidentsWrite]);
        assert!(matches!(transition(&s, &c, "INC-2026-001", "Pondering").unwrap_err(), IncidentsViewError::InvalidState(_)));
        assert!(matches!(transition(&s, &c, "EVIL-001", "Resolved").unwrap_err(), IncidentsViewError::IncidentNotFound(_)));
    }

    #[test]
    fn group_by_severity_counts() {
        let i = list_incidents(&AdminState::seeded(), &ctx(&[Permission::IncidentsRead])).unwrap();
        let g = group_by_severity(&i);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), i.len());
    }

    #[test]
    fn open_incidents_excludes_resolved() {
        let i = list_incidents(&AdminState::seeded(), &ctx(&[Permission::IncidentsRead])).unwrap();
        let o = open_incidents(&i);
        assert!(o.iter().all(|x| x.state != "Resolved"));
    }

    #[test]
    fn render_includes_open_count_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::IncidentsRead])).unwrap();
        assert!(html.contains("open"));
        assert!(html.contains("grafana.com/docs/oncall"));
    }

    #[test]
    fn render_excludes_evil_incident() {
        let (_c, _t) = portal_test_ctx!("plugins/incidents/src/components/IncidentsPage.tsx", "IncidentsPage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::IncidentsRead])).unwrap();
        assert!(html.contains("Incidents (2)"));
        assert!(html.contains("INC-2026-001"));
        assert!(!html.contains("EVIL-001"));
    }
}
