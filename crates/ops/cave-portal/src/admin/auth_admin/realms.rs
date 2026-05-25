// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/realms` — Realm Settings landing (extended). Visual
//! port of `js/apps/admin-ui/src/realm-settings/RealmSettingsTabs.tsx`
//! plus the list view from `js/apps/admin-ui/src/realm/RealmSection.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};

/// One realm row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmInfo {
    pub name: String,
    pub enabled: bool,
    pub display_name: String,
    pub login_theme: String,
    pub access_token_lifespan_seconds: u64,
    pub sso_session_idle_seconds: u64,
}

/// Realm fixture — every cave-auth install has at least the master
/// realm. Tenant realms get added by the realm-import flow.
pub fn known_realms() -> Vec<RealmInfo> {
    vec![
        RealmInfo {
            name: "master".into(),
            enabled: true,
            display_name: "Master".into(),
            login_theme: "keycloak".into(),
            access_token_lifespan_seconds: 300,
            sso_session_idle_seconds: 1800,
        },
        RealmInfo {
            name: "cave".into(),
            enabled: true,
            display_name: "Cave Runtime".into(),
            login_theme: "keycloak".into(),
            access_token_lifespan_seconds: 300,
            sso_session_idle_seconds: 1800,
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let realms = known_realms();
    let table_rows: Vec<Vec<String>> = realms
        .iter()
        .map(|r| {
            let badge = if r.enabled {
                r#"<span class="text-green-700">enabled</span>"#
            } else {
                r#"<span class="text-zinc-500">disabled</span>"#
            };
            vec![
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/realms/{n}">{n}</a>"#,
                    n = escape(&r.name)
                ),
                escape(&r.display_name),
                badge.to_string(),
                escape(&r.login_theme),
                r.access_token_lifespan_seconds.to_string(),
                r.sso_session_idle_seconds.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Realm settings</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/realms/new">Create realm</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Realms isolate users + clients + tokens.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_realms_admin_resource">Keycloak Admin REST</a>.
  </p>
  {tbl}
</section>"#,
        nav = render_admin_nav("/admin/auth/realms"),
        tbl = table_html(
            &[
                "realm",
                "display name",
                "status",
                "login theme",
                "AT lifespan (s)",
                "SSO idle (s)"
            ],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/realms",
        &format!("auth/realms · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn known_realms_includes_master_and_cave() {
        let r = known_realms();
        assert!(r.iter().any(|x| x.name == "master"));
        assert!(r.iter().any(|x| x.name == "cave"));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_admits_platform_admin() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Realm settings"));
        assert!(html.contains("master"));
        assert!(html.contains("Create realm"));
    }

    #[test]
    fn render_includes_admin_nav() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Clients"));
        assert!(html.contains("Identity providers"));
    }
}
