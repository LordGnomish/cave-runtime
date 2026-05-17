// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/idp` — Identity-provider instances. Calls A5's
//! `admin_idp` endpoints. Visual port of
//! `js/apps/admin-ui/src/identity-providers/IdentityProvidersSection.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdpRow {
    pub alias: String,
    pub provider_id: String,
    pub display_name: String,
    pub enabled: bool,
    pub trust_email: bool,
    pub first_broker_login_flow: String,
}

pub fn seeded_idps() -> Vec<IdpRow> {
    vec![
        IdpRow {
            alias: "github".into(),
            provider_id: "github".into(),
            display_name: "GitHub".into(),
            enabled: true,
            trust_email: false,
            first_broker_login_flow: "first broker login".into(),
        },
        IdpRow {
            alias: "google".into(),
            provider_id: "google".into(),
            display_name: "Google Workspace".into(),
            enabled: true,
            trust_email: true,
            first_broker_login_flow: "first broker login".into(),
        },
        IdpRow {
            alias: "okta".into(),
            provider_id: "saml".into(),
            display_name: "Okta (SAML)".into(),
            enabled: false,
            trust_email: false,
            first_broker_login_flow: "first broker login".into(),
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let idps = seeded_idps();
    let rows: Vec<Vec<String>> = idps
        .iter()
        .map(|i| {
            vec![
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/idp/{a}">{a}</a>"#,
                    a = escape(&i.alias)
                ),
                escape(&i.display_name),
                format!(r#"<code class="text-xs">{}</code>"#, escape(&i.provider_id)),
                if i.enabled {
                    r#"<span class="text-green-700">enabled</span>"#.into()
                } else {
                    r#"<span class="text-zinc-500">disabled</span>"#.into()
                },
                if i.trust_email {
                    r#"<span class="text-green-700">✓</span>"#.into()
                } else {
                    "".into()
                },
                escape(&i.first_broker_login_flow),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Identity providers ({n})</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/idp/new">Add provider</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Brokered identity providers (OIDC, SAML 2.0, social).
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_identity_providers_resource">Keycloak Identity Providers</a>.
  </p>
  {tbl}
  <h3 class="text-base font-semibold mt-6 mb-2">Add a provider</h3>
  <div class="flex flex-wrap gap-2">
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/idp/new?provider=oidc">OpenID Connect v1.0</a>
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/idp/new?provider=saml">SAML v2.0</a>
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/idp/new?provider=google">Google</a>
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/idp/new?provider=github">GitHub</a>
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/idp/new?provider=facebook">Facebook</a>
    <a class="px-3 py-1 rounded bg-zinc-100 dark:bg-zinc-800" href="/admin/auth/idp/new?provider=microsoft">Microsoft</a>
  </div>
</section>"#,
        nav = render_admin_nav("/admin/auth/idp"),
        n = idps.len(),
        tbl = table_html(
            &["alias", "display name", "provider", "status", "trust email", "first broker flow"],
            &rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/idp",
        &format!("auth/idp · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn seeded_idps_covers_github_google_okta() {
        let i = seeded_idps();
        assert!(i.iter().any(|x| x.alias == "github"));
        assert!(i.iter().any(|x| x.alias == "google"));
        assert!(i.iter().any(|x| x.provider_id == "saml"));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_lists_add_provider_options() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("OpenID Connect"));
        assert!(html.contains("SAML v2.0"));
        assert!(html.contains("Google"));
    }

    #[test]
    fn render_marks_disabled_idps_visibly() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Okta"));
        assert!(html.contains("disabled"));
    }
}
