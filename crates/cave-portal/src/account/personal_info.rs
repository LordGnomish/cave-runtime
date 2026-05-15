// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/account/personal-info` — Keycloak Account console "Personal info".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/personal-info/PersonalInfo.tsx`.

use crate::account::{account_shell, fixtures};
use crate::admin::render::escape;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersonalInfoError {
    EmptyField(&'static str),
    InvalidEmail,
}

/// Render the personal info form.
pub fn render(principal: &str) -> String {
    let info = fixtures::personal_info(principal);
    let attrs_rows: String = info
        .attributes
        .iter()
        .map(|(k, v)| {
            format!(
                r#"<tr class="border-t"><td class="px-3 py-2"><code>{k}</code></td><td class="px-3 py-2"><input class="w-full px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700" type="text" name="attr.{k}" value="{v}" autocomplete="off"></td></tr>"#,
                k = escape(k),
                v = escape(v),
            )
        })
        .collect();
    let body = format!(
        r#"<form method="post" action="/account/personal-info" class="space-y-4 max-w-xl">
  <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
    <label class="block">
      <span class="text-sm font-medium">First name</span>
      <input type="text" name="first_name" value="{first}" required class="mt-1 w-full px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
    </label>
    <label class="block">
      <span class="text-sm font-medium">Last name</span>
      <input type="text" name="last_name" value="{last}" class="mt-1 w-full px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
    </label>
    <label class="block md:col-span-2">
      <span class="text-sm font-medium">Email</span>
      <input type="email" name="email" value="{email}" required class="mt-1 w-full px-2 py-1 border rounded dark:bg-zinc-900 dark:border-zinc-700">
      <span class="text-xs text-zinc-500">Verified: {verified}</span>
    </label>
  </div>
  <h2 class="text-md font-semibold mt-6">Attributes</h2>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr><th class="px-3 py-2 text-left">Key</th><th class="px-3 py-2 text-left">Value</th></tr></thead>
    <tbody>{attrs}</tbody>
  </table>
  <div class="flex gap-2">
    <button type="submit" class="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700">Save</button>
    <a href="/account/personal-info" class="px-4 py-2 rounded border hover:bg-zinc-50 dark:hover:bg-zinc-800">Cancel</a>
  </div>
</form>"#,
        first = escape(&info.first_name),
        last = escape(&info.last_name),
        email = escape(&info.email),
        verified = if info.email_verified { "yes" } else { "no" },
        attrs = attrs_rows,
    );
    account_shell(principal, "/account/personal-info", "Personal info", &body)
}

/// Validate a personal-info update form body (server-side gate).
/// Mirrors the per-field `errorState` checks in upstream
/// `account-ui/src/personal-info/PersonalInfo.tsx` (zod-equivalent).
pub fn validate(first_name: &str, last_name: &str, email: &str) -> Result<(), PersonalInfoError> {
    let _ = last_name;
    if first_name.trim().is_empty() {
        return Err(PersonalInfoError::EmptyField("first_name"));
    }
    if email.trim().is_empty() {
        return Err(PersonalInfoError::EmptyField("email"));
    }
    if !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
        return Err(PersonalInfoError::InvalidEmail);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_first_last_email_inputs() {
        let html = render("alice.smith@acme");
        assert!(html.contains(r#"name="first_name""#));
        assert!(html.contains(r#"name="last_name""#));
        assert!(html.contains(r#"name="email""#));
        assert!(html.contains("Alice"));
    }

    #[test]
    fn render_includes_attribute_table() {
        let html = render("bob@acme");
        assert!(html.contains("Attributes"));
        assert!(html.contains("locale"));
        assert!(html.contains("phoneNumber"));
    }

    #[test]
    fn render_emits_post_action_to_personal_info_endpoint() {
        let html = render("u@a");
        assert!(html.contains(r#"action="/account/personal-info""#));
        assert!(html.contains(r#"method="post""#));
    }

    #[test]
    fn validate_rejects_empty_first_name() {
        let err = validate("", "x", "x@y").unwrap_err();
        assert_eq!(err, PersonalInfoError::EmptyField("first_name"));
    }

    #[test]
    fn validate_rejects_empty_email() {
        assert_eq!(
            validate("a", "b", "  ").unwrap_err(),
            PersonalInfoError::EmptyField("email")
        );
    }

    #[test]
    fn validate_rejects_email_without_at_sign() {
        assert_eq!(
            validate("a", "b", "missing-at").unwrap_err(),
            PersonalInfoError::InvalidEmail
        );
    }

    #[test]
    fn validate_rejects_email_starting_or_ending_with_at() {
        assert_eq!(validate("a", "b", "@x.com").unwrap_err(), PersonalInfoError::InvalidEmail);
        assert_eq!(validate("a", "b", "x@").unwrap_err(), PersonalInfoError::InvalidEmail);
    }

    #[test]
    fn validate_passes_well_formed_input() {
        assert!(validate("Alice", "Smith", "alice@acme.com").is_ok());
    }
}
