// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 themes/src/main/resources/theme/keycloak.v2/admin/messages — UserFederation tab
//
// `/admin/auth/federation` — LDAP + Kerberos provider Admin Console.
//
// Tabs (mirroring Keycloak's `Admin -> User Federation`):
//
//   * [`list`]      — provider roster (Add provider button)
//   * [`ldap_edit`] — LDAP config editor with attribute mapping
//   * [`krb_edit`]  — Kerberos config editor (keytab path, SPN)
//   * [`test`]      — Test bind / Sync now / Test ticket actions
//   * [`mapper`]    — Sub-mapper editor for group/role/attr

pub mod krb_edit;
pub mod ldap_edit;
pub mod list;
pub mod mapper;
pub mod test;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::types::Cite;

/// Federation provider summary surfaced on the list view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRow {
    pub id: String,
    pub display_name: String,
    pub kind: ProviderKind,
    pub vendor: String,
    pub edit_mode: String,
    pub sync_policy: String,
    pub connection_url: String,
    pub last_sync_iso: Option<String>,
    pub users_imported: u64,
    pub last_bind_result: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Ldap,
    Kerberos,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Ldap => "ldap",
            ProviderKind::Kerberos => "kerberos",
        }
    }
}

/// Seeded fixtures the Admin Console renders by default.  In
/// production these come from `AdminState.federation_providers`
/// but cave-portal's `AdminState` doesn't yet carry that field;
/// we use a deterministic small set so the portal compiles and
/// renders honestly during the OSS launch window.
pub fn seeded_rows() -> Vec<ProviderRow> {
    vec![
        ProviderRow {
            id: "acme-openldap".into(),
            display_name: "OpenLDAP — Engineering".into(),
            kind: ProviderKind::Ldap,
            vendor: "OpenLDAP".into(),
            edit_mode: "READ_ONLY".into(),
            sync_policy: "ChangedOnly".into(),
            connection_url: "ldap://ldap.eng.acme.corp:389".into(),
            last_sync_iso: Some("2026-05-15T08:00:00Z".into()),
            users_imported: 314,
            last_bind_result: "success".into(),
        },
        ProviderRow {
            id: "acme-ad".into(),
            display_name: "Active Directory — ACME.CORP".into(),
            kind: ProviderKind::Ldap,
            vendor: "AD".into(),
            edit_mode: "READ_ONLY".into(),
            sync_policy: "Full".into(),
            connection_url: "ldaps://dc1.acme.corp:636".into(),
            last_sync_iso: Some("2026-05-15T07:30:00Z".into()),
            users_imported: 2_117,
            last_bind_result: "success".into(),
        },
        ProviderRow {
            id: "acme-krb5".into(),
            display_name: "Kerberos SPNEGO — portal.acme.corp".into(),
            kind: ProviderKind::Kerberos,
            vendor: "MIT-Krb5".into(),
            edit_mode: "READ_ONLY".into(),
            sync_policy: "OnDemand".into(),
            connection_url: "krb5://kdc.acme.corp:88".into(),
            last_sync_iso: None,
            users_imported: 0,
            last_bind_result: "n/a".into(),
        },
    ]
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FederationViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("provider `{0}` not found")]
    NotFound(String),
}

