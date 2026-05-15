// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/account/linked-accounts` — Keycloak Account console "Linked accounts".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/account-security/LinkedAccounts.tsx`.

use crate::account::{account_shell, fixtures};
use crate::admin::render::escape;

pub fn render(principal: &str) -> String {
    let accounts = fixtures::linked_accounts(principal);
    let mut rows = String::new();
    for a in &accounts {
        let status_badge = if a.linked {
            r#"<span class="px-2 py-0.5 rounded bg-green-100 dark:bg-green-900/30 text-green-900 dark:text-green-200 text-xs">linked</span>"#
        } else {
            r#"<span class="px-2 py-0.5 rounded bg-zinc-200 dark:bg-zinc-700 text-zinc-700 dark:text-zinc-200 text-xs">not linked</span>"#
        };
        let action = if a.linked {
            format!(
                r#"<form method="post" action="/account/linked-accounts/unlink" class="inline" onsubmit="return confirm('Unlink this provider?');">
  <input type="hidden" name="provider_alias" value="{alias}">
  <button class="text-red-700 hover:underline text-sm">Unlink</button>
</form>"#,
                alias = escape(&a.provider_alias),
            )
        } else {
            format!(
                r#"<form method="post" action="/account/linked-accounts/link" class="inline">
  <input type="hidden" name="provider_alias" value="{alias}">
  <button class="text-blue-700 hover:underline text-sm">Link</button>
</form>"#,
                alias = escape(&a.provider_alias),
            )
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2 font-medium">{name}</td>
  <td class="px-3 py-2"><code class="text-xs">{alias}</code></td>
  <td class="px-3 py-2">{user}</td>
  <td class="px-3 py-2">{badge}</td>
  <td class="px-3 py-2">{action}</td>
</tr>"#,
            name = escape(&a.provider_name),
            alias = escape(&a.provider_alias),
            user = escape(&a.linked_username),
            badge = status_badge,
            action = action,
        ));
    }
    let body = format!(
        r#"<div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Link a social identity provider to sign in with their credentials.
    Unlinking does not delete the account at the provider — only the link.
  </p>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Provider</th>
      <th class="px-3 py-2 text-left">Alias</th>
      <th class="px-3 py-2 text-left">Linked as</th>
      <th class="px-3 py-2 text-left">Status</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</div>"#,
        rows = rows,
    );
    account_shell(principal, "/account/linked-accounts", "Linked accounts", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_lists_seeded_providers() {
        let html = render("alice@acme");
        assert!(html.contains("GitHub"));
        assert!(html.contains("Google"));
    }

    #[test]
    fn render_shows_link_or_unlink_per_status() {
        let html = render("alice@acme");
        assert!(html.contains(r#"action="/account/linked-accounts/unlink""#));
        assert!(html.contains(r#"action="/account/linked-accounts/link""#));
    }

    #[test]
    fn render_marks_linked_status_with_badge() {
        let html = render("alice@acme");
        assert!(html.contains(">linked<"));
        assert!(html.contains(">not linked<"));
    }

    #[test]
    fn render_explains_unlinking_semantics() {
        let html = render("alice@acme");
        assert!(html.contains("does not delete"));
    }
}
