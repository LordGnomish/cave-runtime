//! HTML rendering primitives.
//!
//! Server-rendered htmx + Tailwind-light. Mirrors the layout shell from
//! `packages/core-components/src/layout/Page/Page.tsx` upstream, adapted
//! to a stateless Rust string-builder. No template engine is used; each
//! helper writes valid escaped HTML directly into a `String`.
//!
//! All user data MUST flow through [`escape`]; raw interpolation is a bug.

use std::fmt::Write;

/// HTML attribute / text escaper. Escapes the five XML special characters.
pub fn escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Render the standard admin-page shell.
///
/// `title` is shown in the `<title>` and as the H1.
/// `body` is interpolated as-is — callers MUST escape user data before
/// passing it in.
pub fn page_shell(title: &str, body: &str) -> String {
    let title_e = escape(title);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{title} — cave admin</title>
  <script src="/static/htmx.min.js" defer></script>
  <link rel="stylesheet" href="/static/tailwind-light.css">
</head>
<body class="bg-gray-50 text-gray-900 font-sans">
  <header class="border-b bg-white px-4 py-3">
    <h1 class="text-xl font-semibold">{title}</h1>
  </header>
  <main class="px-4 py-6 max-w-6xl mx-auto">
{body}
  </main>
</body>
</html>"#,
        title = title_e,
        body = body,
    )
}

/// Render an HTML table. `headers` and `rows` are escaped; the caller does
/// not need to.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(r#"<table class="min-w-full text-sm border-collapse">"#);
    out.push_str(r#"<thead class="bg-gray-100"><tr>"#);
    for h in headers {
        let _ = write!(out, r#"<th class="px-3 py-2 text-left">{}</th>"#, escape(h));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str(r#"<tr class="border-t">"#);
        for cell in row {
            let _ = write!(out, r#"<td class="px-3 py-2">{}</td>"#, escape(cell));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// Render a small htmx-driven action button. `hx_target` is the CSS selector
/// the response should swap into; `label` is the visible text.
pub fn htmx_button(hx_get: &str, hx_target: &str, label: &str) -> String {
    format!(
        r#"<button hx-get="{}" hx-target="{}" hx-swap="innerHTML" class="px-3 py-1 rounded bg-blue-600 text-white">{}</button>"#,
        escape(hx_get),
        escape(hx_target),
        escape(label),
    )
}

/// Render a permission-denied notice.
pub fn permission_denied(message: &str) -> String {
    format!(
        r#"<div class="rounded border border-red-300 bg-red-50 px-4 py-3 text-red-800">
  <strong>permission denied:</strong> {}
</div>"#,
        escape(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    #[test]
    fn escape_handles_html_specials_and_quotes() {
        let (_cite, _t) = portal_test_ctx!(
            "packages/core-components/src/layout/Page/Page.tsx",
            "Page",
            "tenant-render-escape"
        );
        let s = escape(r#"<script>alert("x")</script> & 'foo'"#);
        assert_eq!(
            s,
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &#39;foo&#39;"
        );
    }

    #[test]
    fn page_shell_escapes_title() {
        let (_cite, _t) = portal_test_ctx!(
            "packages/core-components/src/layout/Page/Page.tsx",
            "Page",
            "tenant-render-shell"
        );
        let html = page_shell("a<b", "<p>body</p>");
        assert!(html.contains("a&lt;b"));
        assert!(html.contains("/static/htmx.min.js"));
        assert!(html.contains("/static/tailwind-light.css"));
        assert!(html.contains("<p>body</p>"));
    }

    #[test]
    fn table_escapes_every_cell() {
        let (_cite, _t) = portal_test_ctx!(
            "packages/core-components/src/components/Table/Table.tsx",
            "Table",
            "tenant-render-table"
        );
        let html = table(
            &["key", "value"],
            &[vec!["a&b".into(), r#"<x>"#.into()]],
        );
        assert!(html.contains("a&amp;b"));
        assert!(html.contains("&lt;x&gt;"));
        assert!(!html.contains(">a&b<"));
    }

    #[test]
    fn htmx_button_writes_hx_attrs() {
        let (_cite, _t) = portal_test_ctx!(
            "packages/core-components/src/components/Button/Button.tsx",
            "Button",
            "tenant-render-htmx"
        );
        let html = htmx_button("/admin/etcd?refresh=1", "#kv", "Refresh");
        assert!(html.contains(r#"hx-get="/admin/etcd?refresh=1""#));
        assert!(html.contains(r##"hx-target="#kv""##));
        assert!(html.contains(">Refresh<"));
    }

    #[test]
    fn permission_denied_renders_red_panel_with_escaped_message() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/components/PermissionedRoute.tsx",
            "PermissionedRoute",
            "tenant-render-denied"
        );
        let html = permission_denied("missing <perm>");
        assert!(html.contains("border-red-300"));
        assert!(html.contains("missing &lt;perm&gt;"));
    }
}
