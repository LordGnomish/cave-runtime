// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Toast notification container + helper to emit toasts from
//! the server via HX-Trigger response headers.
//!
//! Server-side: handlers add `HX-Trigger: cave-toast={"kind":"success","msg":"..."}`
//! to a response, and the inline JS in the toast container picks it
//! up + renders a transient bubble.

/// Render the toast container — empty `<ol>` that the JS handler
/// populates. Sticky bottom-right; auto-dismiss at 4 s.
pub fn toast_container() -> String {
    r##"<!-- Toast container -->
<ol id="cave-toasts" aria-live="polite" aria-relevant="additions"
    class="fixed bottom-4 right-4 z-30 flex flex-col gap-2 w-72"></ol>
<script>
(function() {
  var box = document.getElementById('cave-toasts');
  function show(kind, msg) {
    var li = document.createElement('li');
    var cls = {
      success: 'bg-green-50 dark:bg-green-900/40 text-green-900 dark:text-green-100 border-green-300 dark:border-green-800',
      error:   'bg-red-50 dark:bg-red-900/40 text-red-900 dark:text-red-100 border-red-300 dark:border-red-800',
      warning: 'bg-yellow-50 dark:bg-yellow-900/40 text-yellow-900 dark:text-yellow-100 border-yellow-300 dark:border-yellow-800',
      info:    'bg-blue-50 dark:bg-blue-900/40 text-blue-900 dark:text-blue-100 border-blue-300 dark:border-blue-800'
    }[kind] || 'bg-zinc-50 dark:bg-zinc-800 text-zinc-900 dark:text-zinc-100 border-zinc-300 dark:border-zinc-700';
    li.className = 'rounded border px-3 py-2 text-sm shadow ' + cls;
    li.setAttribute('role', kind === 'error' ? 'alert' : 'status');
    li.textContent = msg;
    box.appendChild(li);
    setTimeout(function() {
      li.style.opacity = '0';
      li.style.transition = 'opacity 200ms';
      setTimeout(function() { li.remove(); }, 250);
    }, 4000);
  }
  // Listen to htmx triggers — server response headers like:
  //   HX-Trigger: cave-toast={"kind":"success","msg":"Saved"}
  // bubble up here as a CustomEvent.
  document.body.addEventListener('cave-toast', function(e) {
    var d = e.detail || {};
    show(d.kind || 'info', d.msg || '');
  });
  // Imperative API for inline scripts.
  window.caveToast = show;
})();
</script>
<!-- /Toast container -->"##
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_has_aria_live_polite_for_screen_readers() {
        let html = toast_container();
        assert!(html.contains(r#"aria-live="polite""#));
        assert!(html.contains(r#"id="cave-toasts""#));
    }

    #[test]
    fn js_distinguishes_error_with_role_alert() {
        // Error toasts get `role=alert`; everything else gets `role=status`.
        let html = toast_container();
        assert!(html.contains("'alert'"));
        assert!(html.contains("'status'"));
    }

    #[test]
    fn js_listens_for_htmx_trigger_event() {
        let html = toast_container();
        assert!(html.contains("'cave-toast'"));
        assert!(html.contains("addEventListener"));
    }

    #[test]
    fn auto_dismiss_at_four_seconds() {
        let html = toast_container();
        assert!(html.contains("4000"));
    }

    #[test]
    fn dark_mode_variant_classes_present_for_every_kind() {
        let html = toast_container();
        for kind in &["green", "red", "yellow", "blue"] {
            assert!(
                html.contains(&format!("dark:bg-{}-900", kind)),
                "missing dark variant for {kind}"
            );
        }
    }

    #[test]
    fn caveToast_global_is_exposed_for_inline_scripts() {
        let html = toast_container();
        assert!(html.contains("window.caveToast"));
    }
}
