// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/account/applications` — Keycloak Account console "Applications".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/applications/Applications.tsx`.

use crate::account::{account_shell, fixtures};
use crate::admin::render::escape;

pub fn render(principal: &str) -> String {
    let apps = fixtures::applications(principal);
    let mut rows = String::new();
    for a in &apps {
        let scopes: String = a
            .scopes
            .iter()
            .map(|s| {
                format!(
                    r#"<span class="px-2 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-xs mr-1">{s}</span>"#,
                    s = escape(s),
                )
            })
            .collect();
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2 font-medium">{name}</td>
  <td class="px-3 py-2"><code class="text-xs">{cid}</code></td>
  <td class="px-3 py-2">{scopes}</td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{last}</td>
  <td class="px-3 py-2">
    <form method="post" action="/account/applications/revoke" class="inline" onsubmit="return confirm('Revoke access for {cid_js}?');">
      <input type="hidden" name="client_id" value="{cid}">
      <button class="text-red-700 hover:underline text-sm">Revoke</button>
    </form>
  </td>
</tr>"#,
            name = escape(&a.client_name),
            cid = escape(&a.client_id),
            cid_js = escape(&a.client_id).replace('\'', "\\'"),
            scopes = scopes,
            last = a.last_used_unix,
        ));
    }
    let body = format!(
        r#"<div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Applications you have granted access to. Revoking forces re-consent on next sign-in.
  </p>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Application</th>
      <th class="px-3 py-2 text-left">Client ID</th>
      <th class="px-3 py-2 text-left">Granted scopes</th>
      <th class="px-3 py-2 text-left">Last used</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</div>"#,
        rows = rows,
    );
    account_shell(principal, "/account/applications", "Applications", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_lists_seeded_clients() {
        let html = render("alice");
        assert!(html.contains("Cave Portal"));
        assert!(html.contains("cavectl"));
        assert!(html.contains("cave-portal"));
    }

    #[test]
    fn render_emits_revoke_form_per_application() {
        let html = render("alice");
        let count = html.matches(r#"action="/account/applications/revoke""#).count();
        assert_eq!(count, 2);
    }

    #[test]
    fn render_displays_granted_scopes_as_chips() {
        let html = render("alice");
        assert!(html.contains("openid"));
        assert!(html.contains("profile"));
        assert!(html.contains("offline_access"));
    }

    #[test]
    fn render_includes_consent_re_grant_notice() {
        let html = render("alice");
        assert!(html.contains("forces re-consent"));
    }
}
