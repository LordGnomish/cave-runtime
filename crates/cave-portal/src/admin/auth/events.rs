// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/events` — Keycloak "Events" tab. Audit log of
//! admin actions. cave-portal synthesises a per-realm event
//! trail from the live session set: each session generates a
//! `LOGIN` event keyed on its `session_id` for the operator who
//! needs to see "what just happened in this realm".
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_events_resource>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};
use super::AuthViewError;

/// One audit-log row as the operator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub realm: String,
    pub event_type: &'static str,
    pub principal: String,
    pub session_id: String,
    pub unix: i64,
}

pub fn list_events(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<EventRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut events: Vec<EventRow> = Vec::new();
    for s in scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant) {
        events.push(EventRow {
            realm: s.realm.clone(),
            event_type: "LOGIN",
            principal: s.principal.clone(),
            session_id: s.session_id.clone(),
            // Synthesise an "issued at" 30 min before expiry —
            // the upstream UI orders events by this field.
            unix: s.expires_unix.saturating_sub(1800),
        });
    }
    events.sort_by(|a, b| b.unix.cmp(&a.unix));
    Ok(events)
}

pub fn count_by_type<'a>(rows: &'a [EventRow]) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.event_type).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_events(state, ctx)?;
    let by_type = count_by_type(&rows);
    let chips: String = by_type
        .iter()
        .map(|(ty, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-blue-100 text-sm">{ty} <strong>×{n}</strong></span>"#,
                ty = ty,
                n = n,
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.realm),
                r.event_type.to_string(),
                escape(&r.principal),
                escape(&r.session_id),
                r.unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Events ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Audit log of authentication actions. Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_events_resource">Keycloak Events</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        tbl = table(
            &["realm", "type", "principal", "session", "unix"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/events",
        &format!("auth/events · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_returns_one_event_per_session() {
        let s = AdminState::seeded();
        let evs = list_events(&s, &ctx(&[Permission::AuthSessionsRead])).unwrap();
        // Owner has 2 seeded sessions → 2 events.
        assert_eq!(evs.len(), 2);
    }

    #[test]
    fn list_excludes_other_tenants() {
        let evs = list_events(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(evs.iter().all(|e| e.session_id != "sess-evil"));
    }

    #[test]
    fn list_sorts_by_unix_descending() {
        let evs = list_events(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        for w in evs.windows(2) {
            assert!(w[0].unix >= w[1].unix);
        }
    }

    #[test]
    fn count_by_type_groups_events() {
        let evs = list_events(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        let by_type = count_by_type(&evs);
        assert_eq!(by_type.get("LOGIN").copied(), Some(2));
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_events(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_events_link_and_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("Events ("));
        assert!(html.contains("Keycloak Events"));
        assert!(html.contains("LOGIN"));
    }
}
