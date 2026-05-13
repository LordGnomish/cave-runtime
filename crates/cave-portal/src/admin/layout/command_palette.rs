//! Cmd+K / Ctrl+K command palette.
//!
//! Server-rendered modal + inline JS that:
//!
//!   * Opens the dialog on `Cmd+K` (Mac) or `Ctrl+K` (Linux/Win).
//!   * Closes on `Esc`, click outside, or item selection.
//!   * Fuzzy-matches the input against the command list (case-
//!     insensitive substring; subsequence match as a fallback).
//!   * Arrow keys move the highlight; `Enter` navigates.
//!
//! No JS framework — about 100 lines of vanilla JS. Items are
//! injected as JSON via `<script type="application/json">` so the
//! HTML escaping route is preserved.

use crate::admin::render::escape;
use serde::Serialize;

/// One command in the palette.
#[derive(Debug, Clone, Serialize)]
pub struct CommandItem {
    pub label: String,
    pub href: String,
    /// Hint / group ("Navigate", "Action", "Toggle"). Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl CommandItem {
    pub fn nav(label: &str, href: &str) -> Self {
        Self {
            label: label.into(),
            href: href.into(),
            hint: Some("Navigate".into()),
        }
    }

    pub fn action(label: &str, href: &str) -> Self {
        Self {
            label: label.into(),
            href: href.into(),
            hint: Some("Action".into()),
        }
    }
}

/// Render the modal markup + JS. Hidden by default (`hidden`
/// attribute toggled by the open() function).
pub fn command_palette_modal(items: &[CommandItem]) -> String {
    let items_json = serde_json::to_string(items).unwrap_or_else(|_| "[]".into());
    format!(
        r##"<!-- Command palette (Cmd+K) -->
<div id="cave-cmdk" role="dialog" aria-modal="true" aria-label="Command palette" hidden
     class="fixed inset-0 z-50 flex items-start justify-center pt-24 bg-black/40">
  <div class="bg-white dark:bg-zinc-900 dark:text-zinc-100 w-full max-w-xl rounded shadow-lg overflow-hidden">
    <div class="border-b dark:border-zinc-800 px-3 py-2">
      <input id="cave-cmdk-input" type="text" autocomplete="off" spellcheck="false"
             placeholder="Type a command or search…"
             aria-label="Command search"
             class="w-full bg-transparent outline-none text-sm py-1">
    </div>
    <ul id="cave-cmdk-list" role="listbox" aria-label="Command results" class="max-h-72 overflow-y-auto text-sm"></ul>
    <div class="border-t dark:border-zinc-800 px-3 py-1.5 text-[10px] uppercase tracking-wider text-zinc-500 flex justify-between">
      <span>↑↓ navigate · ↵ open · esc close</span>
      <span>cmd/ctrl + k</span>
    </div>
  </div>
</div>
<script type="application/json" id="cave-cmdk-data">{items_json}</script>
<script>
(function() {{
  var modal = document.getElementById('cave-cmdk');
  var input = document.getElementById('cave-cmdk-input');
  var list = document.getElementById('cave-cmdk-list');
  var data = JSON.parse(document.getElementById('cave-cmdk-data').textContent || '[]');
  var hi = 0;

  function escapeHtml(s) {{
    return String(s).replace(/[&<>"']/g, function(c) {{
      return {{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}}[c];
    }});
  }}
  function match(item, q) {{
    if (!q) return true;
    var hay = (item.label + ' ' + (item.hint||'')).toLowerCase();
    var needle = q.toLowerCase();
    if (hay.indexOf(needle) !== -1) return true;
    // Fallback: subsequence — characters of needle appear in order.
    var i = 0, j = 0;
    while (i < needle.length && j < hay.length) {{
      if (needle.charAt(i) === hay.charAt(j)) i++;
      j++;
    }}
    return i === needle.length;
  }}
  function render() {{
    var q = input.value;
    var matches = data.filter(function(i) {{ return match(i, q); }});
    list.innerHTML = matches.map(function(it, idx) {{
      var cls = (idx === hi) ? 'bg-blue-100 dark:bg-blue-900/40' : '';
      return '<li role="option" data-href="' + escapeHtml(it.href) + '" '
        + 'class="px-3 py-1.5 cursor-pointer flex items-center justify-between ' + cls + '">'
        + '<span>' + escapeHtml(it.label) + '</span>'
        + '<span class="text-[10px] text-zinc-500 uppercase tracking-wider">'
        + escapeHtml(it.hint || '') + '</span></li>';
    }}).join('');
    list._matches = matches;
  }}
  function open() {{
    modal.hidden = false;
    input.value = '';
    hi = 0;
    render();
    input.focus();
  }}
  function close() {{ modal.hidden = true; }}
  function commit() {{
    var m = list._matches || [];
    if (hi < m.length && m[hi]) window.location.href = m[hi].href;
  }}
  document.addEventListener('keydown', function(e) {{
    var mod = e.metaKey || e.ctrlKey;
    if (mod && (e.key === 'k' || e.key === 'K')) {{
      e.preventDefault();
      modal.hidden ? open() : close();
      return;
    }}
    if (modal.hidden) return;
    if (e.key === 'Escape') {{ close(); return; }}
    if (e.key === 'ArrowDown') {{ hi = Math.min(hi + 1, (list._matches||[]).length - 1); render(); e.preventDefault(); }}
    if (e.key === 'ArrowUp') {{ hi = Math.max(hi - 1, 0); render(); e.preventDefault(); }}
    if (e.key === 'Enter') {{ commit(); e.preventDefault(); }}
  }});
  input.addEventListener('input', function() {{ hi = 0; render(); }});
  list.addEventListener('click', function(e) {{
    var li = e.target.closest('li');
    if (!li) return;
    window.location.href = li.getAttribute('data-href');
  }});
  modal.addEventListener('click', function(e) {{
    if (e.target === modal) close();
  }});
  // Expose for external triggers (e.g. nav button).
  window.caveCmdk = {{ open: open, close: close }};
}})();
</script>
<!-- /command palette -->"##,
        items_json = escape(&items_json),
    )
}

