// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Inline help — tooltips + empty-state CTAs.
//!
//! Tooltips wrap a label with a native `<span title="...">` so they
//! work without JS (screen readers read the title attribute; visual
//! browsers show it on hover). Empty states render a centered
//! card with an icon, a friendly message, and a single primary CTA.

use crate::admin::render::escape;

/// Wrap `text` with a tooltip. Accessible: the tooltip text lives
/// in `aria-label` so screen readers announce it, AND in `title`
/// for the visual hover. We add a subtle dotted underline so
/// sighted users know it's interactive.
pub fn tooltip(text: &str, help: &str) -> String {
    format!(
        r#"<span class="cursor-help underline decoration-dotted decoration-zinc-400" title="{help}" aria-label="{help}">{text}</span>"#,
        help = escape(help),
        text = escape(text),
    )
}

/// Render an empty-state panel. `icon` is a single emoji or short
/// glyph; `message` is a one-liner; `cta_label` + `cta_href` are
/// the "Create your first X" button.
pub fn empty_state(icon: &str, message: &str, cta_label: &str, cta_href: &str) -> String {
    format!(
        r#"<div class="border border-dashed dark:border-zinc-700 rounded p-8 text-center text-zinc-600 dark:text-zinc-300">
  <div class="text-4xl mb-3" aria-hidden="true">{icon}</div>
  <p class="mb-4">{message}</p>
  <a href="{href}" class="inline-block px-4 py-2 rounded bg-blue-600 hover:bg-blue-700 text-white text-sm focus:outline-none focus:ring-2 focus:ring-blue-400">{cta}</a>
</div>"#,
        icon = escape(icon),
        message = escape(message),
        href = escape(cta_href),
        cta = escape(cta_label),
    )
}

/// Render a small inline-hint paragraph below a form field or
/// section heading. Used to explain what `parity_ratio` means, what
/// a `tolerationSeconds` does, etc.
pub fn hint(text: &str) -> String {
    format!(
        r#"<p class="text-xs text-zinc-500 dark:text-zinc-400 mt-1">{}</p>"#,
        escape(text),
    )
}

/// Render a header with a "Learn more" link pointing into the
/// charter docs. The link opens in a new tab so the user doesn't
/// lose their dashboard context.
pub fn header_with_help(title: &str, doc_anchor: &str) -> String {
    format!(
        r#"<div class="flex items-center justify-between mb-2">
  <h2 class="text-lg font-semibold">{title}</h2>
  <a href="/docs/charter#{anchor}" target="_blank" rel="noopener" class="text-xs text-blue-600 dark:text-blue-300 hover:underline">Learn more ↗</a>
</div>"#,
        title = escape(title),
        anchor = escape(doc_anchor),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_emits_title_and_aria_label_attributes() {
        let html = tooltip("parity_ratio", "(mapped + skipped) / total");
        assert!(html.contains(r#"title="(mapped + skipped) / total""#));
        assert!(html.contains(r#"aria-label="(mapped + skipped) / total""#));
        assert!(html.contains("parity_ratio"));
    }

    #[test]
    fn tooltip_escapes_xss_in_help_text() {
        let html = tooltip("x", r#"<script>alert(1)</script>"#);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn empty_state_renders_icon_message_cta() {
        let html = empty_state("📦", "No ScaledObjects yet", "Create your first", "/admin/keda/new");
        assert!(html.contains("📦"));
        assert!(html.contains("No ScaledObjects yet"));
        assert!(html.contains(r#"href="/admin/keda/new""#));
        assert!(html.contains("Create your first"));
    }

    #[test]
    fn empty_state_button_has_focus_ring_for_keyboard_a11y() {
        let html = empty_state("🪜", "x", "Add", "/x");
        assert!(html.contains("focus:ring-2"));
    }

    #[test]
    fn hint_renders_as_small_paragraph_with_dark_variant() {
        let html = hint("This number measures upstream parity.");
        assert!(html.contains("<p"));
        assert!(html.contains("text-zinc-500"));
        assert!(html.contains("dark:text-zinc-400"));
    }

    #[test]
    fn header_with_help_links_to_charter_anchor() {
        let html = header_with_help("Compliance", "compliance");
        assert!(html.contains(r#"href="/docs/charter#compliance""#));
        assert!(html.contains(r#"target="_blank""#));
        assert!(html.contains(r#"rel="noopener""#));
    }
}
