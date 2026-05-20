// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! `/account/password` — Change password. Visual port of
//! `js/apps/account-ui/src/account-security/SigningIn.tsx` →
//! `UpdatePasswordPage` form (`currentPassword`, `newPassword`,
//! `confirmation`).

use super::{AccountError, account_chrome::render_account_nav, require_account_user};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};

/// Password policy hints shown next to the input — mirrors
/// Keycloak's `passwordPolicy` realm setting (`length(8) digits(1)
/// upperCase(1) lowerCase(1) specialChars(1)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordPolicy {
    pub min_length: usize,
    pub require_digit: bool,
    pub require_upper: bool,
    pub require_lower: bool,
    pub require_special: bool,
}

impl PasswordPolicy {
    pub fn defaults() -> Self {
        Self {
            min_length: 8,
            require_digit: true,
            require_upper: true,
            require_lower: true,
            require_special: true,
        }
    }
    pub fn hints(&self) -> Vec<String> {
        let mut h = vec![format!("at least {} characters", self.min_length)];
        if self.require_upper {
            h.push("one upper-case letter".into());
        }
        if self.require_lower {
            h.push("one lower-case letter".into());
        }
        if self.require_digit {
            h.push("one digit".into());
        }
        if self.require_special {
            h.push("one special character".into());
        }
        h
    }
}

pub fn render(ctx: &RequestCtx) -> Result<String, AccountError> {
    require_account_user(ctx)?;
    let policy = PasswordPolicy::defaults();
    let hints_html: String = policy
        .hints()
        .iter()
        .map(|h| format!("<li>{}</li>", escape(h)))
        .collect();
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">Signing in · Change password</h2>
  <p class="text-sm text-gray-600 mb-3">
    Update the password used to sign in to this account.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/documentation">Keycloak Account Console</a>.
  </p>
  <ul class="text-xs text-gray-600 dark:text-zinc-400 list-disc pl-5 mb-3">
    {hints}
  </ul>
  <form method="post" action="/account/password" class="space-y-3 max-w-lg" data-account-form="password">
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">Current password</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="currentPassword" type="password" required>
    </label>
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">New password</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="newPassword" type="password" minlength="{min}" required>
    </label>
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">Confirm new password</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="confirmation" type="password" minlength="{min}" required>
    </label>
    <div class="pt-2 flex gap-2">
      <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Update password</button>
      <a href="/account/profile" class="px-4 py-2 rounded bg-gray-200 dark:bg-zinc-700">Cancel</a>
    </div>
  </form>
</section>"#,
        nav = render_account_nav("/account/password"),
        hints = hints_html,
        min = policy.min_length,
    );
    Ok(page_shell_full(
        ctx,
        "/account/password",
        &format!("account/password · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Permission, Persona, RequestCtx};

    fn user_ctx() -> RequestCtx {
        RequestCtx::developer_as(
            "acme",
            &[Permission::AuthSessionsRead],
            Persona::TenantAdmin,
        )
    }

    #[test]
    fn policy_defaults_match_keycloak_typical_settings() {
        let p = PasswordPolicy::defaults();
        assert_eq!(p.min_length, 8);
        assert!(p.require_digit);
        assert!(p.require_special);
    }

    #[test]
    fn hints_include_min_length_and_class_requirements() {
        let p = PasswordPolicy::defaults();
        let hints = p.hints();
        assert!(hints.iter().any(|h| h.contains("8")));
        assert!(hints.iter().any(|h| h.contains("digit")));
    }

    #[test]
    fn render_blocks_anonymous() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_three_password_fields() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains(r#"name="currentPassword""#));
        assert!(html.contains(r#"name="newPassword""#));
        assert!(html.contains(r#"name="confirmation""#));
    }

    #[test]
    fn render_emits_policy_hints_list() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("at least 8 characters"));
    }

    #[test]
    fn render_password_inputs_carry_required_min_length() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains(r#"minlength="8""#));
        assert!(html.contains(r#"type="password""#));
    }
}
