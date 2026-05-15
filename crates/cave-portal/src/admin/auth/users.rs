// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/auth/users` — Keycloak "Users" tab. Per-realm user
//! roster. cave-portal derives the list from the principal
//! field of live sessions; one row per distinct (realm, principal)
//! pair.
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_users_resource>

use std::collections::BTreeSet;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};
use super::AuthViewError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UserRow {
    pub realm: String,
    pub principal: String,
}

pub fn list_users(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<UserRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut seen: BTreeSet<UserRow> = BTreeSet::new();
    for s in scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| &r.tenant) {
        seen.insert(UserRow {
            realm: s.realm.clone(),
            principal: s.principal.clone(),
        });
    }
    Ok(seen.into_iter().collect())
}

pub fn count_by_realm(rows: &[UserRow]) -> Vec<(String, usize)> {
    let mut acc: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.realm.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_users(state, ctx)?;
    let summary = count_by_realm(&rows);
    let chips: String = summary
        .iter()
        .map(|(r, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{r} <strong>×{n}</strong></span>"#,
                r = escape(r),
                n = n,
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![escape(&r.realm), escape(&r.principal)])
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Users ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-realm roster. Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_users_resource">Keycloak Users</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        tbl = table(&["realm", "principal"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/users",
        &format!("auth/users · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_unique_principal_per_realm() {
        let rows = list_users(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        // Each (realm, principal) pair should appear once.
        let mut by_pair = std::collections::HashSet::new();
        for r in &rows {
            assert!(by_pair.insert((r.realm.clone(), r.principal.clone())));
        }
    }

    #[test]
    fn list_excludes_other_tenants() {
        let rows = list_users(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(rows.iter().all(|r| !r.principal.contains("mallory")));
    }

    #[test]
    fn count_by_realm_sums_to_total_users() {
        let rows = list_users(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        let summary = count_by_realm(&rows);
        let total: usize = summary.iter().map(|(_, n)| *n).sum();
        assert_eq!(total, rows.len());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_users(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_keycloak_users_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("Keycloak Users"));
        assert!(html.contains("Users ("));
    }
}
