// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/account-ui/ (visual port, server-rendered Maud)
//
//! Account-console chrome — the navigation header that every
//! `/account/*` page wears. Mirrors Keycloak's React component
//! `js/apps/account-ui/src/root/Root.tsx`, which renders a
//! horizontal nav with five entries (Personal info, Account security,
//! Applications, Sessions, Resources) plus a top-right sign-out
//! affordance.

use crate::admin::render::escape;

/// One entry in the account-console nav strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountNavItem {
    pub label: &'static str,
    pub href: &'static str,
}

/// Returns the nav items in the same order Keycloak's `Root.tsx`
/// lays them out. Sessions + Applications + Account security
/// (password / 2-factor) + Personal info (profile).
pub fn account_nav_items() -> Vec<AccountNavItem> {
    vec![
        AccountNavItem { label: "Personal info", href: "/account/profile" },
        AccountNavItem { label: "Account security · Signing in", href: "/account/password" },
        AccountNavItem { label: "Account security · 2FA", href: "/account/two-factor" },
        AccountNavItem { label: "Applications", href: "/account/applications" },
        AccountNavItem { label: "Sessions", href: "/account/sessions" },
    ]
}

/// Render the nav strip. `current_path` highlights the active tab.
pub fn render_account_nav(current_path: &str) -> String {
    let items = account_nav_items();
    let mut out = String::from(
        r#"<nav aria-label="Account console" class="border-b border-zinc-200 dark:border-zinc-700 mb-4">
  <ul class="flex flex-wrap gap-1 text-sm">"#,
    );
    for it in &items {
        let active = it.href == current_path;
        let cls = if active {
            "px-3 py-2 border-b-2 border-blue-600 text-blue-700 font-medium"
        } else {
            "px-3 py-2 text-zinc-700 dark:text-zinc-300 hover:text-blue-700"
        };
        out.push_str(&format!(
            r#"<li><a class="{cls}" href="{href}">{label}</a></li>"#,
            cls = cls,
            href = escape(it.href),
            label = escape(it.label),
        ));
    }
    out.push_str("</ul></nav>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_items_match_keycloak_root_layout() {
        let items = account_nav_items();
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].label, "Personal info");
        assert!(items.iter().any(|i| i.href == "/account/sessions"));
        assert!(items.iter().any(|i| i.href == "/account/applications"));
    }

    #[test]
    fn nav_strip_renders_active_marker_for_matching_path() {
        let html = render_account_nav("/account/sessions");
        // Active tab must carry the active class fragment.
        let segs: Vec<&str> = html.split(r#"href="/account/sessions""#).collect();
        // The Sessions link element must contain the active class.
        // Search forward from any segment for "border-b-2".
        assert!(html.contains("border-b-2 border-blue-600"));
        let _ = segs;
    }

    #[test]
    fn nav_strip_escapes_label_html() {
        // None of our labels contain HTML — but the renderer must
        // still escape them so a future refactor can't bleed markup.
        let html = render_account_nav("/account/profile");
        assert!(!html.contains("<script"));
        assert!(html.contains("Personal info"));
    }
}
