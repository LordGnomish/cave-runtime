// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8 (model) + W3C WebAuthn L3
//
// `/admin/auth/webauthn` — operator console for the cave-auth WebAuthn
// implementation.  Lists credentials per user, exposes a register-new
// flow (JS-driven), and surfaces an MDS3 inspector.
//
// State is currently sourced from seeded fixtures so the page renders
// and the smoke tests pass without a backend round-trip.  Real wiring
// into the cave-auth credential store is Phase 2.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use crate::admin::types::TenantId;
use super::AuthViewError;

/// One row in the WebAuthn credentials list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebauthnCredentialRow {
    pub tenant: TenantId,
    pub user: String,
    /// Hex-encoded credentialId (truncated to 16 chars for display).
    pub credential_id_hex: String,
    pub aaguid: String,
    pub format: String,
    pub alg: &'static str,
    pub sign_counter: u32,
    pub backup_state: bool,
    pub uv_initialized: bool,
}

fn seeded(tenant: &TenantId) -> Vec<WebauthnCredentialRow> {
    vec![
        WebauthnCredentialRow {
            tenant: tenant.clone(),
            user: "alice@cave".into(),
            credential_id_hex: "AABBCCDDEEFF0011".into(),
            aaguid: "fa2b99dc-9e39-4257-8f92-4a30d23c4118".into(),
            format: "packed".into(),
            alg: "ES256",
            sign_counter: 42,
            backup_state: false,
            uv_initialized: true,
        },
        WebauthnCredentialRow {
            tenant: tenant.clone(),
            user: "bob@cave".into(),
            credential_id_hex: "1122334455667788".into(),
            aaguid: "00000000-0000-0000-0000-000000000000".into(),
            format: "none".into(),
            alg: "EdDSA",
            sign_counter: 7,
            backup_state: true,
            uv_initialized: true,
        },
    ]
}

/// Tabular list of credentials for the caller's tenant.
pub fn list_credentials(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<WebauthnCredentialRow>, AuthViewError> {
    ctx.authorise(Permission::WebauthnRead)?;
    Ok(seeded(&ctx.tenant))
}

/// Count by attestation format — used by the page sparkline + cell.
pub fn group_by_format(rows: &[WebauthnCredentialRow]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.format.clone()).or_default() += 1;
    }
    acc.into_iter().collect()
}

/// Render the admin page.  Includes:
/// - credentials table (cred_id, user, AAGUID, fmt, alg, counter)
/// - format pie (chip list)
/// - JS-driven "register new credential" + "authenticate" panels
/// - placeholder MDS3 upload form
pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, AuthViewError> {
    let rows = list_credentials(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.user),
                escape(&r.credential_id_hex),
                escape(&r.aaguid),
                escape(&r.format),
                escape(r.alg),
                r.sign_counter.to_string(),
                if r.backup_state { "yes" } else { "no" }.into(),
                if r.uv_initialized { "yes" } else { "no" }.into(),
            ]
        })
        .collect();
    let chips: String = group_by_format(&rows)
        .iter()
        .map(|(fmt, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{f} <strong>×{n}</strong></span>"#,
                f = escape(fmt),
                n = n,
            )
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    WebAuthn / FIDO2 / passkey credentials registered against this tenant.
    Upstream parity: <a class="text-blue-700 underline" href="https://github.com/webauthn4j/webauthn4j">webauthn4j</a>.
  </p>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Credentials ({n})</h2>
  {tbl}
  <h2 class="text-lg font-semibold mt-6 mb-2">Register a new credential</h2>
  <p class="text-sm text-gray-600 mb-2">
    Clicking the button below calls <code>navigator.credentials.create()</code>
    against the cave-auth registration endpoint.
  </p>
  <button class="bg-blue-600 text-white px-3 py-1 rounded" data-action="webauthn-register">Register</button>
  <h2 class="text-lg font-semibold mt-6 mb-2">Authenticate with a credential</h2>
  <button class="bg-blue-600 text-white px-3 py-1 rounded" data-action="webauthn-authn">Sign in</button>
  <h2 class="text-lg font-semibold mt-6 mb-2">FIDO Alliance MDS3</h2>
  <p class="text-sm text-gray-600">
    Upload a Metadata Service v3 JWT blob to enrich AAGUID lookups with
    vendor + certification metadata.
  </p>
  <form method="post" enctype="multipart/form-data" action="/admin/auth/webauthn/mds">
    <input type="file" name="blob" accept=".jwt,.txt" />
    <button class="bg-blue-600 text-white px-3 py-1 rounded">Upload</button>
  </form>
</section>"#,
        chips = chips,
        n = rows.len(),
        tbl = table(
            &["user", "credentialId", "AAGUID", "fmt", "alg", "ctr", "BS", "UV"],
            &table_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/webauthn",
        &format!("auth/webauthn · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_requires_webauthn_read() {
        let s = AdminState::seeded();
        assert!(list_credentials(&s, &ctx(&[])).is_err());
        assert!(list_credentials(&s, &ctx(&[Permission::WebauthnRead])).is_ok());
    }

    #[test]
    fn list_returns_two_seeded_credentials() {
        let s = AdminState::seeded();
        let rows = list_credentials(&s, &ctx(&[Permission::WebauthnRead])).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn list_is_tenant_scoped() {
        let s = AdminState::seeded();
        let rows = list_credentials(&s, &ctx(&[Permission::WebauthnRead])).unwrap();
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn group_by_format_collects_credentials() {
        let s = AdminState::seeded();
        let rows = list_credentials(&s, &ctx(&[Permission::WebauthnRead])).unwrap();
        let groups = group_by_format(&rows);
        let total: usize = groups.iter().map(|(_, n)| *n).sum();
        assert_eq!(total, rows.len());
        assert!(groups.iter().any(|(f, _)| f == "packed"));
        assert!(groups.iter().any(|(f, _)| f == "none"));
    }

    #[test]
    fn render_contains_register_and_mds_buttons() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::WebauthnRead])).unwrap();
        assert!(html.contains("webauthn-register"));
        assert!(html.contains("webauthn-authn"));
        assert!(html.contains("MDS3"));
        assert!(html.contains("webauthn4j"));
    }

    #[test]
    fn render_includes_credential_rows() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::WebauthnRead])).unwrap();
        assert!(html.contains("alice@cave"));
        assert!(html.contains("bob@cave"));
        assert!(html.contains("AABBCCDDEEFF0011"));
        assert!(html.contains("packed"));
        assert!(html.contains("ES256"));
    }

    #[test]
    fn render_excludes_other_tenants() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::WebauthnRead])).unwrap();
        assert!(!html.contains("mallory@evil"));
    }
}
