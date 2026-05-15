// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 services/src/main/java/org/keycloak/services/resources/admin/UserStorageProviderResource.java#testLDAPConnection

use super::{ProviderKind, ProviderRow};
use crate::admin::render::escape;

pub fn render(row: &ProviderRow) -> String {
    let actions = match row.kind {
        ProviderKind::Ldap => format!(
            r#"<div class="flex flex-col gap-3">
  <button class="px-3 py-1.5 rounded bg-blue-600 text-white text-sm w-fit"
          hx-post="/admin/auth/federation/{id}/test-bind"
          hx-swap="outerHTML">Test bind</button>
  <button class="px-3 py-1.5 rounded bg-gray-700 text-white text-sm w-fit"
          hx-post="/admin/auth/federation/{id}/sync-now"
          hx-swap="outerHTML">Sync now</button>
  <button class="px-3 py-1.5 rounded bg-gray-500 text-white text-sm w-fit"
          hx-post="/admin/auth/federation/{id}/preview-import"
          hx-swap="outerHTML">Preview import</button>
</div>"#,
            id = escape(&row.id)
        ),
        ProviderKind::Kerberos => format!(
            r#"<div class="flex flex-col gap-3">
  <button class="px-3 py-1.5 rounded bg-blue-600 text-white text-sm w-fit"
          hx-post="/admin/auth/federation/{id}/test-ticket"
          hx-swap="outerHTML">Test ticket</button>
  <button class="px-3 py-1.5 rounded bg-gray-700 text-white text-sm w-fit"
          hx-post="/admin/auth/federation/{id}/inspect-keytab"
          hx-swap="outerHTML">Inspect keytab</button>
</div>"#,
            id = escape(&row.id)
        ),
    };
    format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Test — {name}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Buttons below dispatch background jobs against the
    <code>cave_auth::federation</code> backend.  Test bind issues a
    BindRequest with the configured credentials; Sync now triggers a
    one-shot full sync against the directory.  Test ticket calls the
    SPNEGO state machine with a freshly-minted challenge token.
  </p>
  {actions}
  <h3 class="text-md font-semibold mt-6 mb-2">Recent activity</h3>
  <p class="text-sm text-gray-600">Last bind: {last_bind} · Last sync: {last_sync} · Users imported: {n}</p>
</section>"#,
        name = escape(&row.display_name),
        actions = actions,
        last_bind = escape(&row.last_bind_result),
        last_sync = escape(row.last_sync_iso.as_deref().unwrap_or("—")),
        n = row.users_imported,
    )
}

#[cfg(test)]
mod tests {
    use super::super::seeded_rows;
    use super::*;

    #[test]
    fn test_panel_shows_test_bind_for_ldap_provider() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-openldap").unwrap();
        let html = render(&r);
        assert!(html.contains("Test bind"));
        assert!(html.contains("Sync now"));
    }

    #[test]
    fn test_panel_shows_test_ticket_for_kerberos_provider() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-krb5").unwrap();
        let html = render(&r);
        assert!(html.contains("Test ticket"));
        assert!(html.contains("Inspect keytab"));
    }

    #[test]
    fn test_panel_wires_htmx_post_to_backend_route() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-openldap").unwrap();
        let html = render(&r);
        assert!(html.contains("/admin/auth/federation/acme-openldap/test-bind"));
        assert!(html.contains("/admin/auth/federation/acme-openldap/sync-now"));
    }

    #[test]
    fn test_panel_renders_recent_activity_summary() {
        let r = seeded_rows().into_iter().find(|r| r.id == "acme-ad").unwrap();
        let html = render(&r);
        assert!(html.contains("Last bind"));
        assert!(html.contains("Users imported"));
    }
}
