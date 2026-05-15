//! `/admin/auth/clients` — Keycloak "Clients" tab. OIDC client
//! registrations per realm. cave-portal synthesises a per-realm
//! roster from the live session principals (one client row per
//! distinct principal-host) — surfaced for the operator who
//! needs to see "which IdP-clients have anyone using them right
//! now".
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_clients_resource>

use std::collections::BTreeMap;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};
use super::AuthViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRow {
    /// The OIDC `client_id` — for the cave portal this is
    /// derived from the principal's domain part (`@host` → host).
    pub client_id: String,
    pub realm: String,
    pub session_count: usize,
}

pub fn list_clients(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ClientRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut grouped: BTreeMap<(String, String), usize> = BTreeMap::new();
    for s in scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant) {
        let client = s
            .principal
            .split_once('@')
            .map(|(_, host)| host.to_string())
            .unwrap_or_else(|| s.principal.clone());
        *grouped.entry((s.realm.clone(), client)).or_insert(0) += 1;
    }
    Ok(grouped
        .into_iter()
        .map(|((realm, client_id), session_count)| ClientRow {
            client_id,
            realm,
            session_count,
        })
        .collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_clients(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.client_id),
                escape(&r.realm),
                r.session_count.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Clients ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    OIDC client registrations. Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_clients_resource">Keycloak Clients</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["client_id", "realm", "active sessions"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/clients",
        &format!("auth/clients · {}", escape(ctx.tenant.as_str())),
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
    fn list_derives_clients_from_principal_host() {
        let rows = list_clients(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().any(|r| r.client_id == "acme"));
    }

    #[test]
    fn list_excludes_other_tenants() {
        let rows = list_clients(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().all(|r| r.client_id != "evil"));
    }

    #[test]
    fn list_groups_by_realm_and_client() {
        let rows = list_clients(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        // Two acme@acme sessions in acme-realm → one client row, count = 2.
        let r = rows.iter().find(|r| r.client_id == "acme").unwrap();
        assert_eq!(r.session_count, 2);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_clients(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_clients_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("Clients ("));
        assert!(html.contains("Keycloak Clients"));
    }
}
