// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/ldap` — LDAP federation config UI. Calls A3's
//! cave-auth surfaces. Visual port of
//! `js/apps/admin-ui/src/user-federation/UserFederationLdapSettings.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};
use super::{render_admin_nav, require_platform, AuthAdminError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LdapProvider {
    pub display_name: String,
    pub vendor: LdapVendor,
    pub connection_url: String,
    pub bind_dn: String,
    pub users_dn: String,
    pub edit_mode: EditMode,
    pub sync_period_seconds: u32,
    pub use_starttls: bool,
    pub allow_kerberos_auth: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LdapVendor {
    Ad,
    Rhds,
    Tivoli,
    Novell,
    Other,
}
impl LdapVendor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ad => "Active Directory",
            Self::Rhds => "Red Hat Directory Server",
            Self::Tivoli => "Tivoli",
            Self::Novell => "Novell eDirectory",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditMode {
    ReadOnly,
    Writable,
    Unsynced,
}
impl EditMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "READ_ONLY",
            Self::Writable => "WRITABLE",
            Self::Unsynced => "UNSYNCED",
        }
    }
}

pub fn default_provider() -> LdapProvider {
    LdapProvider {
        display_name: "corp-ldap".into(),
        vendor: LdapVendor::Ad,
        connection_url: "ldaps://ldap.corp.example.com:636".into(),
        bind_dn: "cn=cave-bind,ou=Service,dc=corp,dc=example,dc=com".into(),
        users_dn: "ou=Users,dc=corp,dc=example,dc=com".into(),
        edit_mode: EditMode::ReadOnly,
        sync_period_seconds: 86400,
        use_starttls: true,
        allow_kerberos_auth: false,
    }
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let p = default_provider();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">LDAP federation</h2>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Bridge an external LDAP/AD directory into a realm.
    Upstream: cave-auth A3 LDAP federation.
  </p>
  <form method="post" action="/admin/auth/ldap" class="space-y-3 max-w-2xl" data-ldap-form>
    <label class="block">
      <span class="block text-sm font-medium">Display name</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="displayName" value="{name}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Vendor</span>
      <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="vendor">
        <option value="ad" selected>Active Directory</option>
        <option value="rhds">Red Hat Directory Server</option>
        <option value="tivoli">Tivoli</option>
        <option value="novell">Novell eDirectory</option>
        <option value="other">Other</option>
      </select>
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Connection URL</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="connectionUrl" value="{url}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Bind DN</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="bindDn" value="{bdn}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Bind credential</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="bindCredential" type="password" placeholder="••••••••">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Users DN</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="usersDn" value="{udn}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Edit mode</span>
      <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="editMode">
        <option value="READ_ONLY" selected>READ_ONLY</option>
        <option value="WRITABLE">WRITABLE</option>
        <option value="UNSYNCED">UNSYNCED</option>
      </select>
    </label>
    <label class="block">
      <span class="block text-sm font-medium">Sync period (seconds, 0 = manual)</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="syncPeriodSeconds" type="number" value="{sync}" min="0">
    </label>
    <label class="inline-flex items-center mr-4">
      <input type="checkbox" name="useStarttls" {starttls}> <span class="ml-1 text-sm">Use STARTTLS</span>
    </label>
    <label class="inline-flex items-center">
      <input type="checkbox" name="allowKerberosAuth" {krb}> <span class="ml-1 text-sm">Allow Kerberos authentication</span>
    </label>
    <div class="flex gap-2 pt-2">
      <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Save</button>
      <button type="submit" formaction="/admin/auth/ldap/test-connection" class="px-4 py-2 rounded bg-zinc-200 dark:bg-zinc-700">Test connection</button>
      <button type="submit" formaction="/admin/auth/ldap/sync-all" class="px-4 py-2 rounded bg-zinc-200 dark:bg-zinc-700">Sync all users</button>
    </div>
  </form>
</section>"#,
        nav = render_admin_nav("/admin/auth/ldap"),
        name = escape(&p.display_name),
        url = escape(&p.connection_url),
        bdn = escape(&p.bind_dn),
        udn = escape(&p.users_dn),
        sync = p.sync_period_seconds,
        starttls = if p.use_starttls { "checked" } else { "" },
        krb = if p.allow_kerberos_auth { "checked" } else { "" },
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/ldap",
        &format!("auth/ldap · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn default_provider_targets_ldaps_active_directory_read_only() {
        let p = default_provider();
        assert_eq!(p.vendor, LdapVendor::Ad);
        assert_eq!(p.edit_mode, EditMode::ReadOnly);
        assert!(p.connection_url.starts_with("ldaps://"));
    }

    #[test]
    fn edit_mode_wire_strings_match_keycloak_kc_constants() {
        assert_eq!(EditMode::ReadOnly.as_str(), "READ_ONLY");
        assert_eq!(EditMode::Writable.as_str(), "WRITABLE");
        assert_eq!(EditMode::Unsynced.as_str(), "UNSYNCED");
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_required_ldap_form_fields() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        for name in [
            "displayName",
            "vendor",
            "connectionUrl",
            "bindDn",
            "bindCredential",
            "usersDn",
            "editMode",
            "syncPeriodSeconds",
        ] {
            assert!(
                html.contains(&format!(r#"name="{}""#, name)),
                "missing form field {name}"
            );
        }
    }

    #[test]
    fn render_includes_test_connection_and_sync_actions() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Test connection"));
        assert!(html.contains("Sync all users"));
    }
}
