// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 themes/src/main/resources/theme/keycloak.v2/admin/messages/userFederation.html

use super::ProviderRow;
use crate::admin::render::{escape, table};

pub fn render(rows: &[ProviderRow]) -> String {
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/auth/federation/{id}">{name}</a>"#,
                    id = escape(&r.id),
                    name = escape(&r.display_name),
                ),
                escape(r.kind.as_str()),
                escape(&r.vendor),
                escape(&r.edit_mode),
                escape(&r.sync_policy),
                escape(&r.connection_url),
                escape(r.last_sync_iso.as_deref().unwrap_or("—")),
                r.users_imported.to_string(),
                escape(&r.last_bind_result),
            ]
        })
        .collect();

    format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Keycloak <code>User Federation</code> parity.  Add an LDAP /
    Active Directory / Kerberos provider here.  Cave-auth's
    <code>federation/</code> backend handles RFC 4511 LDAPv3 + RFC 4178
    SPNEGO directly — no JNDI dependency.
  </p>
  <div class="mb-4 flex gap-2">
    <a href="/admin/auth/federation/new?kind=ldap"
       class="px-3 py-1.5 rounded bg-blue-600 text-white text-sm">+ LDAP provider</a>
    <a href="/admin/auth/federation/new?kind=kerberos"
       class="px-3 py-1.5 rounded bg-gray-700 text-white text-sm">+ Kerberos provider</a>
  </div>
  <h2 class="text-lg font-semibold mb-2">Providers ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["name", "kind", "vendor", "edit-mode", "sync", "url", "last-sync", "users", "last-bind"],
            &rows_html,
        )
    )
}

#[cfg(test)]
mod tests {
    use super::super::seeded_rows;
    use super::*;

    #[test]
    fn render_table_lists_every_provider() {
        let rows = seeded_rows();
        let html = render(&rows);
        assert!(html.contains("OpenLDAP"));
        assert!(html.contains("Active Directory"));
        assert!(html.contains("SPNEGO"));
    }

    #[test]
    fn render_includes_new_buttons() {
        let html = render(&seeded_rows());
        assert!(html.contains("LDAP provider"));
        assert!(html.contains("Kerberos provider"));
    }

    #[test]
    fn render_includes_link_to_detail() {
        let html = render(&seeded_rows());
        assert!(html.contains("/admin/auth/federation/acme-openldap"));
    }
}