/// Render the index page (`/admin/auth/federation`).
pub fn render_index(ctx: &RequestCtx) -> Result<String, FederationViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_rows();
    let body = list::render(&rows);
    Ok(page_shell_full(
        ctx,
        "/admin/auth/federation",
        &format!("auth · federation · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// Detail page for one provider.
pub fn render_detail(ctx: &RequestCtx, id: &str) -> Result<String, FederationViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_rows();
    let row = rows.iter().find(|r| r.id == id).ok_or_else(|| FederationViewError::NotFound(id.to_string()))?;
    let body = match row.kind {
        ProviderKind::Ldap => ldap_edit::render(row),
        ProviderKind::Kerberos => krb_edit::render(row),
    };
    Ok(page_shell_full(
        ctx,
        &format!("/admin/auth/federation/{id}"),
        &format!("auth · federation · {} · {}", escape(ctx.tenant.as_str()), escape(&row.display_name)),
        &body,
    ))
}

/// Render the test panel (test-bind / sync-now / test-ticket).
pub fn render_test(ctx: &RequestCtx, id: &str) -> Result<String, FederationViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_rows();
    let row = rows.iter().find(|r| r.id == id).ok_or_else(|| FederationViewError::NotFound(id.to_string()))?;
    let body = test::render(row);
    Ok(page_shell_full(
        ctx,
        &format!("/admin/auth/federation/{id}/test"),
        &format!("auth · federation · {} · test", escape(&row.display_name)),
        &body,
    ))
}

/// Render mapper editor.
pub fn render_mapper(ctx: &RequestCtx, id: &str) -> Result<String, FederationViewError> {
    ctx.authorise(Permission::AuthSessionsRead)?;
    let rows = seeded_rows();
    let row = rows.iter().find(|r| r.id == id).ok_or_else(|| FederationViewError::NotFound(id.to_string()))?;
    let body = mapper::render(row);
    Ok(page_shell_full(
        ctx,
        &format!("/admin/auth/federation/{id}/mappers"),
        &format!("auth · federation · {} · mappers", escape(&row.display_name)),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/keycloak-backend/src/api/federation.ts",
    "UserFederationApi",
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_index_lists_seeded_providers() {
        let html = render_index(&ctx(&[Permission::AuthSessionsRead])).unwrap();
        assert!(html.contains("OpenLDAP"));
        assert!(html.contains("Active Directory"));
        assert!(html.contains("SPNEGO"));
    }

    #[test]
    fn render_index_refuses_without_perm() {
        assert!(matches!(
            render_index(&ctx(&[])).err(),
            Some(FederationViewError::Auth(_))
        ));
    }

    #[test]
    fn render_detail_returns_ldap_page_for_ldap_provider() {
        let html = render_detail(&ctx(&[Permission::AuthSessionsRead]), "acme-openldap").unwrap();
        assert!(html.contains("Connection URL"));
        assert!(html.contains("ldap.eng.acme.corp"));
    }

    #[test]
    fn render_detail_returns_kerberos_page_for_krb_provider() {
        let html = render_detail(&ctx(&[Permission::AuthSessionsRead]), "acme-krb5").unwrap();
        assert!(html.contains("Kerberos"));
        assert!(html.contains("Keytab"));
    }

    #[test]
    fn render_detail_unknown_id_returns_not_found() {
        assert!(matches!(
            render_detail(&ctx(&[Permission::AuthSessionsRead]), "missing"),
            Err(FederationViewError::NotFound(_))
        ));
    }

    #[test]
    fn render_test_panel_includes_test_bind_button_for_ldap() {
        let html = render_test(&ctx(&[Permission::AuthSessionsRead]), "acme-openldap").unwrap();
        assert!(html.contains("Test bind"));
        assert!(html.contains("Sync now"));
    }

    #[test]
    fn render_test_panel_includes_test_ticket_for_kerberos() {
        let html = render_test(&ctx(&[Permission::AuthSessionsRead]), "acme-krb5").unwrap();
        assert!(html.contains("Test ticket"));
    }

    #[test]
    fn render_mapper_includes_attribute_table() {
        let html = render_mapper(&ctx(&[Permission::AuthSessionsRead]), "acme-openldap").unwrap();
        assert!(html.contains("Group mapper") || html.contains("group-ldap-mapper"));
        assert!(html.contains("Attribute mapper") || html.contains("user-attribute-ldap-mapper"));
    }

    #[test]
    fn provider_kind_as_str_returns_canonical_form() {
        assert_eq!(ProviderKind::Ldap.as_str(), "ldap");
        assert_eq!(ProviderKind::Kerberos.as_str(), "kerberos");
    }

    #[test]
    fn seeded_rows_have_three_providers() {
        assert_eq!(seeded_rows().len(), 3);
    }
}
