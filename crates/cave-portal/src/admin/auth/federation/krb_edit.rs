// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 themes/src/main/resources/theme/keycloak.v2/admin/messages/kerberos-provider.html

use super::ProviderRow;
use crate::admin::render::escape;

pub fn render(row: &ProviderRow) -> String {
    format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Kerberos / SPNEGO — {name}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Backed by <code>cave_auth::federation::kerberos</code>.  The
    portal accepts SPNEGO via the
    <code>WWW-Authenticate: Negotiate</code> handshake; AP-REQ ticket
    verification is delegated to libgssapi (see Phase 2 backlog).
  </p>
  <form class="space-y-3 max-w-2xl">
    <div>
      <label class="block text-xs font-semibold mb-1">Default realm</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="ACME.CORP">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Server principal (SPN)</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="HTTP/portal.acme.corp@ACME.CORP">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Keytab path</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="/etc/cave/secrets/portal.keytab">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">KDC URL</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="{url}">
    </div>
    <div>
      <label class="block text-xs font-semibold mb-1">Allowed enctypes</label>
      <input class="w-full px-3 py-2 border rounded" disabled value="aes256-cts-hmac-sha1-96, aes128-cts-hmac-sha1-96">
    </div>
  </form>
  <div class="mt-4 flex gap-2 text-sm">
    <a href="/admin/auth/federation/{id}/test"
       class="px-3 py-1.5 rounded bg-blue-600 text-white">Test ticket</a>
  </div>
</section>"#,
        id = escape(&row.id),
        name = escape(&row.display_name),
        url = escape(&row.connection_url),
    )
}

#[cfg(test)]
mod tests {
    use super::super::seeded_rows;
    use super::*;

    #[test]
    fn krb_edit_renders_realm_and_spn() {
        let rows = seeded_rows();
        let r = rows.iter().find(|r| r.id == "acme-krb5").unwrap();
        let html = render(r);
        assert!(html.contains("ACME.CORP"));
        assert!(html.contains("HTTP/portal.acme.corp"));
        assert!(html.contains("Keytab"));
    }

    #[test]
    fn krb_edit_renders_kdc_url_from_row() {
        let rows = seeded_rows();
        let r = rows.iter().find(|r| r.id == "acme-krb5").unwrap();
        let html = render(r);
        assert!(html.contains("kdc.acme.corp"));
    }
}
