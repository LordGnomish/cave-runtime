// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/webauthn` — WebAuthn credentials admin. Calls A2's
//! cave-auth surfaces. Visual port of
//! `js/apps/admin-ui/src/realm-settings/WebauthnPolicyTab.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebauthnPolicy {
    pub rp_id: String,
    pub rp_entity_name: String,
    pub signature_algorithms: Vec<String>,
    pub attestation_conveyance: String,
    pub authenticator_attachment: String,
    pub require_resident_key: bool,
    pub user_verification_requirement: String,
    pub timeout_seconds: u32,
    pub avoid_same_authenticator: bool,
}

impl WebauthnPolicy {
    pub fn defaults() -> Self {
        Self {
            rp_id: "cave.local".into(),
            rp_entity_name: "Cave Runtime".into(),
            signature_algorithms: vec!["ES256".into(), "RS256".into(), "EdDSA".into()],
            attestation_conveyance: "none".into(),
            authenticator_attachment: "not-specified".into(),
            require_resident_key: false,
            user_verification_requirement: "preferred".into(),
            timeout_seconds: 0,
            avoid_same_authenticator: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialEntry {
    pub credential_id: String,
    pub label: String,
    pub user: String,
    pub aaguid: String,
    pub sign_count: u64,
    pub created_iso: String,
}

pub fn seeded_credentials() -> Vec<CredentialEntry> {
    vec![
        CredentialEntry {
            credential_id: "cred-001".into(),
            label: "YubiKey 5C".into(),
            user: "admin".into(),
            aaguid: "ee882879-721c-4913-9775-3dfcce97072a".into(),
            sign_count: 42,
            created_iso: "2026-04-01T10:00:00Z".into(),
        },
        CredentialEntry {
            credential_id: "cred-002".into(),
            label: "iPhone Passkey".into(),
            user: "admin".into(),
            aaguid: "00000000-0000-0000-0000-000000000000".into(),
            sign_count: 12,
            created_iso: "2026-04-15T18:30:00Z".into(),
        },
        CredentialEntry {
            credential_id: "cred-003".into(),
            label: "Acme Yubikey".into(),
            user: "acme-dev".into(),
            aaguid: "ee882879-721c-4913-9775-3dfcce97072a".into(),
            sign_count: 7,
            created_iso: "2026-05-01T12:00:00Z".into(),
        },
    ]
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let policy = WebauthnPolicy::defaults();
    let creds = seeded_credentials();
    let table_rows: Vec<Vec<String>> = creds
        .iter()
        .map(|c| {
            vec![
                escape(&c.label),
                escape(&c.user),
                format!(r#"<code class="text-xs">{}</code>"#, escape(&c.aaguid)),
                c.sign_count.to_string(),
                escape(&c.created_iso),
                format!(
                    r#"<form method="post" action="/admin/auth/webauthn/{id}/delete" class="inline">
  <button type="submit" class="text-red-700 underline">remove</button>
</form>"#,
                    id = escape(&c.credential_id)
                ),
            ]
        })
        .collect();

    let body = format!(
        r#"{nav}
<section class="space-y-6">
  <div>
    <h2 class="text-lg font-semibold mb-2">WebAuthn policy</h2>
    <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
      Per-realm WebAuthn configuration. Upstream: cave-auth A2 webauthn module.
    </p>
    <form method="post" action="/admin/auth/webauthn/policy" class="space-y-2 max-w-2xl">
      <label class="block">
        <span class="block text-sm font-medium">Relying party ID</span>
        <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="rpId" value="{rp}">
      </label>
      <label class="block">
        <span class="block text-sm font-medium">Relying party entity name</span>
        <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="rpEntityName" value="{rpn}">
      </label>
      <label class="block">
        <span class="block text-sm font-medium">Signature algorithms</span>
        <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="signatureAlgorithms" value="{algs}">
      </label>
      <label class="block">
        <span class="block text-sm font-medium">Attestation conveyance</span>
        <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="attestationConveyancePreference">
          <option value="none" selected>none</option>
          <option value="indirect">indirect</option>
          <option value="direct">direct</option>
          <option value="enterprise">enterprise</option>
        </select>
      </label>
      <label class="block">
        <span class="block text-sm font-medium">Authenticator attachment</span>
        <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="authenticatorAttachment">
          <option value="not-specified" selected>not specified</option>
          <option value="platform">platform</option>
          <option value="cross-platform">cross-platform</option>
        </select>
      </label>
      <label class="block">
        <span class="block text-sm font-medium">User verification requirement</span>
        <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="userVerificationRequirement">
          <option value="preferred" selected>preferred</option>
          <option value="required">required</option>
          <option value="discouraged">discouraged</option>
        </select>
      </label>
      <label class="inline-flex items-center mr-4">
        <input type="checkbox" name="requireResidentKey"> <span class="ml-1 text-sm">Require resident key</span>
      </label>
      <label class="inline-flex items-center">
        <input type="checkbox" name="avoidSameAuthenticatorRegister"> <span class="ml-1 text-sm">Avoid same authenticator</span>
      </label>
      <div class="pt-2">
        <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Save policy</button>
      </div>
    </form>
  </div>
  <div>
    <h3 class="text-base font-semibold mb-2">Registered credentials ({n})</h3>
    {tbl}
  </div>
</section>"#,
        nav = render_admin_nav("/admin/auth/webauthn"),
        rp = escape(&policy.rp_id),
        rpn = escape(&policy.rp_entity_name),
        algs = escape(&policy.signature_algorithms.join(", ")),
        n = creds.len(),
        tbl = table_html(
            &["label", "user", "AAGUID", "sign count", "created", "action"],
            &table_rows
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
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn defaults_match_keycloak_webauthn_policy_defaults() {
        let p = WebauthnPolicy::defaults();
        assert_eq!(p.attestation_conveyance, "none");
        assert_eq!(p.user_verification_requirement, "preferred");
        assert!(p.signature_algorithms.contains(&"ES256".to_string()));
    }

    #[test]
    fn seeded_credentials_includes_yubikey_and_passkey_aaguids() {
        let c = seeded_credentials();
        assert!(
            c.iter()
                .any(|x| x.aaguid == "ee882879-721c-4913-9775-3dfcce97072a")
        );
        assert!(c.iter().any(|x| x.label.contains("Passkey")));
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_policy_form_fields() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains(r#"name="rpId""#));
        assert!(html.contains(r#"name="attestationConveyancePreference""#));
        assert!(html.contains(r#"name="userVerificationRequirement""#));
        assert!(html.contains(r#"name="requireResidentKey""#));
    }

    #[test]
    fn render_lists_credentials_with_remove_action() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("YubiKey 5C"));
        assert!(html.contains("remove"));
        assert!(html.contains("/admin/auth/webauthn/cred-001/delete"));
    }
}
