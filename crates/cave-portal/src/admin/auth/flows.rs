// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/auth/flows` — Keycloak "Authentication → Flows" tab. Lists the
//! eight built-in flows Keycloak ships with so the operator can see the
//! per-realm chain at a glance. Per-flow execution editing is delegated
//! to the cave-auth admin_flows REST surface; this page is read-only.
//!
//! Upstream: <https://www.keycloak.org/docs-api/latest/rest-api/index.html#_authentication_management_resource>
//! Backing crate: `cave-auth/src/admin_flows/`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::AuthViewError;

/// One built-in Keycloak authentication flow.  `top_level=true` flows
/// can be selected as a realm's binding (browser, direct grant, …);
/// non-top-level flows are sub-flows referenced by an execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRow {
    pub alias: String,
    pub top_level: bool,
    pub built_in: bool,
    pub description: &'static str,
}

/// Keycloak v22+ ships these eight flows by default. Source:
/// services/src/main/java/org/keycloak/authentication/DefaultAuthenticationFlows.java.
const BUILTIN_FLOWS: &[(&str, bool, bool, &str)] = &[
    ("browser", true, true, "Browser based authentication"),
    ("direct-grant", true, true, "OpenID Connect Resource Owner Grant"),
    ("registration", true, true, "Registration flow"),
    ("reset-credentials", true, true, "Reset credentials for a user if they forgot their password"),
    ("clients", true, true, "Base authentication for clients"),
    ("first-broker-login", true, true, "First broker login"),
    ("http-challenge", true, true, "HTTP-Challenge (Negotiate / Basic) flow"),
    ("docker-auth", true, true, "Used by docker client to authenticate against the IdP"),
];

pub fn list_flows(_state: &AdminState, ctx: &RequestCtx) -> Result<Vec<FlowRow>, AuthViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    Ok(BUILTIN_FLOWS
        .iter()
        .map(|(alias, top_level, built_in, description)| FlowRow {
            alias: (*alias).to_string(),
            top_level: *top_level,
            built_in: *built_in,
            description,
        })
        .collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_flows(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.alias),
                r.top_level.to_string(),
                r.built_in.to_string(),
                escape(r.description),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Authentication Flows ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Built-in Keycloak flows. Per-flow execution editing via
    <code>cavectl auth admin-flows {{flows,executions,required-actions}}</code>.
    Upstream:
    <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_authentication_management_resource">Keycloak Authentication Management</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["alias", "top_level", "built_in", "description"],
            &table_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/flows",
        &format!("auth/flows · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}
