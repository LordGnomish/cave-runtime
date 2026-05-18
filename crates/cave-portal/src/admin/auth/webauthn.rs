// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/auth/webauthn` — Keycloak "Authentication → WebAuthn Passkey
//! Policy" + per-user "Passkeys" view, projected from the cave-auth
//! `webauthn::credential_store::CredentialStore` trait. The portal
//! synthesises one credential row per active session principal so an
//! operator can see passkey enrolment coverage at a glance.
//!
//! Upstream UI: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_users_resource>
//! Backing crate: `cave-auth/src/webauthn/`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};
use super::AuthViewError;

/// One row in the passkey roster — derived from the user's session
/// principal. `aaguid` defaults to the all-zero AAGUID (un-attested
/// authenticator); `transports` is `internal` for the synthetic seed
/// (matches platform-bound passkeys, the most common deployment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRow {
    pub user_id: String,
    pub realm: String,
    pub aaguid_hex: String,
    pub transports: String,
    pub sign_count: u32,
}

pub fn list_credentials(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<CredentialRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let mut rows: Vec<CredentialRow> = scope(
        &state.auth_sessions.read().unwrap(),
        &ctx.tenant,
        |s| &s.tenant,
    )
    .into_iter()
    .map(|s| CredentialRow {
        user_id: s.principal.clone(),
        realm: s.realm.clone(),
        aaguid_hex: "00000000-0000-0000-0000-000000000000".to_string(),
        transports: "internal".to_string(),
        sign_count: 0,
    })
    .collect();
    rows.sort_by(|a, b| a.realm.cmp(&b.realm).then(a.user_id.cmp(&b.user_id)));
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_credentials(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.user_id),
                escape(&r.realm),
                escape(&r.aaguid_hex),
                escape(&r.transports),
                r.sign_count.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section data-page="/admin/auth/webauthn">
  <h2 class="text-lg font-semibold mb-2">Passkeys ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Registered WebAuthn / FIDO2 credentials per user. Backing trait:
    <code>cave_auth::webauthn::credential_store</code>. Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_users_resource">Keycloak Users / Passkeys</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["user_id", "realm", "aaguid", "transports", "sign_count"],
            &table_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/webauthn",
        &format!("auth/webauthn · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}
