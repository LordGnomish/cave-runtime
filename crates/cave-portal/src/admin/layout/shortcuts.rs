// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keyboard shortcuts + the `?` help modal.
//!
//! `cave-runtime` doesn't yet ship a SPA, so the shortcuts live on
//! the same inline JS pattern as the command palette: a single
//! global `keydown` listener that watches for `?`, then leader-key
//! sequences (`g h`, `g k`, …), then the per-page Vim-style
//! navigation (`j` / `k` / `Enter` / `/`).
//!
//! The renderer emits both the help-modal HTML and the JS handler.
//! Callers include this once near the bottom of `<body>`.

use crate::admin::render::escape;
use serde::Serialize;

/// One row in the help modal.
#[derive(Debug, Clone, Serialize)]
pub struct ShortcutBinding {
    pub keys: &'static str,
    pub description: &'static str,
    /// Optional href to navigate to. `None` means the binding is
    /// page-local (j/k list navigation, `/` focus search, …) and
    /// the help modal just documents it.
    pub href: Option<&'static str>,
}

pub const DEFAULT_BINDINGS: &[ShortcutBinding] = &[
    ShortcutBinding { keys: "?",     description: "Show this help",                       href: None },
    ShortcutBinding { keys: "cmd k", description: "Open command palette",                 href: None },
    ShortcutBinding { keys: "g h",   description: "Go to home",                           href: Some("/") },
    ShortcutBinding { keys: "g c",   description: "Go to Compliance",                     href: Some("/admin/compliance") },
    // 2026-05-14 discoverability fix — Cluster Status had no leader-
    // key shortcut despite being the canonical Raft live view.
    ShortcutBinding { keys: "g l",   description: "Go to Cluster Status (live)",          href: Some("/admin/cluster/live") },
    ShortcutBinding { keys: "g k",   description: "Go to KEDA",                           href: Some("/admin/keda") },
    ShortcutBinding { keys: "g v",   description: "Go to Vault",                          href: Some("/admin/vault") },
    ShortcutBinding { keys: "g u",   description: "Go to Upstream",                       href: Some("/admin/upstream") },
    ShortcutBinding { keys: "g a",   description: "Go to ADR Browser",                    href: Some("/admin/adr") },
    // 2026-05-14 consolidation: Scheduler folded into K8s Dashboard.
    ShortcutBinding { keys: "g s",   description: "Go to K8s · Scheduler Queue",          href: Some("/admin/k8s-dashboard/scheduler/queue") },
    ShortcutBinding { keys: "/",     description: "Focus search (where present)",         href: None },
    ShortcutBinding { keys: "j",     description: "Next row (list pages)",                href: None },
    ShortcutBinding { keys: "k",     description: "Previous row (list pages)",            href: None },
    ShortcutBinding { keys: "enter", description: "Open highlighted row",                 href: None },
    ShortcutBinding { keys: "esc",   description: "Close modal / cancel",                 href: None },
];

