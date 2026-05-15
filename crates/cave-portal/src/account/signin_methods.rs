// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/account/signin-methods` — Keycloak Account console "Signing in".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/account-security/SigningIn.tsx`.

use crate::account::{account_shell, fixtures};
use crate::admin::render::escape;

/// Render the sign-in methods page.
pub fn render(principal: &str) -> String {
    let creds = fixtures::credentials(principal);
    let mut rows = String::new();
    for c in &creds {
        let kind_label = match c.kind.as_str() {
            "password" => "Password",
            "otp" => "Authenticator app (OTP)",
            "webauthn" | "webauthn-passwordless" => "Passkey / Security key",
            other => other,
        };
        let delete_btn = if c.removable {
            format!(
                r#"<form method="post" action="/account/signin-methods/delete" class="inline" onsubmit="return confirm('Remove this credential?');">
  <input type="hidden" name="credential_id" value="{cid}">
  <button type="submit" class="text-red-700 hover:underline text-sm">Remove</button>
</form>"#,
                cid = escape(&c.credential_id),
            )
        } else {
            r#"<span class="text-zinc-400 text-sm">required</span>"#.to_string()
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2">{kind}</td>
  <td class="px-3 py-2">{label}</td>
  <td class="px-3 py-2"><code class="text-xs">{cid}</code></td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{created}</td>
  <td class="px-3 py-2">{btn}</td>
</tr>"#,
            kind = escape(kind_label),
            label = escape(&c.label),
            cid = escape(&c.credential_id),
            created = c.created_unix,
            btn = delete_btn,
        ));
    }
    let body = format!(
        r#"<div class="space-y-6">
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Type</th>
      <th class="px-3 py-2 text-left">Label</th>
      <th class="px-3 py-2 text-left">Credential ID</th>
      <th class="px-3 py-2 text-left">Created</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
  <div class="flex gap-2 flex-wrap">
    <form method="post" action="/account/signin-methods/password" class="inline">
      <button class="px-3 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">Update password</button>
    </form>
    <form method="post" action="/account/signin-methods/otp/setup" class="inline">
      <button class="px-3 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">Set up authenticator app</button>
    </form>
    <form method="post" action="/account/signin-methods/webauthn/register" class="inline">
      <button class="px-3 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">Register passkey</button>
    </form>
  </div>
</div>"#,
        rows = rows,
    );
    account_shell(principal, "/account/signin-methods", "Sign-in methods", &body)
}

/// Whether a given credential is allowed to be removed.
pub fn can_remove(kind: &str) -> bool {
    !matches!(kind, "password")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_lists_all_seeded_credentials() {
        let html = render("alice");
        assert!(html.contains("Password"));
        assert!(html.contains("Authenticator app"));
        assert!(html.contains("Passkey"));
    }

    #[test]
    fn render_emits_three_setup_buttons() {
        let html = render("alice");
        assert!(html.contains(r#"action="/account/signin-methods/password""#));
        assert!(html.contains(r#"action="/account/signin-methods/otp/setup""#));
        assert!(html.contains(r#"action="/account/signin-methods/webauthn/register""#));
    }

    #[test]
    fn render_does_not_offer_remove_for_password() {
        let html = render("alice");
        // The password row's Actions column says "required", not Remove.
        // Crude check: find the password row.
        let row_start = html.find("Password").unwrap();
        let row_slice = &html[row_start..row_start.saturating_add(800)];
        assert!(row_slice.contains("required"));
    }

    #[test]
    fn render_offers_remove_for_otp_and_webauthn() {
        let html = render("alice");
        // Two delete forms (otp + webauthn).
        let count = html.matches(r#"action="/account/signin-methods/delete""#).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn can_remove_blocks_password_but_allows_others() {
        assert!(!can_remove("password"));
        assert!(can_remove("otp"));
        assert!(can_remove("webauthn"));
        assert!(can_remove("webauthn-passwordless"));
    }

    #[test]
    fn render_emits_confirm_dialog_for_destructive_action() {
        let html = render("alice");
        assert!(html.contains("Remove this credential?"));
    }
}
