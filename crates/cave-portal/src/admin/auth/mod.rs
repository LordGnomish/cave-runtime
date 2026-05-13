//! `/admin/auth` — Keycloak Admin Console parity. Lists active
//! sessions for the caller's tenant grouped by realm so the operator
//! can see per-realm utilisation at a glance, matching the upstream's
//! `Sessions` view.
//!
//! Tab layout — mirrors the Keycloak Admin Console:
//!
//! * [`realms`]   — Realm Settings
//! * [`clients`]  — OIDC client registrations
//! * [`users`]    — per-realm user roster
//! * [`sessions`] — per-session detail view
//! * [`events`]   — synthesised audit-log trail
//!
//! Upstream UI: <https://www.keycloak.org/documentation>

pub mod clients;
pub mod events;
pub mod realms;
pub mod sessions;
pub mod users;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, AuthSession};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<AuthSession>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut rows: Vec<AuthSession> =
        scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.realm.cmp(&b.realm).then(a.principal.cmp(&b.principal)));
    Ok(rows)
}

/// Group sessions by realm — Keycloak's primary scoping unit.
pub fn group_by_realm(rows: &[AuthSession]) -> Vec<(String, Vec<AuthSession>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<AuthSession>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.realm.clone()).or_default().push(r.clone());
    }
    acc.into_iter().collect()
}

/// Sessions whose `expires_unix < now` are stale and would be filtered
/// out by Keycloak's own UI. Surfaced separately so an operator can
/// see lingering dead sessions.
pub fn expired_before<'a>(rows: &'a [AuthSession], now: i64) -> Vec<&'a AuthSession> {
    rows.iter().filter(|s| s.expires_unix < now).collect()
}

pub fn detail(state: &AdminState, ctx: &RequestCtx, session_id: &str) -> Result<Option<AuthSession>, AuthViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|s| s.session_id == session_id))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_records(state, ctx)?;
    let groups = group_by_realm(&rows);
    let realm_chips: String = groups
        .iter()
        .map(|(r, v)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{r} <strong>×{n}</strong></span>"#,
                r = escape(r),
                n = v.len(),
            )
        })
        .collect();
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
  <p class="text-sm text-gray-600 mb-3">
    Keycloak Admin Console parity (cave-auth).
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/documentation">keycloak.org</a>.
  </p>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Sessions ({n})</h2>
  {tbl}
</section>"#,
        chips = realm_chips,
        n = rows.len(),
        tbl = table(&["session", "principal", "realm", "expires"], &table_rows),
    );
    Ok(page_shell(&format!("auth · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/auth/src/components/SessionsList.tsx", "SessionsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/auth/src/components/SessionsList.tsx",
            "SessionsList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_realm_collects_sessions() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        let groups = group_by_realm(&rows);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, rows.len());
    }

    #[test]
    fn expired_before_filters_stale_sessions() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        let none = expired_before(&rows, 0);
        assert!(none.is_empty());
        let all = expired_before(&rows, i64::MAX);
        assert_eq!(all.len(), rows.len());
    }

    #[test]
    fn detail_returns_session_by_id() {
        let s = AdminState::seeded();
        let rows = list_records(&s, &ctx(&[Permission::AuthSessionsRead])).unwrap();
        if let Some(first) = rows.first() {
            let id = first.session_id.clone();
            assert!(detail(&s, &ctx(&[Permission::AuthSessionsRead]), &id).unwrap().is_some());
        }
        assert!(detail(&s, &ctx(&[Permission::AuthSessionsRead]), "no-such").unwrap().is_none());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("sess-aaa"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(!html.contains("sess-evil"));
    }

    #[test]
    fn render_includes_realm_chips_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("Sessions ("));
        assert!(html.contains("keycloak.org"));
    }
}
