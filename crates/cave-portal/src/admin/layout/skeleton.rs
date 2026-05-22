// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Loading skeletons + error boundary panel.
//!
//! Skeleton rows are content-shaped placeholders that show the
//! table while the data fetches — way nicer than a centered
//! spinner because the layout doesn't jump when the real rows
//! arrive.

use crate::admin::render::escape;

/// Render a skeleton table: N rows × C columns with shimmering
/// grey bars. `aria-busy="true"` so screen readers know content
/// is loading.
pub fn skeleton_table(rows: usize, columns: usize) -> String {
    let mut out = String::new();
    out.push_str(
        r#"<div aria-busy="true" aria-live="polite" class="border rounded overflow-hidden dark:border-zinc-700">
  <table class="min-w-full text-sm">
    <tbody>"#,
    );
    for _ in 0..rows.min(50) {
        out.push_str(r#"<tr class="border-t dark:border-zinc-800">"#);
        for _ in 0..columns.min(10) {
            out.push_str(
                r#"<td class="px-3 py-2"><div class="h-3 bg-zinc-200 dark:bg-zinc-700 rounded animate-pulse w-3/4"></div></td>"#,
            );
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

/// Error boundary panel — server-rendered when a route's handler
/// caught an unexpected error. Friendly fallback that includes a
/// retry button + a "Report a bug" link prefilled with the
/// failing URL.
pub fn error_panel(error_message: &str, failing_url: &str) -> String {
    let report_url = format!(
        "https://github.com/anthropic/cave-runtime/issues/new?title={}&body={}",
        urlencode("Bug: portal error"),
        urlencode(&format!(
            "URL: {failing_url}\n\nError: {error_message}\n\nWhat were you trying to do?",
            failing_url = failing_url,
            error_message = error_message,
        )),
    );
    format!(
        r##"<div role="alert" class="border border-red-300 dark:border-red-700 bg-red-50 dark:bg-red-900/30 text-red-900 dark:text-red-100 rounded p-4">
  <h2 class="text-base font-semibold mb-1">Something went wrong</h2>
  <p class="text-sm mb-3">{msg}</p>
  <div class="flex gap-2">
    <button onclick="location.reload()"
            class="px-3 py-1.5 rounded bg-red-600 hover:bg-red-700 text-white text-sm focus:outline-none focus:ring-2 focus:ring-red-400">
      Retry
    </button>
    <a href="{report}" target="_blank" rel="noopener"
       class="px-3 py-1.5 rounded border border-red-300 dark:border-red-700 text-sm hover:bg-red-100 dark:hover:bg-red-900/50">
      Report a bug ↗
    </a>
  </div>
</div>"##,
        msg = escape(error_message),
        report = escape(&report_url),
    )
}

/// Render an inline loading spinner — useful inside a button or
/// next to a "Refresh" label. Sized to match the surrounding text.
pub fn loading_spinner() -> String {
    r#"<span aria-label="Loading" role="status" class="inline-block w-3 h-3 border-2 border-zinc-300 dark:border-zinc-600 border-t-blue-600 rounded-full animate-spin"></span>"#.to_string()
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_table_renders_aria_busy_and_n_rows() {
        let html = skeleton_table(5, 3);
        assert!(html.contains(r#"aria-busy="true""#));
        assert!(html.contains(r#"aria-live="polite""#));
        // 5 rows × 3 cells = 15 placeholder divs.
        let count = html.matches("animate-pulse").count();
        assert_eq!(count, 15);
    }

    #[test]
    fn skeleton_table_caps_at_reasonable_size() {
        // Don't render a 1000-row skeleton.
        let html = skeleton_table(1000, 1000);
        let row_count = html.matches("<tr").count();
        assert!(row_count <= 50);
    }

    #[test]
    fn error_panel_has_role_alert_and_retry_button() {
        let html = error_panel("Connection refused", "/admin/keda?tenant_id=x");
        assert!(html.contains(r#"role="alert""#));
        assert!(html.contains("Retry"));
        assert!(html.contains("location.reload()"));
        assert!(html.contains("Connection refused"));
    }

    #[test]
    fn error_panel_report_link_url_encodes_failing_url() {
        let html = error_panel("err", "/admin/x?q=a b");
        // ' ' encodes to %20.
        assert!(html.contains("%20"));
    }

    #[test]
    fn error_panel_escapes_html_in_message() {
        let html = error_panel(r#"<script>alert(1)</script>"#, "/x");
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn loading_spinner_has_aria_label_and_role_status() {
        let html = loading_spinner();
        assert!(html.contains(r#"aria-label="Loading""#));
        assert!(html.contains(r#"role="status""#));
        assert!(html.contains("animate-spin"));
    }

    #[test]
    fn skeleton_dark_mode_variants_present() {
        let html = skeleton_table(2, 2);
        assert!(html.contains("dark:bg-zinc-700"));
        assert!(html.contains("dark:border-zinc-700"));
    }
}
