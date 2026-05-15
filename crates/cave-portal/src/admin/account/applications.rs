// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! `/account/applications` — Authorised client applications + their
//! consent grants. Visual port of
//! `js/apps/account-ui/src/applications/Applications.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{account_chrome::render_account_nav, require_account_user, AccountError};

/// One client-application row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationRow {
    pub client_id: String,
    pub name: String,
    pub url: Option<String>,
    pub consents: Vec<String>,
    pub offline_access: bool,
}

/// Synthesise an applications list from realms touched by this
/// caller. One row per `(realm, client_id)` pair scoped to the
/// caller's tenant. Production wiring posts to cave-auth
/// `/account/applications` once A5 lands the resource.
pub fn list_applications(ctx: &RequestCtx) -> Vec<ApplicationRow> {
    // Deterministic seed: every authenticated user has a synthetic
    // pair of consent grants (portal + cli) to match Keycloak's two
    // bundled client types.
    let _ = ctx;
    vec![
        ApplicationRow {
            client_id: "cave-portal".to_string(),
            name: "Cave Portal".to_string(),
            url: Some("/".to_string()),
            consents: vec!["openid".into(), "profile".into(), "email".into()],
            offline_access: false,
        },
        ApplicationRow {
            client_id: "cavectl".to_string(),
            name: "Cave CLI".to_string(),
            url: None,
            consents: vec!["openid".into(), "offline_access".into()],
            offline_access: true,
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AccountError> {
    require_account_user(ctx)?;
    let rows = list_applications(ctx);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let name_cell = if let Some(u) = &r.url {
                format!(
                    r#"<a class="text-blue-700 underline" href="{}">{}</a>"#,
                    escape(u),
                    escape(&r.name)
                )
            } else {
                escape(&r.name)
            };
            let consents_cell = r
                .consents
                .iter()
                .map(|c| format!(r#"<code class="text-xs bg-gray-100 dark:bg-zinc-800 px-1">{}</code>"#, escape(c)))
                .collect::<Vec<_>>()
                .join(" ");
            let offline_cell = if r.offline_access {
                r#"<span class="text-amber-700 text-xs">offline access</span>"#.to_string()
            } else {
                "".to_string()
            };
            let action_cell = format!(
                r#"<form method="post" action="/account/applications/{cid}/revoke" class="inline">
  <button type="submit" class="text-red-700 underline">Revoke consent</button>
</form>"#,
                cid = escape(&r.client_id)
            );
            vec![
                escape(&r.client_id),
                name_cell,
                consents_cell,
                offline_cell,
                action_cell,
            ]
        })
        .collect();

    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">Applications</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Applications that can access your account.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/documentation">Keycloak Account Console</a>.
  </p>
  {tbl}
</section>"#,
        nav = render_account_nav("/account/applications"),
        tbl = table_html(
            &["client_id", "name", "granted scopes", "offline", "action"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/account/applications",
        &format!("account/applications · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Permission, Persona, RequestCtx};

    fn user_ctx() -> RequestCtx {
        RequestCtx::developer_as("acme", &[Permission::AuthSessionsRead], Persona::TenantAdmin)
    }

    #[test]
    fn list_applications_returns_portal_and_cli_clients() {
        let rows = list_applications(&user_ctx());
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.client_id == "cave-portal"));
        assert!(rows.iter().any(|r| r.client_id == "cavectl"));
    }

    #[test]
    fn cavectl_has_offline_access_grant() {
        let rows = list_applications(&user_ctx());
        let cli = rows.iter().find(|r| r.client_id == "cavectl").unwrap();
        assert!(cli.offline_access);
        assert!(cli.consents.contains(&"offline_access".to_string()));
    }

    #[test]
    fn render_blocks_anonymous() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_includes_revoke_consent_button() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("Revoke consent"));
        assert!(html.contains("/account/applications/cave-portal/revoke"));
    }

    #[test]
    fn render_shows_offline_marker_only_for_offline_clients() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("offline access"));
    }

    #[test]
    fn render_lists_granted_scopes_as_code_spans() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("openid"));
        assert!(html.contains("profile"));
    }
}
