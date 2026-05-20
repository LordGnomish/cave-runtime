// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/oauth-endpoints` — Enabled OAuth/OIDC endpoints per
//! realm (auth, device, ciba, revoke, par, …). Visual port of the
//! `.well-known/openid-configuration` summary panel from
//! `js/apps/admin-ui/src/realm-settings/GeneralTab.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OauthEndpoint {
    pub name: String,
    pub path_suffix: String,
    pub enabled: bool,
    pub notes: String,
    pub rfc: &'static str,
}

pub fn endpoint_catalog() -> Vec<OauthEndpoint> {
    vec![
        OauthEndpoint {
            name: "Authorization".into(),
            path_suffix: "/protocol/openid-connect/auth".into(),
            enabled: true,
            notes: "Authorization Code, Implicit (off), Hybrid".into(),
            rfc: "RFC 6749 §3.1",
        },
        OauthEndpoint {
            name: "Token".into(),
            path_suffix: "/protocol/openid-connect/token".into(),
            enabled: true,
            notes: "Authorization code + refresh + client credentials + token exchange".into(),
            rfc: "RFC 6749 §3.2",
        },
        OauthEndpoint {
            name: "Token introspect".into(),
            path_suffix: "/protocol/openid-connect/token/introspect".into(),
            enabled: true,
            notes: "Per RFC 7662".into(),
            rfc: "RFC 7662",
        },
        OauthEndpoint {
            name: "Token revoke".into(),
            path_suffix: "/protocol/openid-connect/revoke".into(),
            enabled: true,
            notes: "Per RFC 7009".into(),
            rfc: "RFC 7009",
        },
        OauthEndpoint {
            name: "UserInfo".into(),
            path_suffix: "/protocol/openid-connect/userinfo".into(),
            enabled: true,
            notes: "OIDC §5.3".into(),
            rfc: "OIDC Core §5.3",
        },
        OauthEndpoint {
            name: "Device authorization".into(),
            path_suffix: "/protocol/openid-connect/auth/device".into(),
            enabled: true,
            notes: "Per RFC 8628".into(),
            rfc: "RFC 8628",
        },
        OauthEndpoint {
            name: "CIBA".into(),
            path_suffix: "/protocol/openid-connect/ext/ciba/auth".into(),
            enabled: false,
            notes: "Decoupled / back-channel authentication".into(),
            rfc: "OpenID CIBA Core 1.0",
        },
        OauthEndpoint {
            name: "PAR (pushed auth request)".into(),
            path_suffix: "/protocol/openid-connect/ext/par/request".into(),
            enabled: true,
            notes: "Per RFC 9126".into(),
            rfc: "RFC 9126",
        },
        OauthEndpoint {
            name: "Logout (RP-initiated)".into(),
            path_suffix: "/protocol/openid-connect/logout".into(),
            enabled: true,
            notes: "OIDC RP-Initiated Logout".into(),
            rfc: "OIDC RP-Initiated Logout 1.0",
        },
        OauthEndpoint {
            name: "JWKS".into(),
            path_suffix: "/protocol/openid-connect/certs".into(),
            enabled: true,
            notes: "Per RFC 7517".into(),
            rfc: "RFC 7517",
        },
        OauthEndpoint {
            name: "OpenID configuration".into(),
            path_suffix: "/.well-known/openid-configuration".into(),
            enabled: true,
            notes: "OIDC Discovery 1.0".into(),
            rfc: "OIDC Discovery 1.0",
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let eps = endpoint_catalog();
    let rows: Vec<Vec<String>> = eps
        .iter()
        .map(|e| {
            vec![
                escape(&e.name),
                format!(
                    r#"<code class="text-xs">/realms/{{realm}}{p}</code>"#,
                    p = escape(&e.path_suffix)
                ),
                if e.enabled {
                    r#"<span class="text-green-700">enabled</span>"#.into()
                } else {
                    r#"<span class="text-zinc-500">disabled</span>"#.into()
                },
                escape(e.rfc),
                escape(&e.notes),
            ]
        })
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">OAuth / OIDC endpoints</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Per-realm enabled endpoints. The discovery document at
    <code>/.well-known/openid-configuration</code> reflects only the
    enabled ones.
  </p>
  {tbl}
</section>"#,
        nav = render_admin_nav("/admin/auth/oauth-endpoints"),
        tbl = table_html(&["endpoint", "path", "status", "RFC", "notes"], &rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/oauth-endpoints",
        &format!("auth/oauth-endpoints · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn endpoint_catalog_covers_keycloak_core_oidc_endpoints() {
        let e = endpoint_catalog();
        let names: Vec<&str> = e.iter().map(|x| x.name.as_str()).collect();
        for needed in [
            "Authorization",
            "Token",
            "UserInfo",
            "JWKS",
            "OpenID configuration",
        ] {
            assert!(names.contains(&needed), "missing {needed}");
        }
    }

    #[test]
    fn endpoint_catalog_includes_par_device_revoke_introspect() {
        let e = endpoint_catalog();
        assert!(e.iter().any(|x| x.path_suffix.contains("/par/")));
        assert!(e.iter().any(|x| x.path_suffix.contains("/device")));
        assert!(e.iter().any(|x| x.path_suffix.contains("/revoke")));
        assert!(e.iter().any(|x| x.path_suffix.contains("/introspect")));
    }

    #[test]
    fn ciba_is_disabled_by_default() {
        let e = endpoint_catalog();
        let ciba = e.iter().find(|x| x.name == "CIBA").unwrap();
        assert!(!ciba.enabled);
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_lists_endpoints_with_rfc_citations() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("RFC 9126"));
        assert!(html.contains("RFC 8628"));
        assert!(html.contains("RFC 7662"));
        assert!(html.contains("OIDC Discovery 1.0"));
    }
}
