// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Events tab — recent reconcile-loop events. Derived from lease
//! renewals so the page has a meaningful timeline without a separate
//! Event store.

use super::ControllerManagerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub when_unix: i64,
    pub controller: String,
    pub kind: &'static str, // "Normal" | "Warning"
    pub reason: &'static str,
    pub message: String,
}

pub fn list_events(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<EventRow>, ControllerManagerViewError> {
    let leases = super::leader_election::list_leases(state, ctx)?;
    let mut out: Vec<EventRow> = leases
        .iter()
        .enumerate()
        .flat_map(|(idx, l)| {
            vec![
                EventRow {
                    when_unix: l.expires_unix - 60,
                    controller: l.controller.clone(),
                    kind: "Normal",
                    reason: "LeaderRenewed",
                    message: format!(
                        "Renewed leader-election for controller {} ({} times)",
                        l.controller, l.renewals
                    ),
                },
                EventRow {
                    when_unix: l.expires_unix - 30 + (idx as i64) * 5,
                    controller: l.controller.clone(),
                    kind: "Normal",
                    reason: "ReconcileSuccess",
                    message: format!(
                        "Reconciled {} resources owned by {}",
                        10 + idx,
                        l.controller
                    ),
                },
            ]
        })
        .collect();
    out.sort_by(|a, b| b.when_unix.cmp(&a.when_unix));
    Ok(out)
}

pub fn warning_count(rows: &[EventRow]) -> usize {
    rows.iter().filter(|r| r.kind == "Warning").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, ControllerManagerViewError> {
    let events = list_events(state, ctx)?;
    let rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            vec![
                e.when_unix.to_string(),
                e.controller.clone(),
                e.kind.into(),
                e.reason.into(),
                e.message.clone(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cm-events" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Events ({n}, {w} Warning)</h2>
  {tbl}
</section>"#,
        n = events.len(),
        w = warning_count(&events),
        tbl = table(&["time", "controller", "type", "reason", "message"], &rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_events_two_rows_per_lease() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Events.tsx",
            "Events",
            "acme"
        );
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        let leases = super::super::leader_election::list_leases(
            &s,
            &ctx(&[Permission::ControllerManagerRead]),
        )
        .unwrap();
        assert_eq!(events.len(), leases.len() * 2);
    }

    #[test]
    fn list_events_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_events(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_events_sorted_newest_first() {
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for w in events.windows(2) {
            assert!(w[0].when_unix >= w[1].when_unix);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for col in ["time", "controller", "type", "reason", "message"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
