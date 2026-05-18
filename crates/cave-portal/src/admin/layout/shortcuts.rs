// SPDX-License-Identifier: AGPL-3.0-or-later
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

use crate::admin::permission::Persona;
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
    /// Minimum persona that may use this shortcut. Bindings whose
    /// persona is above the caller's are *visible* in the help modal
    /// but greyed-out, and pressing them fires a toast instead of
    /// navigating — same UX pattern as a disabled menu item.
    ///
    /// Defaults to `Persona::Anonymous` so `?`, `cmd k`, `j`, `k`,
    /// `enter`, `esc`, `/` (purely page-local bindings) work for
    /// every caller including the dev `?tenant_id=...` shortcut.
    pub min_persona: Persona,
}

pub const DEFAULT_BINDINGS: &[ShortcutBinding] = &[
    ShortcutBinding { keys: "?",     description: "Show this help",                       href: None,                                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "cmd k", description: "Open command palette",                 href: None,                                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "g h",   description: "Go to home",                           href: Some("/"),                                         min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "g c",   description: "Go to Compliance",                     href: Some("/admin/compliance"),                         min_persona: Persona::PlatformAdmin },
    // 2026-05-14 discoverability fix — Cluster Status had no leader-
    // key shortcut despite being the canonical Raft live view.
    ShortcutBinding { keys: "g l",   description: "Go to Cluster Status (live)",          href: Some("/admin/cluster/live"),                       min_persona: Persona::PlatformAdmin },
    ShortcutBinding { keys: "g k",   description: "Go to KEDA",                           href: Some("/admin/keda"),                               min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "g v",   description: "Go to Vault",                          href: Some("/admin/vault"),                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "g u",   description: "Go to Upstream",                       href: Some("/admin/upstream"),                           min_persona: Persona::PlatformAdmin },
    ShortcutBinding { keys: "g a",   description: "Go to ADR Browser",                    href: Some("/admin/adr"),                                min_persona: Persona::PlatformAdmin },
    // 2026-05-15 polish: /admin/_audit roll-up.
    ShortcutBinding { keys: "g _",   description: "Go to Audit roll-up",                  href: Some("/admin/_audit"),                             min_persona: Persona::PlatformAdmin },
    // 2026-05-14 consolidation: Scheduler folded into K8s Dashboard.
    ShortcutBinding { keys: "g s",   description: "Go to K8s · Scheduler Queue",          href: Some("/admin/k8s-dashboard/scheduler/queue"),      min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "/",     description: "Focus search (where present)",         href: None,                                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "j",     description: "Next row (list pages)",                href: None,                                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "k",     description: "Previous row (list pages)",            href: None,                                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "enter", description: "Open highlighted row",                 href: None,                                              min_persona: Persona::Anonymous },
    ShortcutBinding { keys: "esc",   description: "Close modal / cancel",                 href: None,                                              min_persona: Persona::Anonymous },
];

impl ShortcutBinding {
    /// True iff a caller of `persona` is allowed to invoke this
    /// shortcut. Disabled rows still appear in the help modal, but
    /// pressing them only fires a toast.
    pub fn enabled_for(&self, persona: Persona) -> bool {
        persona.can_access(self.min_persona)
    }
}

