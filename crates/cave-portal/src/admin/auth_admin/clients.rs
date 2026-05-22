// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/clients` — OAuth/OIDC client registrations + scopes +
//! protocol mappers. Visual port of
//! `js/apps/admin-ui/src/clients/ClientsSection.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    Oidc,
    Saml,
}
impl Protocol {
    fn as_str(&self) -> &'static str {
        match self {
            Protocol::Oidc => "openid-connect",
            Protocol::Saml => "saml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRow {
    pub client_id: String,
    pub protocol: Protocol,
    pub enabled: bool,
    pub public_client: bool,
    pub default_scopes: Vec<String>,
    pub optional_scopes: Vec<String>,
}

pub fn known_clients() -> Vec<ClientRow> {
    vec![
        ClientRow {
            client_id: "cave-portal".into(),
            protocol: Protocol::Oidc,
            enabled: true,
            public_client: true,
            default_scopes: vec!["openid".into(), "profile".into(), "email".into()],
            optional_scopes: vec!["offline_access".into()],
        },
        ClientRow {
            client_id: "cavectl".into(),
            protocol: Protocol::Oidc,
            enabled: true,
            public_client: true,
            default_scopes: vec!["openid".into(), "offline_access".into()],
            optional_scopes: vec![],
        },
        ClientRow {
            client_id: "cave-saml-sp".into(),
            protocol: Protocol::Saml,
            enabled: true,
            public_client: false,
            default_scopes: vec![],
            optional_scopes: vec![],
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let clients = known_clients();
    let rows: Vec<Vec<String>> = clients
        .iter()
        .map(|c| {
            vec![
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/clients/{cid}">{cid}</a>"#,
                    cid = escape(&c.client_id)
                ),
                format!(
                    r#"<code class="text-xs">{}</code>"#,
                    escape(c.protocol.as_str())
                ),
                if c.enabled {
                    r#"<span class="text-green-700">enabled</span>"#.into()
                } else {
                    r#"<span class="text-zinc-500">disabled</span>"#.into()
                },
                if c.public_client {
                    r#"<span class="text-amber-700">public</span>"#.into()
                } else {
                    r#"<span class="text-blue-700">confidential</span>"#.into()
                },
                c.default_scopes.join(", "),
                c.optional_scopes.join(", "),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Clients ({n})</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/clients/new">Create client</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    OAuth 2.1 / OIDC / SAML client registrations.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_clients_resource">Keycloak Clients</a>.
  </p>
  {tbl}
  <h3 class="text-base font-semibold mt-6 mb-2">Protocol mappers</h3>
  <p class="text-sm text-gray-600 dark:text-zinc-400">
    Per-client claim mappers — select a client to manage its mappers, audience overrides, role-attribute joins.
  </p>
</section>"#,
        nav = render_admin_nav("/admin/auth/clients"),
        n = clients.len(),
        tbl = table_html(
            &[
                "client_id",
                "protocol",
                "status",
                "access type",
                "default scopes",
                "optional scopes"
            ],
            &rows
        ),
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
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn known_clients_covers_portal_cli_and_saml_sp() {
        let c = known_clients();
        assert!(c.iter().any(|x| x.client_id == "cave-portal"));
        assert!(c.iter().any(|x| x.client_id == "cavectl"));
        assert!(c.iter().any(|x| x.protocol == Protocol::Saml));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_create_client_button() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Create client"));
        assert!(html.contains("/admin/auth/clients/new"));
    }

    #[test]
    fn render_shows_access_type_for_public_clients() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(">public<"));
    }

    #[test]
    fn render_lists_protocol_per_client() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("openid-connect"));
        assert!(html.contains("saml"));
    }
}