/// Default command set — useful out-of-the-box. Callers can append
/// per-tenant items or replace entirely.
pub fn default_commands(tenant_id: &str) -> Vec<CommandItem> {
    let t = tenant_id;
    vec![
        CommandItem::nav("Go to Compliance",     &format!("/admin/compliance?tenant_id={t}")),
        CommandItem::nav("Go to Upstream",       &format!("/admin/upstream?tenant_id={t}")),
        CommandItem::nav("Go to ADR Browser",    &format!("/admin/adr?tenant_id={t}")),
        CommandItem::nav("Go to KEDA",           &format!("/admin/keda?tenant_id={t}")),
        CommandItem::nav("Go to Vault",          &format!("/admin/vault?tenant_id={t}")),
        CommandItem::nav("Go to Kubelet",        &format!("/admin/kubelet?tenant_id={t}")),
        CommandItem::nav("Go to Scheduler",      &format!("/admin/scheduler?tenant_id={t}")),
        CommandItem::nav("Go to API Server",     &format!("/admin/apiserver?tenant_id={t}")),
        CommandItem::nav("Go to etcd",           &format!("/admin/etcd?tenant_id={t}")),
        CommandItem::nav("Go to Networking",     &format!("/admin/net?tenant_id={t}")),
        CommandItem::action("Toggle dark mode",  "/api/portal/theme/toggle"),
        CommandItem::action("Sign out",          "/api/auth/logout"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_contains_aria_attributes_and_input() {
        let items = vec![CommandItem::nav("Home", "/")];
        let html = command_palette_modal(&items);
        assert!(html.contains(r#"role="dialog""#));
        assert!(html.contains(r#"aria-modal="true""#));
        assert!(html.contains(r#"aria-label="Command palette""#));
        assert!(html.contains(r#"id="cave-cmdk-input""#));
        assert!(html.contains("hidden"));
    }

    #[test]
    fn modal_embeds_items_as_json() {
        let items = vec![CommandItem::nav("KEDA", "/admin/keda")];
        let html = command_palette_modal(&items);
        // Items embedded as JSON inside the data script tag.
        assert!(html.contains(r#"id="cave-cmdk-data""#));
        assert!(html.contains("KEDA"));
        assert!(html.contains("/admin/keda"));
    }

    #[test]
    fn modal_keyboard_shortcut_targets_metaKey_or_ctrlKey() {
        let html = command_palette_modal(&[]);
        // We bind on metaKey || ctrlKey + 'k'.
        assert!(html.contains("metaKey"));
        assert!(html.contains("ctrlKey"));
    }

    #[test]
    fn default_commands_propagates_tenant_id() {
        let cs = default_commands("acme");
        for c in &cs {
            if c.href.contains("/admin/") {
                assert!(c.href.contains("tenant_id=acme"), "missing tenant_id in {}", c.href);
            }
        }
    }

    #[test]
    fn default_commands_includes_toggle_dark_mode_and_logout() {
        let cs = default_commands("x");
        assert!(cs.iter().any(|c| c.label.contains("dark mode")));
        assert!(cs.iter().any(|c| c.label.contains("Sign out")));
    }

    #[test]
    fn command_item_serializes_without_optional_hint_when_none() {
        let c = CommandItem { label: "x".into(), href: "/".into(), hint: None };
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("hint"));
    }
}
