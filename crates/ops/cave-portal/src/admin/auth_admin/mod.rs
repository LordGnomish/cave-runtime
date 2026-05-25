// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/*` — Keycloak Admin Console parity (extended).
//!
//! The legacy `crate::admin::auth` module ships the original five
//! tabs (realms / clients / users / sessions / events) driven by the
//! seeded `AuthSession` table. This module ports the *rest* of the
//! React Admin UI: roles, groups, identity providers, authentication
//! flows, events log, SAML broker, WebAuthn, LDAP, Kerberos, UMA,
//! Token Exchange, DPoP, JWE config, and the OAuth endpoint catalog.
//!
//! All pages are **PlatformAdmin-only** — the gate runs at the top
//! of every `render` function via `ctx.require_persona(Persona::PlatformAdmin)`.
//!
//! Wiring note: the live-data hooks for IdP / Flows / SAML / WebAuthn
//! / LDAP / Kerberos / UMA / Token Exchange / DPoP / JWE / OAuth
//! endpoints live in the corresponding cave-auth modules owned by
//! sub-agents A1–A5. Until those endpoints land, the pages render
//! against an in-memory mock fixture so the UI shape can be reviewed
//! and screenshot-tested ahead of the backend integration.

pub mod clients;
pub mod dpop;
pub mod events;
pub mod flows;
pub mod groups;
pub mod idp;
pub mod jwe;
pub mod kerberos;
pub mod ldap;
pub mod oauth_endpoints;
pub mod realms;
pub mod roles;
pub mod saml;
pub mod token_exchange;
pub mod uma;
pub mod users;
pub mod webauthn;

use crate::admin::permission::{Persona, RequestCtx};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthAdminError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

/// Persona gate shared by every page in this module.
pub fn require_platform(ctx: &RequestCtx) -> Result<(), AuthAdminError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    Ok(())
}

/// Tabs in the Keycloak Admin Console left-hand nav, in display order.
/// Sub-tabs (e.g. realm-settings → login / themes / keys / etc.) are
/// rendered inside each tab page.
pub fn admin_tabs() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Realm settings", "/admin/auth/realms"),
        ("Clients", "/admin/auth/clients"),
        ("Client scopes", "/admin/auth/clients?tab=scopes"),
        ("Realm roles", "/admin/auth/roles"),
        ("Users", "/admin/auth/users"),
        ("Groups", "/admin/auth/groups"),
        ("Sessions", "/admin/auth-sessions"),
        ("Events", "/admin/auth/events"),
        ("Identity providers", "/admin/auth/idp"),
        ("Authentication", "/admin/auth/flows"),
        ("SAML broker", "/admin/auth/saml"),
        ("WebAuthn", "/admin/auth/webauthn"),
        ("LDAP federation", "/admin/auth/ldap"),
        ("Kerberos federation", "/admin/auth/kerberos"),
        ("UMA", "/admin/auth/uma"),
        ("Token Exchange", "/admin/auth/token-exchange"),
        ("DPoP", "/admin/auth/dpop"),
        ("JWE config", "/admin/auth/jwe"),
        ("OAuth endpoints", "/admin/auth/oauth-endpoints"),
    ]
}

/// Render the secondary navigation strip. `current_path` highlights
/// the active tab.
pub fn render_admin_nav(current_path: &str) -> String {
    let mut out = String::from(
        r#"<nav aria-label="Auth admin" class="border-b border-zinc-200 dark:border-zinc-700 mb-4 overflow-x-auto">
  <ul class="flex flex-wrap gap-1 text-sm">"#,
    );
    for (label, href) in admin_tabs() {
        let active = current_path.starts_with(href.split('?').next().unwrap_or(href))
            && (href == current_path
                || current_path
                    .starts_with(&format!("{}/", href.split('?').next().unwrap_or(href))));
        let cls = if active {
            "px-3 py-2 border-b-2 border-blue-600 text-blue-700 font-medium whitespace-nowrap"
        } else {
            "px-3 py-2 text-zinc-700 dark:text-zinc-300 hover:text-blue-700 whitespace-nowrap"
        };
        out.push_str(&format!(
            r#"<li><a class="{cls}" href="{href}">{label}</a></li>"#,
            cls = cls,
            href = crate::admin::render::escape(href),
            label = crate::admin::render::escape(label),
        ));
    }
    out.push_str("</ul></nav>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn require_platform_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(require_platform(&ctx).is_err());
    }

    #[test]
    fn require_platform_blocks_anonymous() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        assert!(require_platform(&ctx).is_err());
    }

    #[test]
    fn require_platform_admits_platform_admin() {
        let ctx = RequestCtx::developer("acme", &[]);
        assert!(require_platform(&ctx).is_ok());
    }

    #[test]
    fn admin_tabs_covers_keycloak_left_nav_set() {
        let tabs = admin_tabs();
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/realms"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/clients"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/idp"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/flows"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/webauthn"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/saml"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/dpop"));
        assert!(tabs.iter().any(|(_, h)| *h == "/admin/auth/uma"));
        assert!(tabs.len() >= 18);
    }

    #[test]
    fn render_admin_nav_marks_active_tab() {
        let html = render_admin_nav("/admin/auth/realms");
        assert!(html.contains("border-b-2 border-blue-600"));
        assert!(html.contains("Realm settings"));
    }
}