/// Render the help-modal HTML + the global keydown handler script.
/// `tenant_id` is appended as a query param to the leader-key
/// navigation hrefs so persona-scoped routes resolve correctly.
///
/// `persona` filters the leader-key map: bindings whose `min_persona`
/// is not satisfied are listed in a separate "denied" map so the JS
/// can fire a toast ("Shortcut requires Platform Admin") instead of
/// silently ignoring the press. The help modal still lists every
/// binding so the user can discover what's available — disabled rows
/// get a `data-disabled` attribute + a "Platform" badge.
pub fn shortcuts_help_modal(
    bindings: &[ShortcutBinding],
    persona: Persona,
    tenant_id: &str,
) -> String {
    // Build the table rows. Disabled rows get a visible badge.
    let mut rows = String::new();
    for b in bindings {
        let enabled = b.enabled_for(persona);
        let row_attr = if enabled { "" } else { r#" data-disabled="true""# };
        let row_cls = if enabled {
            "border-t dark:border-zinc-800"
        } else {
            "border-t dark:border-zinc-800 opacity-60"
        };
        let badge = if enabled {
            String::new()
        } else {
            r#" <span class="ml-2 px-1.5 py-0.5 rounded text-[10px] uppercase tracking-wider bg-amber-200 text-amber-900 dark:bg-amber-900/40 dark:text-amber-100" title="Requires Platform Admin">Platform</span>"#.to_string()
        };
        rows.push_str(&format!(
            r#"<tr class="{cls}"{attr}>
                <td class="px-3 py-1.5"><kbd class="px-1.5 py-0.5 rounded border bg-zinc-100 dark:bg-zinc-800 dark:border-zinc-700 text-xs">{keys}</kbd></td>
                <td class="px-3 py-1.5">{desc}{badge}</td>
              </tr>"#,
            cls = row_cls,
            attr = row_attr,
            keys = escape(b.keys),
            desc = escape(b.description),
            badge = badge,
        ));
    }

    // Build TWO maps: enabled "g <key>" → href (navigates) and
    // denied "g <key>" → description (fires toast). The handler
    // checks `g_denied_map[key]` first so the user gets feedback.
    let mut map_entries: Vec<String> = Vec::new();
    let mut denied_entries: Vec<String> = Vec::new();
    for b in bindings {
        if let Some(href) = b.href {
            let parts: Vec<&str> = b.keys.split_whitespace().collect();
            if parts.len() == 2 && parts[0] == "g" {
                let key = parts[1].replace('"', "\\\"");
                if b.enabled_for(persona) {
                    let href_with_q = if href.starts_with("/admin/") || href == "/" {
                        format!("{}?tenant_id={}", href, urlencode_minimal(tenant_id))
                    } else {
                        href.to_string()
                    };
                    map_entries.push(format!(
                        "\"{}\": \"{}\"",
                        key,
                        href_with_q.replace('"', "\\\""),
                    ));
                } else {
                    denied_entries.push(format!(
                        "\"{}\": \"{}\"",
                        key,
                        b.description.replace('"', "\\\""),
                    ));
                }
            }
        }
    }
    let g_map = format!("{{ {} }}", map_entries.join(", "));
    let g_denied_map = format!("{{ {} }}", denied_entries.join(", "));

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
  var gDeniedMap = {g_denied_map};
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
      if (target) {{ e.preventDefault(); window.location.href = target; return; }}
      // Persona-disabled binding — show toast instead of silent ignore.
      var deniedDesc = gDeniedMap[e.key];
      if (deniedDesc && window.caveToast) {{
        e.preventDefault();
        window.caveToast('warning', deniedDesc + ' — requires Platform Admin');
      }}
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
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "acme");
        for b in DEFAULT_BINDINGS {
            assert!(html.contains(b.description), "missing: {}", b.description);
        }
    }

    #[test]
    fn help_modal_has_dialog_role_and_aria_label() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "acme");
        assert!(html.contains(r#"role="dialog""#));
        assert!(html.contains(r#"aria-modal="true""#));
        assert!(html.contains(r#"aria-label="Keyboard shortcuts""#));
    }

    #[test]
    fn help_modal_kbd_elements_use_kbd_tag() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "acme");
        assert!(html.contains("<kbd"));
    }

    #[test]
    fn leader_g_map_embeds_tenant_id_in_admin_routes() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "tenant1");
        // "g k" → /admin/keda?tenant_id=tenant1
        assert!(html.contains(r#""k": "/admin/keda?tenant_id=tenant1""#));
    }

    #[test]
    fn typing_target_check_excludes_inputs_in_js() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "x");
        // The JS bails out if the keydown target is an INPUT/TEXTAREA/SELECT.
        assert!(html.contains("INPUT"));
        assert!(html.contains("TEXTAREA"));
    }

    #[test]
    fn question_mark_keydown_handler_emits_preventDefault() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "x");
        assert!(html.contains("e.key === '?'"));
    }

    #[test]
    fn list_navigation_js_uses_data_list_row_attribute() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "x");
        // j/k navigation should use [data-list-row].
        assert!(html.contains("data-list-row"));
    }

    // ── 2026-05-15 polish sweep — persona filter ─────────────────────

    #[test]
    fn enabled_for_blocks_platform_only_bindings_for_tenant_admin() {
        let g_a = DEFAULT_BINDINGS.iter().find(|b| b.keys == "g a").unwrap();
        assert!(g_a.enabled_for(Persona::PlatformAdmin));
        assert!(!g_a.enabled_for(Persona::TenantAdmin));
        assert!(!g_a.enabled_for(Persona::Anonymous));

        let g_k = DEFAULT_BINDINGS.iter().find(|b| b.keys == "g k").unwrap();
        // KEDA is tenant-scoped — anyone can use it.
        assert!(g_k.enabled_for(Persona::Anonymous));
        assert!(g_k.enabled_for(Persona::TenantAdmin));
        assert!(g_k.enabled_for(Persona::PlatformAdmin));
    }

    #[test]
    fn tenant_admin_g_map_omits_platform_only_bindings() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::TenantAdmin, "tenant1");
        // KEDA stays in the enabled map.
        assert!(html.contains(r#""k": "/admin/keda?tenant_id=tenant1""#));
        // ADR (g a), Compliance (g c), Upstream (g u), Cluster (g l)
        // should NOT appear in the enabled gMap — they go into the
        // denied map instead.
        assert!(!html.contains(r#""a": "/admin/adr"#));
        assert!(!html.contains(r#""c": "/admin/compliance"#));
        assert!(!html.contains(r#""u": "/admin/upstream"#));
        assert!(!html.contains(r#""l": "/admin/cluster/live"#));
    }

    #[test]
    fn tenant_admin_help_modal_lists_disabled_rows_with_platform_badge() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::TenantAdmin, "tenant1");
        // Disabled rows still appear (so the user can discover them).
        assert!(html.contains("Go to ADR Browser"));
        // …but they get the Platform badge + data-disabled marker.
        assert!(html.contains(r#"data-disabled="true""#));
        assert!(html.contains(">Platform</span>"));
    }

    #[test]
    fn platform_admin_help_modal_has_no_disabled_rows() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "acme");
        assert!(!html.contains(r#"data-disabled="true""#));
        assert!(!html.contains(">Platform</span>"));
    }

    #[test]
    fn denied_keys_route_through_caveToast_in_js() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::TenantAdmin, "tenant1");
        // The handler must reference the denied map and call caveToast.
        assert!(html.contains("gDeniedMap"));
        assert!(html.contains("window.caveToast"));
        assert!(html.contains("requires Platform Admin"));
    }

    #[test]
    fn denied_map_carries_descriptions_for_blocked_bindings() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::TenantAdmin, "tenant1");
        // Blocked g-key entries land in gDeniedMap with their description
        // as the value (so the toast can say what the user wanted).
        assert!(html.contains(r#""a": "Go to ADR Browser""#));
        assert!(html.contains(r#""c": "Go to Compliance""#));
        assert!(html.contains(r#""u": "Go to Upstream""#));
    }

    #[test]
    fn platform_admin_denied_map_is_empty() {
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::PlatformAdmin, "acme");
        // Empty {} when there's nothing to deny.
        assert!(html.contains("var gDeniedMap = {  };") || html.contains("var gDeniedMap = { };"));
    }

    #[test]
    fn anonymous_persona_treated_like_tenant_admin_for_platform_routes() {
        // The dev `?tenant_id=...` shortcut without a JWT cookie
        // resolves to Anonymous, which is below TenantAdmin in the
        // capability lattice — must still NOT see ADR / Compliance.
        let html = shortcuts_help_modal(DEFAULT_BINDINGS, Persona::Anonymous, "tenant1");
        assert!(!html.contains(r#""a": "/admin/adr"#));
        assert!(!html.contains(r#""c": "/admin/compliance"#));
        // …but tenant-scoped routes (KEDA) are still accessible.
        assert!(html.contains(r#""k": "/admin/keda?tenant_id=tenant1""#));
    }
}
