// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/auth/sessions` — Keycloak "Sessions" tab.
//! Per-session detail view, complementary to the realm-grouped
//! summary on the parent page. Operators reach this tab to
//! revoke a specific session.
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_users_resource>
//! (Sessions are managed via the Users + Realms endpoints in
//! the REST API — the UI exposes them as a dedicated tab.)

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, AuthSession};
use super::AuthViewError;

pub fn list_sessions(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<AuthSession>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut rows: Vec<AuthSession> =
        scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.expires_unix.cmp(&b.expires_unix));
    Ok(rows)
}

/// Sessions that will expire within `window_seconds` of `now`,
/// for the operator's "expiring soon" badge.
pub fn expiring_within<'a>(rows: &'a [AuthSession], now: i64, window: i64) -> Vec<&'a AuthSession> {
    rows.iter()
        .filter(|s| s.expires_unix > now && s.expires_unix - now <= window)
        .collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_sessions(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.session_id),
                escape(&r.principal),
                escape(&r.realm),
                r.expires_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Sessions ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-session view, ordered by expiry. Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html">Keycloak Admin REST</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["session", "principal", "realm", "expires"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/sessions",
        &format!("auth/sessions · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_seeded_rows_for_tenant() {
        let rows = list_sessions(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().any(|s| s.session_id == "sess-aaa"));
    }

    #[test]
    fn list_excludes_other_tenants() {
        let rows = list_sessions(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().all(|s| s.session_id != "sess-evil"));
    }

    #[test]
    fn list_sorts_by_expiry_ascending() {
        let rows = list_sessions(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].expires_unix <= w[1].expires_unix);
        }
    }

    #[test]
    fn expiring_within_window_finds_sessions() {
        let rows = list_sessions(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        // Window large enough to include every seeded session.
        let near = expiring_within(&rows, 0, i64::MAX);
        assert_eq!(near.len(), rows.len());
        // Window of 0 — nothing strictly within zero seconds.
        let none = expiring_within(&rows, 0, 0);
        assert!(none.is_empty());
    }

    #[test]
    fn render_includes_session_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("sess-aaa"));
        assert!(html.contains("Sessions ("));
    }
}
