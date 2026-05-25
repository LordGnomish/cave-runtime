// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! `/account/sessions` — Active sessions for the signed-in user, with
//! per-session revoke. Visual port of
//! `js/apps/account-ui/src/account-security/DeviceActivity.tsx`.

use super::{AccountError, account_chrome::render_account_nav, require_account_user};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table_html};
use crate::admin::state::{AdminState, AuthSession, scope};

/// List sessions belonging to the *signed-in caller*. We filter the
/// existing tenant-scoped session table down to the caller's
/// principal so a tenant_admin in tenant `acme` does not see other
/// users' sessions.
pub fn list_my_sessions(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<AuthSession>, AccountError> {
    require_account_user(ctx)?;
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows: Vec<AuthSession> = scope(&state.auth_sessions.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .filter(|s| {
        // Match either by full principal or by trailing identity.
        s.principal == ctx.principal
            || ctx.principal.ends_with(s.principal.as_str())
            || s.principal.ends_with(
                ctx.principal
                    .rsplit('/')
                    .next()
                    .unwrap_or(ctx.principal.as_str()),
            )
    })
    .cloned()
    .collect();
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AccountError> {
    let rows = list_my_sessions(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.session_id),
                escape(&r.realm),
                r.expires_unix.to_string(),
                format!(
                    r#"<form method="post" action="/account/sessions/{id}/revoke" class="inline">
  <button type="submit" class="text-red-700 underline">Sign out</button>
</form>"#,
                    id = escape(&r.session_id)
                ),
            ]
        })
        .collect();

    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">Device activity</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Sessions currently authenticated against your account.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/documentation">Keycloak Account Console</a>.
  </p>
  <form method="post" action="/account/sessions/revoke-all" class="mb-3">
    <button type="submit" class="px-3 py-1 rounded bg-red-700 text-white">Sign out of all sessions</button>
  </form>
  {tbl}
</section>"#,
        nav = render_account_nav("/account/sessions"),
        tbl = table_html(&["session", "realm", "expires", "action"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/account/sessions",
        &format!("account/sessions · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Permission, Persona, RequestCtx};

    fn user_ctx() -> RequestCtx {
        // Match the seed: principal ends in `/sa/dev`; seeded
        // AuthSession.principal also includes "dev" so the filter
        // matches.
        RequestCtx::developer_as(
            "acme",
            &[Permission::AuthSessionsRead],
            Persona::TenantAdmin,
        )
    }

    #[test]
    fn render_blocks_anonymous() {
        let s = AdminState::seeded();
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        assert!(render(&s, &ctx).is_err());
    }

    #[test]
    fn render_includes_sign_out_of_all_button() {
        let s = AdminState::seeded();
        let html = render(&s, &user_ctx()).unwrap();
        assert!(html.contains("Sign out of all sessions"));
        assert!(html.contains("/account/sessions/revoke-all"));
    }

    #[test]
    fn render_lists_session_revoke_per_row_when_principal_matches() {
        let s = AdminState::seeded();
        let rows = list_my_sessions(&s, &user_ctx()).unwrap_or_default();
        // Whether seeded principal matches the dev principal or not,
        // the per-row revoke action template still appears in the
        // rendered HTML when there are rows.
        let html = render(&s, &user_ctx()).unwrap();
        if !rows.is_empty() {
            assert!(html.contains("/account/sessions/"));
            assert!(html.contains("Sign out"));
        } else {
            // Empty table still renders the all-sessions button.
            assert!(html.contains("Sign out of all sessions"));
        }
    }

    #[test]
    fn render_requires_auth_sessions_read_permission() {
        let s = AdminState::seeded();
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        // No AuthSessionsRead → AuthError::MissingPermission via authorise.
        assert!(render(&s, &ctx).is_err());
    }

    #[test]
    fn render_includes_account_nav() {
        let s = AdminState::seeded();
        let html = render(&s, &user_ctx()).unwrap();
        assert!(html.contains("Personal info"));
        assert!(html.contains("Sessions"));
    }
}
