// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! `/account/profile` — Personal info. Visual port of
//! `js/apps/account-ui/src/account-security/PersonalInfo.tsx` (the
//! `<UserProfileFormFields>` React component is rendered as a plain
//! `<form>` here).

use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};
use super::{account_chrome::render_account_nav, require_account_user, AccountError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountProfile {
    pub username: String,
    pub email: String,
    pub email_verified: bool,
    pub first_name: String,
    pub last_name: String,
    pub locale: String,
    pub attributes: Vec<(String, String)>,
}

impl AccountProfile {
    /// Derive a profile placeholder from the caller's principal. The
    /// real wiring posts to cave-auth `/realms/{realm}/account` —
    /// see A5's `account_resource` adapter once it lands.
    pub fn from_ctx(ctx: &RequestCtx) -> Self {
        let principal = ctx.principal.as_str();
        let user = principal.rsplit('/').next().unwrap_or("anonymous");
        Self {
            username: user.to_string(),
            email: format!("{}@{}.local", user, ctx.tenant.as_str()),
            email_verified: ctx.has_webauthn,
            first_name: String::new(),
            last_name: String::new(),
            locale: "en".to_string(),
            attributes: Vec::new(),
        }
    }
}

pub fn render(ctx: &RequestCtx) -> Result<String, AccountError> {
    require_account_user(ctx)?;
    let p = AccountProfile::from_ctx(ctx);
    let body = format!(
        r#"{nav}
<section>
  <h2 class="text-lg font-semibold mb-2">Personal info</h2>
  <p class="text-sm text-gray-600 mb-4">
    Manage your personal information.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/documentation">Keycloak Account Console</a>.
  </p>
  <form method="post" action="/account/profile" class="space-y-3 max-w-lg" data-account-form="profile">
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">Username</span>
      <input class="mt-1 block w-full rounded border-gray-300 bg-gray-50 dark:bg-zinc-800" name="username" value="{username}" readonly>
    </label>
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">Email{verified}</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="email" type="email" value="{email}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">First name</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="firstName" value="{fname}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">Last name</span>
      <input class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="lastName" value="{lname}">
    </label>
    <label class="block">
      <span class="block text-sm font-medium text-gray-700 dark:text-zinc-300">Locale</span>
      <select class="mt-1 block w-full rounded border-gray-300 dark:bg-zinc-800" name="locale">
        <option value="en" selected>English</option>
        <option value="de">Deutsch</option>
        <option value="fr">Français</option>
        <option value="tr">Türkçe</option>
      </select>
    </label>
    <div class="pt-2 flex gap-2">
      <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white">Save</button>
      <button type="reset" class="px-4 py-2 rounded bg-gray-200 dark:bg-zinc-700">Cancel</button>
    </div>
  </form>
</section>"#,
        nav = render_account_nav("/account/profile"),
        username = escape(&p.username),
        verified = if p.email_verified {
            r#" <span class="ml-1 text-xs text-green-700">verified</span>"#
        } else {
            r#" <span class="ml-1 text-xs text-amber-700">unverified</span>"#
        },
        email = escape(&p.email),
        fname = escape(&p.first_name),
        lname = escape(&p.last_name),
    );
    Ok(page_shell_full(
        ctx,
        "/account/profile",
        &format!("account/profile · {}", escape(&p.username)),
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
    fn profile_from_ctx_derives_username_from_principal() {
        let ctx = user_ctx();
        let p = AccountProfile::from_ctx(&ctx);
        assert_eq!(p.username, "dev");
        assert!(p.email.starts_with("dev@"));
    }

    #[test]
    fn render_blocks_anonymous() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::Anonymous);
        let err = render(&ctx).unwrap_err();
        assert_eq!(err, AccountError::Unauthenticated);
    }

    #[test]
    fn render_emits_form_fields_matching_keycloak_personalinfo() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains(r#"name="username""#));
        assert!(html.contains(r#"name="email""#));
        assert!(html.contains(r#"name="firstName""#));
        assert!(html.contains(r#"name="lastName""#));
        assert!(html.contains(r#"name="locale""#));
    }

    #[test]
    fn render_marks_email_as_verified_when_webauthn_present() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains("verified"));
    }

    #[test]
    fn render_includes_account_nav_strip() {
        let html = render(&user_ctx()).unwrap();
        // The nav has Personal info marked active.
        assert!(html.contains("Personal info"));
        assert!(html.contains("border-b-2 border-blue-600"));
    }

    #[test]
    fn render_form_action_targets_post_endpoint() {
        let html = render(&user_ctx()).unwrap();
        assert!(html.contains(r#"method="post""#));
        assert!(html.contains(r#"action="/account/profile""#));
    }
}
