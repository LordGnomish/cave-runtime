// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 themes/src/main/resources/theme/keycloak.v2/admin/messages/ldap-provider.html

use super::ProviderRow;
use crate::admin::render::escape;

pub fn render(row: &ProviderRow) -> String {
    format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">LDAP Provider — {name}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Read-only view (Editing live config requires <code>auth.federation.write</code>).
    Backed by <code>cave_auth::federation::ldap</code>.
  </p>
  <form class="space-y-3 max-w-2xl">
    <div>
      <label class="block text-xs font-semibold mb-1">Vendor</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="{vendor}">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Connection URL</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="{url}">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Edit Mode</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="{edit}">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Sync Policy</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="{sync}">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Bind DN</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="cn=admin,dc=acme,dc=corp">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Users DN</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="ou=People,dc=acme,dc=corp">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Username LDAP attribute</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="uid">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">UUID LDAP attribute</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="entryUUID">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">User object classes</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="inetOrgPerson, organizationalPerson">
    </div>
  </form>
  <div class="mt-4 flex gap-2 text-sm">
    <a href="/admin/auth/federation/{id}/test"
       class="px-3 py-1.5 rounded bg-blue-600 text-white">Test bind / Sync</a>
    <a href="/admin/auth/federation/{id}/mappers"
       class="px-3 py-1.5 rounded bg-gray-700 text-white">Mappers</a>
  </div>
</section>"#,
        id = escape(&row.id),
        name = escape(&row.display_name),
        vendor = escape(&row.vendor),
        url = escape(&row.connection_url),
        edit = escape(&row.edit_mode),
        sync = escape(&row.sync_policy),
    )
}

#[cfg(test)]
mod tests {
    use super::super::seeded_rows;
    use super::*;

    #[test]
    fn ldap_edit_renders_connection_url() {
        let rows = seeded_rows();
        let r = rows.iter().find(|r| r.id == "acme-openldap").unwrap();
        let html = render(r);
        assert!(html.contains("ldap.eng.acme.corp"));
        assert!(html.contains("Connection URL"));
    }

    #[test]
    fn ldap_edit_exposes_test_link() {
        let rows = seeded_rows();
        let html = render(&rows[0]);
        assert!(html.contains("/admin/auth/federation/acme-openldap/test"));
    }

    #[test]
    fn ldap_edit_exposes_mappers_link() {
        let rows = seeded_rows();
        let html = render(&rows[0]);
        assert!(html.contains("/admin/auth/federation/acme-openldap/mappers"));
    }
}
