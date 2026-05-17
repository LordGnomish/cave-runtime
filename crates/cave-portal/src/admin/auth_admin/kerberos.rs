// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/kerberos` — Kerberos federation config UI. Calls A3's
//! cave-auth surfaces. Visual port of
//! `js/apps/admin-ui/src/user-federation/UserFederationKerberosSettings.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KerberosConfig {
    pub display_name: String,
    pub kerberos_realm: String,
    pub server_principal: String,
    pub keytab: String,
    pub debug: bool,
    pub allow_password_authentication: bool,
    pub update_profile_first_login: bool,
}

pub fn default_config() -> KerberosConfig {
    KerberosConfig {
        display_name: "corp-kerberos".into(),
        kerberos_realm: "CORP.EXAMPLE.COM".into(),
        server_principal: "HTTP/cave.corp.example.com@CORP.EXAMPLE.COM".into(),
        keytab: "/etc/cave/krb5.keytab".into(),
        debug: false,
        allow_password_authentication: true,
        update_profile_first_login: true,
    }
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let c = default_config();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">Kerberos federation</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    SPNEGO/GSSAPI integration. Upstream: cave-auth A3 Kerberos.
  </p>
  <form method="post" action="/admin/auth/kerberos" class="space-y-3 max-w-2xl" data-kerberos-form>
    <label class="block">
      <span class="block text-sm font-medium">Display name</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="displayName" value="{n}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Kerberos realm</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="kerberosRealm" value="{r}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Server principal</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="serverPrincipal" value="{sp}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Keytab</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="keyTab" value="{kt}">
    </label>
    <label class="inline-flex items-center mr-4">
      <input type="checkbox" name="debug" {dbg}> <span class="ml-1 text-sm">Debug</span>
    </label>
    <label class="inline-flex items-center mr-4">
      <input type="checkbox" name="allowPasswordAuthentication" {pwd}> <span class="ml-1 text-sm">Allow password authentication</span>
    </label>
    <label class="inline-flex items-center">
      <input type="checkbox" name="updateProfileFirstLogin" {upd}> <span class="ml-1 text-sm">Update profile on first login</span>
    </label>
    <div class="flex gap-2 pt-2">
      <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Save</button>
      <button type="submit" formaction="/admin/auth/kerberos/test-keytab" class="px-4 py-2 rounded bg-zinc-200 dark:bg-zinc-700">Test keytab</button>
    </div>
  </form>
</section>"#,
        nav = render_admin_nav("/admin/auth/kerberos"),
        n = escape(&c.display_name),
        r = escape(&c.kerberos_realm),
        sp = escape(&c.server_principal),
        kt = escape(&c.keytab),
        dbg = if c.debug { "checked" } else { "" },
        pwd = if c.allow_password_authentication { "checked" } else { "" },
        upd = if c.update_profile_first_login { "checked" } else { "" },
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/kerberos",
        &format!("auth/kerberos · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn default_config_has_uppercase_realm_and_http_principal() {
        let c = default_config();
        assert_eq!(c.kerberos_realm, "CORP.EXAMPLE.COM");
        assert!(c.server_principal.starts_with("HTTP/"));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_kerberos_form_fields() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(r#"name="kerberosRealm""#));
        assert!(html.contains(r#"name="serverPrincipal""#));
        assert!(html.contains(r#"name="keyTab""#));
        assert!(html.contains(r#"name="allowPasswordAuthentication""#));
    }

    #[test]
    fn render_offers_test_keytab_action() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Test keytab"));
        assert!(html.contains("/admin/auth/kerberos/test-keytab"));
    }
}
