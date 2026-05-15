// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/account/device-activity` — Keycloak Account console "Device activity".
//!
//! Source: keycloak/keycloak@b825ba97 `js/apps/account-ui/src/account-security/DeviceActivity.tsx`.

use crate::account::{account_shell, fixtures};
use crate::admin::render::escape;

pub fn render(principal: &str) -> String {
    let sessions = fixtures::devices(principal);
    let mut rows = String::new();
    for s in &sessions {
        let badge = if s.current {
            r#"<span class="px-2 py-0.5 rounded bg-green-100 dark:bg-green-900/30 text-green-900 dark:text-green-200 text-xs">current</span>"#
        } else {
            ""
        };
        let revoke = if s.current {
            r#"<span class="text-zinc-400 text-sm">—</span>"#.to_string()
        } else {
            format!(
                r#"<form method="post" action="/account/device-activity/revoke" class="inline" onsubmit="return confirm('Sign out this session?');">
  <input type="hidden" name="session_id" value="{sid}">
  <button class="text-red-700 hover:underline text-sm">Sign out</button>
</form>"#,
                sid = escape(&s.session_id),
            )
        };
        rows.push_str(&format!(
            r#"<tr class="border-t">
  <td class="px-3 py-2"><code class="text-xs">{sid}</code> {badge}</td>
  <td class="px-3 py-2">{browser}</td>
  <td class="px-3 py-2">{os}</td>
  <td class="px-3 py-2">{ip}</td>
  <td class="px-3 py-2 text-zinc-500 text-sm">{last}</td>
  <td class="px-3 py-2">{revoke}</td>
</tr>"#,
            sid = escape(&s.session_id),
            badge = badge,
            browser = escape(&s.browser),
            os = escape(&s.os),
            ip = escape(&s.ip),
            last = s.last_access_unix,
            revoke = revoke,
        ));
    }
    let body = format!(
        r#"<div class="space-y-4">
  <table class="min-w-full text-sm border-collapse">
    <thead class="bg-gray-100 dark:bg-zinc-800"><tr>
      <th class="px-3 py-2 text-left">Session</th>
      <th class="px-3 py-2 text-left">Browser</th>
      <th class="px-3 py-2 text-left">OS</th>
      <th class="px-3 py-2 text-left">IP</th>
      <th class="px-3 py-2 text-left">Last access</th>
      <th class="px-3 py-2 text-left">Actions</th>
    </tr></thead>
    <tbody>{rows}</tbody>
  </table>
  <form method="post" action="/account/device-activity/logout-all"
        onsubmit="return confirm('This signs you out of every device including this one. Continue?');">
    <button class="px-3 py-2 rounded bg-red-600 text-white hover:bg-red-700">Sign out of all devices</button>
  </form>
</div>"#,
        rows = rows,
    );
    account_shell(principal, "/account/device-activity", "Device activity", &body)
}

/// Whether a revoke is allowed for the given session id. The current
/// session can never be revoked via the per-row button (it has to go
/// through "Sign out of all devices" which is gated by a confirm).
pub fn can_revoke(target_session_id: &str, current_session_id: &str) -> bool {
    target_session_id != current_session_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_marks_current_session_with_badge() {
        let html = render("alice");
        assert!(html.contains("current"));
    }

    #[test]
    fn render_emits_revoke_button_for_non_current_sessions() {
        let html = render("alice");
        assert!(html.contains(r#"action="/account/device-activity/revoke""#));
    }

    #[test]
    fn render_includes_logout_all_form() {
        let html = render("alice");
        assert!(html.contains(r#"action="/account/device-activity/logout-all""#));
    }

    #[test]
    fn render_lists_all_seeded_sessions() {
        let html = render("alice");
        assert!(html.contains("Firefox 122"));
        assert!(html.contains("Safari 17"));
    }

    #[test]
    fn can_revoke_blocks_self_revocation() {
        assert!(!can_revoke("sess-current", "sess-current"));
        assert!(can_revoke("sess-other", "sess-current"));
    }

    #[test]
    fn render_emits_double_confirm_for_destructive_actions() {
        let html = render("alice");
        // Per-row revoke + logout-all both get confirm() prompts.
        assert!(html.contains("Sign out this session?"));
        assert!(html.contains("This signs you out of every device"));
    }
}
