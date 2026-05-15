// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/account/groups` — Keycloak Account console "Groups" (read-only).
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/groups/Groups.tsx`.

use crate::account::{account_shell, fixtures};
use crate::admin::render::escape;

pub fn render(principal: &str) -> String {
    let groups = fixtures::group_memberships(principal);
    let mut rows = String::new();
    for g in &groups {
        let direct_badge = if g.direct {
            r#"<span class="px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-900/30 text-blue-900 dark:text-blue-200 text-xs">direct</span>"#
        } else {
            r#"<span class="px-2 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-200 text-xs">inherited</span>"#
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2"><code class="text-xs">{path}</code></td>
  <td class="px-3 py-2">{badge}</td>
</tr>"#,
            path = escape(&g.path),
            badge = direct_badge,
        ));
    }
    let body = format!(
        r#"<div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Read-only view of the groups you are a member of. Direct memberships are
    administered explicitly; inherited memberships are derived from parent groups.
  </p>
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Path</th>
      <th class="px-3 py-2 text-left">Membership</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
</div>"#,
        rows = rows,
    );
    account_shell(principal, "/account/groups", "Groups", &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_lists_seeded_group_paths() {
        let html = render("alice@acme");
        assert!(html.contains("/acme/engineering"));
        assert!(html.contains("/acme/employees"));
    }

    #[test]
    fn render_marks_direct_vs_inherited() {
        let html = render("alice@acme");
        assert!(html.contains(">direct<"));
        assert!(html.contains(">inherited<"));
    }

    #[test]
    fn render_does_not_offer_mutation_forms() {
        // Groups are read-only on the account console.
        let html = render("alice@acme");
        assert!(!html.contains(r#"action="/account/groups"#));
    }

    #[test]
    fn render_explains_inheritance() {
        let html = render("alice@acme");
        assert!(html.contains("derived from parent groups"));
    }
}