/// Render the help-modal HTML + the global keydown handler script.
/// `tenant_id` is appended as a query param to the leader-key
/// navigation hrefs so persona-scoped routes resolve correctly.
pub fn shortcuts_help_modal(bindings: &[ShortcutBinding], tenant_id: &str) -> String {
    // Build the table rows.
    let mut rows = String::new();
    for b in bindings {
        rows.push_str(&format!(
            r#"<tr class="border-t dark:border-zinc-800">
                <td class="px-3 py-1.5"><kbd class="px-1.5 py-0.5 rounded border bg-zinc-100 dark:bg-zinc-800 dark:border-zinc-700 text-xs">{keys}</kbd></td>
                <td class="px-3 py-1.5">{desc}</td>
              </tr>"#,
            keys = escape(b.keys),
            desc = escape(b.description),
        ));
    }

    // Build the leader-key → href map as JS object for the handler.
    let mut map_entries: Vec<String> = Vec::new();
    for b in bindings {
        if let Some(href) = b.href {
            // Leader-key bindings are always `<leader> <key>`.
            // We split at the space and key on the second char.
            let parts: Vec<&str> = b.keys.split_whitespace().collect();
            if parts.len() == 2 && parts[0] == "g" {
                // Append tenant_id query so admin routes resolve.
                let href_with_q = if href.starts_with("/admin/") || href == "/" {
                    format!("{}?tenant_id={}", href, urlencode_minimal(tenant_id))
                } else {
                    href.to_string()
                };
                map_entries.push(format!(
                    "\"{}\": \"{}\"",
                    parts[1].replace('"', "\\\""),
                    href_with_q.replace('"', "\\\""),
                ));
            }
        }
    }
    let g_map = format!("{{ {} }}", map_entries.join(", "));

    format!(
        r##"<!-- Keyboard shortcuts -->
<div id="cave-help" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts" hidden
     class="fixed inset-0 z-40 flex items-center justify-center bg-black/40">
  <div class="bg-white dark:bg-zinc-900 dark:text-zinc-100 w-full max-w-md rounded shadow-lg">
    <div class="border-b dark:border-zinc-800 px-3 py-2 flex items-center justify-between">
      <h2 class="text-sm font-semibold">Keyboard shortcuts</h2>
      <button id="cave-help-close" aria-label="Close" class="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">×</button>
    </div>
    <div class="max-h-[60vh] overflow-y-auto">
      <table class="w-full text-sm"><tbody>{rows}</tbody></table>
    </div>
  </div>
</div>
<script>
(function() {{
  var modal = document.getElementById('cave-help');
  var close = document.getElementById('cave-help-close');
  var gMap = {g_map};
  var leader = null;
  var leaderTimer = null;

  function open() {{ modal.hidden = false; }}
  function shut() {{ modal.hidden = true; }}

  function isTypingTarget(t) {{
    if (!t) return false;
    var tag = (t.tagName || '').toUpperCase();
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || t.isContentEditable;
  }}

  document.addEventListener('keydown', function(e) {{
    if (isTypingTarget(e.target)) return;
    // ? → help
    if (e.key === '?' && !e.metaKey && !e.ctrlKey && !e.altKey) {{
      e.preventDefault(); open(); return;
    }}
    // esc → close any open modal
    if (e.key === 'Escape') {{ shut(); return; }}
    // g leader: 'g' arms; next key triggers nav (within 1s).
    if (leader === 'g') {{
      clearTimeout(leaderTimer);
      leader = null;
      var target = gMap[e.key];
      if (target) {{ e.preventDefault(); window.location.href = target; }}
      return;
    }}
    if (e.key === 'g' && !e.metaKey && !e.ctrlKey) {{
      leader = 'g';
      clearTimeout(leaderTimer);
      leaderTimer = setTimeout(function() {{ leader = null; }}, 1000);
      return;
    }}
    // / → focus search (looks for [data-search-focus])
    if (e.key === '/' && !e.metaKey && !e.ctrlKey) {{
      var inp = document.querySelector('[data-search-focus]');
      if (inp) {{ e.preventDefault(); inp.focus(); }}
    }}
    // j / k — list navigation (looks for [data-list-row])
    if (e.key === 'j' || e.key === 'k') {{
      var rows = document.querySelectorAll('[data-list-row]');
      if (!rows.length) return;
      var cur = document.querySelector('[data-list-row][data-active]');
      var idx = -1;
      rows.forEach(function(r, i) {{ if (r === cur) idx = i; }});
      if (cur) cur.removeAttribute('data-active');
      idx = (e.key === 'j') ? Math.min(idx + 1, rows.length - 1) : Math.max(idx - 1, 0);
      if (idx < 0) idx = 0;
      rows[idx].setAttribute('data-active', '');
      rows[idx].scrollIntoView({{ block: 'nearest' }});
    }}
    // enter — open active row's link (first <a> inside).
    if (e.key === 'Enter' && !e.metaKey && !e.ctrlKey) {{
      var row = document.querySelector('[data-list-row][data-active]');
      if (row) {{
        var a = row.querySelector('a[href]');
        if (a) {{ e.preventDefault(); window.location.href = a.href; }}
      }}
    }}
  }});
  close.addEventListener('click', shut);
  modal.addEventListener('click', function(e) {{ if (e.target === modal) shut(); }});
}})();
</script>
<!-- /Keyboard shortcuts -->"##,
        rows = rows,
        g_map = g_map,
    )
}

fn urlencode_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_modal_renders_every_binding_row() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "acme");
        for b in DEFAULT_BINDINGS {
            assert!(html.contains(b.description), "missing: {}", b.description);
        }
    }

    #[test]
    fn help_modal_has_dialog_role_and_aria_label() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "acme");
        assert!(html.contains(r#"role="dialog""#));
        assert!(html.contains(r#"aria-modal="true""#));
        assert!(html.contains(r#"aria-label="Keyboard shortcuts""#));
    }

    #[test]
    fn help_modal_kbd_elements_use_kbd_tag() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "acme");
        assert!(html.contains("<kbd"));
    }

    #[test]
    fn leader_g_map_embeds_tenant_id_in_admin_routes() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "tenant1");
        // "g k" → /admin/keda?tenant_id=tenant1
        assert!(html.contains(r#""k": "/admin/keda?tenant_id=tenant1""#));
    }

    #[test]
    fn typing_target_check_excludes_inputs_in_js() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "x");
        // The JS bails out if the keydown target is an INPUT/TEXTAREA/SELECT.
        assert!(html.contains("INPUT"));
        assert!(html.contains("TEXTAREA"));
    }

    #[test]
    fn question_mark_keydown_handler_emits_preventDefault() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "x");
        assert!(html.contains("e.key === '?'"));
    }

    #[test]
    fn list_navigation_js_uses_data_list_row_attribute() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, "x");
        // j/k navigation should use [data-list-row].
        assert!(html.contains("data-list-row"));
    }
}
