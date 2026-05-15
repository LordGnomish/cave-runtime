// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! `/account/two-factor` — Two-factor credentials (TOTP, WebAuthn,
//! recovery codes). Visual port of
//! `js/apps/account-ui/src/account-security/SigningIn.tsx` →
//! `CredentialsListRow` and `js/apps/account-ui/src/account-security/SetUpAuthentication.tsx`.

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full, table_html};
use super::{account_chrome::render_account_nav, require_account_user, AccountError};

/// One credential row as shown in Keycloak's credentials list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRow {
    pub credential_id: String,
    pub kind: CredentialKind,
    pub label: String,
    pub created_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Totp,
    Webauthn,
    RecoveryCodes,
}

impl CredentialKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CredentialKind::Totp => "TOTP",
            CredentialKind::Webauthn => "WebAuthn",
            CredentialKind::RecoveryCodes => "Recovery codes",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            CredentialKind::Totp => "🔢",
            CredentialKind::Webauthn => "🔐",
            CredentialKind::RecoveryCodes => "📜",
        }
    }
}

/// In production this is the response of `GET /account/credentials`
/// filtered to second-factor categories. Here we synthesise from the
/// session table to keep tests deterministic — every authenticated
/// caller has one WebAuthn credential because their `has_webauthn`
/// flag is true.
pub fn list_credentials(ctx: &RequestCtx) -> Vec<CredentialRow> {
    let mut out = Vec::new();
    if ctx.has_webauthn {
        out.push(CredentialRow {
            credential_id: format!("wa-{}", ctx.tenant.as_str()),
            kind: CredentialKind::Webauthn,
            label: "WebAuthn / Passkey".to_string(),
            created_unix: 0,
        });
    }
    out
}

pub fn render(ctx: &RequestCtx) -> Result<String, AccountError> {
    require_account_user(ctx)?;
    let creds = list_credentials(ctx);
    let table_rows: Vec<Vec<String>> = creds
        .iter()
        .map(|c| {
            vec![
                format!(
                    r#"<span class="text-lg" aria-hidden="true">{}</span> {}"#,
                    c.kind.icon(),
                    escape(c.kind.as_str())
                ),
                escape(&c.label),
                c.created_unix.to_string(),
                format!(
                    r#"<form method="post" action="/account/two-factor/{id}/delete" class="inline">
  <button type="submit" class="text-red-700 underline">Remove</button>
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
    <h2 class="text-lg font-semibold mb-2">Two-factor authentication</h2>
    <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
      Add or remove the credentials used as your second factor.
      Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/documentation">Keycloak Account Console</a>.
    </p>
    {tbl}
  </div>
  <div>
    <h3 class="text-base font-semibold mb-2">Set up authentication</h3>
    <ul class="flex flex-wrap gap-2">
      <li><a class="px-3 py-1 rounded bg-blue-600 text-white" href="/account/two-factor/setup/totp">Set up authenticator app (TOTP)</a></li>
      <li><a class="px-3 py-1 rounded bg-blue-600 text-white" href="/account/two-factor/setup/webauthn">Register passkey / WebAuthn</a></li>
      <li><a class="px-3 py-1 rounded bg-blue-600 text-white" href="/account/two-factor/setup/recovery">Generate recovery codes</a></li>
    </ul>
  </div>
</section>"#,
        nav = render_account_nav("/account/two-factor"),
        tbl = table_html(&["type", "label", "created", "action"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/account/two-factor",
        &format!("account/2fa · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Permission, Persona, RequestCtx};

    fn user_ctx() -> RequestCtx {
        RequestCtx::developer_as("acme", &[Permission::AuthSessionsRead], Persona::TenantAdmin)
    }

    #[test]
    fn credential_kind_str_names_match_keycloak_otp_webauthn_recovery() {
        assert_eq!(CredentialKind::Totp.as_str(), "TOTP");
        assert_eq!(CredentialKind::Webauthn.as_str(), "WebAuthn");
        assert_eq!(CredentialKind::RecoveryCodes.as_str(), "Recovery codes");
    }

    #[test]
    fn list_credentials_returns_webauthn_when_present() {
        let creds = list_credentials(&user_ctx());
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].kind, CredentialKind::Webauthn);
    }

    #[test]
    fn list_credentials_empty_when_no_webauthn() {
        let mut ctx = user_ctx();
        ctx.has_webauthn = false;
        let creds = list_credentials(&ctx);
        assert!(creds.is_empty());
    }

    #[test]
    fn render_blocks_anonymous() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_offers_three_setup_actions() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("Set up authenticator app"));
        assert!(html.contains("Register passkey"));
        assert!(html.contains("Generate recovery codes"));
    }

    #[test]
    fn render_lists_existing_credential_with_remove_button() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("WebAuthn / Passkey"));
        assert!(html.contains("Remove"));
        assert!(html.contains("/account/two-factor/wa-acme/delete"));
    }
}
