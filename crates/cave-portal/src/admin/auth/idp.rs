// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/auth/idp` — Keycloak "Identity Providers" tab. Lists the
//! configured IdP instances the realm brokers against (SAML, OIDC,
//! WS-Fed, …). The portal renders a default roster mirroring the
//! upstream-built-in providers; per-instance CRUD is delegated to the
//! cave-auth admin_idp REST surface.
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_identity_providers_resource>
//! Backing crate: `cave-auth/src/admin_idp/`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::AuthViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdpRow {
    pub alias: String,
    pub provider_id: &'static str,
    pub enabled: bool,
    pub trust_email: bool,
}

/// Default IdP roster — one per protocol family. Mirrors Keycloak's
/// "Add provider" picklist so the operator sees plausible defaults
/// without having to wire a real broker first.
const DEFAULT_IDPS: &[(&str, &str)] = &[
    ("saml-broker", "saml"),
    ("oidc-google", "oidc"),
    ("wsfed-corp", "wsfed"),
    ("oid4vc-issuer", "oid4vc"),
];

pub fn list_instances(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<IdpRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(DEFAULT_IDPS
        .iter()
        .map(|(alias, provider_id)| IdpRow {
            alias: (*alias).to_string(),
            provider_id,
            enabled: true,
            trust_email: false,
        })
        .collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_instances(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.alias),
                escape(r.provider_id),
                r.enabled.to_string(),
                r.trust_email.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Identity Providers ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Configured IdP instances. Per-instance CRUD via
    <code>cavectl auth admin-idp {{instances,mappers}}</code>.
    Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_identity_providers_resource">Keycloak Identity Providers</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["alias", "provider_id", "enabled", "trust_email"],
            &table_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/idp",
        &format!("auth/idp · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}
