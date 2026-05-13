//! `/admin/auth/realms` — Keycloak Admin Console "Realm Settings"
//! tab. Lists the realms the operator's tenant touches, derived
//! from the live session list (one realm row per distinct
//! `AuthSession::realm`).
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_realms_admin_resource>

use std::collections::BTreeMap;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState};
use super::AuthViewError;

/// One realm-row as the operator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmRow {
    pub realm: String,
    pub active_sessions: usize,
    /// Largest `expires_unix` observed under this realm — proxy
    /// for "freshest session", same surface Keycloak exposes
    /// under `Last login`.
    pub last_seen_unix: i64,
}

pub fn list_realms(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<RealmRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut grouped: BTreeMap<String, RealmRow> = BTreeMap::new();
    for s in scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant) {
        let row = grouped.entry(s.realm.clone()).or_insert_with(|| RealmRow {
            realm: s.realm.clone(),
            active_sessions: 0,
            last_seen_unix: i64::MIN,
        });
        row.active_sessions += 1;
        row.last_seen_unix = row.last_seen_unix.max(s.expires_unix);
    }
    Ok(grouped.into_values().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_realms(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.realm),
                r.active_sessions.to_string(),
                r.last_seen_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Realms ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Realm Settings — derived from live sessions. Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_realms_admin_resource">Keycloak Admin REST</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["realm", "active sessions", "last seen"], &table_rows),
    );
    Ok(page_shell(
        &format!("auth/realms · {}", escape(ctx.tenant.as_str())),
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
    fn list_groups_sessions_by_realm() {
        let s = AdminState::seeded();
        let rows = list_realms(&s, &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().any(|r| r.realm == "acme-realm"));
    }

    #[test]
    fn list_excludes_other_tenants() {
        let s = AdminState::seeded();
        let rows = list_realms(&s, &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().all(|r| r.realm != "evil-realm"));
    }

    #[test]
    fn list_counts_sessions_per_realm() {
        let s = AdminState::seeded();
        let rows = list_realms(&s, &ctx(&[Permission::AuthSessionsRead])).unwrap();
        let acme = rows.iter().find(|r| r.realm == "acme-realm").unwrap();
        assert_eq!(acme.active_sessions, 2);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_realms(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_realm_count_and_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("Realms ("));
        assert!(html.contains("acme-realm"));
    }
}
