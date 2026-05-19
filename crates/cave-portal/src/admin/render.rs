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
///
/// **2026-05-13 UX foundation update.** This shell delegates to
/// [`crate::admin::layout::shell::shell_v2`] with the full chrome
/// (top bar, sidebar, breadcrumb, command palette, shortcuts, toasts,
/// dark-mode toggle, footer).
///
/// **2026-05-14 sidebar mass adoption.** The default no longer hides
/// the sidebar — every handler that calls `page_shell(title, body)`
/// now ships with the navigation visible. The fallback persona is
/// [`Persona::PlatformAdmin`] because the legacy callers (~136 of
/// them) all sit behind the platform-admin JWT gate; rendering the
/// full nav for them is the right default. Callers that want
/// persona-aware filtering + active-route highlight should migrate
/// to [`page_shell_full`].
///
/// Existing tests that assert the presence of `/static/htmx.min.js`,
/// `/static/tailwind-light.css`, and the escaped
/// `<h1>{title}</h1>` / `<title>{title} — cave admin</title>` continue
/// to pass because shell_v2 emits exactly those tags.
pub fn page_shell(title: &str, body: &str) -> String {
    use crate::admin::layout::shell::{shell_v2, ShellOptions};
    use crate::admin::permission::Persona;
    shell_v2(ShellOptions {
        title,
        persona: Persona::PlatformAdmin,
        tenant_id: "dev",
        current_path: "/",
        theme_cookie: None,
        breadcrumb: None,
        extra_commands: Vec::new(),
        cluster_info: "cave-runtime",
        hide_sidebar: false,
        body,
    })
}

/// Render the admin-page shell with full request context.
///
/// This is the preferred form for any handler that has a
/// [`crate::admin::permission::RequestCtx`] in scope (and they all
/// should — the JWT middleware constructs one on every request).
///
/// Compared to [`page_shell`], this routes the *real* persona +
/// tenant id + current URL path into the chrome, so:
///
///   * the sidebar's persona filter actually filters
///     (TenantAdmin only sees the tenant-relevant items),
///   * the active item in the sidebar gets the `aria-current="page"`
///     marker + the highlighted styling, and
///   * the breadcrumb resolves the right trail from the current URL.
///
/// `current_path` should be the path the user is viewing — e.g.
/// `/admin/keda` for the keda list page. Pass an empty string and
/// the breadcrumb will degrade to "Home" only.
///
/// 2026-05-14 — added during the sidebar mass-adoption sweep. New
/// handlers should call this directly; legacy handlers continue to
/// work via [`page_shell`].
pub fn page_shell_full(
    ctx: &crate::admin::permission::RequestCtx,
    current_path: &str,
    title: &str,
    body: &str,
) -> String {
    use crate::admin::layout::shell::{shell_v2, ShellOptions};
    shell_v2(ShellOptions {
        title,
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path,
        theme_cookie: None,
        breadcrumb: None,
        extra_commands: Vec::new(),
        cluster_info: "cave-runtime",
        hide_sidebar: false,
        body,
    })
}

/// Render an HTML table. `headers` and `rows` are escaped; the caller does
/// not need to.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(r#"<table class="min-w-full text-sm border-collapse">"#);
    out.push_str(r#"<thead class="bg-gray-100"><tr>"#);
    for h in headers {
        let _ = write!(out, r#"<th scope="col" class="px-3 py-2 text-left">{}</th>"#, escape(h));
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

/// Render an HTML table whose cells are **already-escaped raw HTML**
/// (badges, links, icons). Headers are still escaped — those are
/// always plain-text labels in this codebase. The caller is
/// responsible for escaping any user-controlled substrings inside
/// each cell.
///
/// Added 2026-05-13 to fix the compliance-matrix double-escape bug:
/// `compliance.rs` builds per-cell HTML like
/// `<span class="...">92%</span>`, which `table()` would render as
/// literal `&lt;span class=...&gt;92%&lt;/span&gt;` text. The
/// dashboard's matrix now goes through `table_html` so badges /
/// links / status icons render as intended.
pub fn table_html(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(r#"<table class="min-w-full text-sm border-collapse">"#);
    out.push_str(r#"<thead class="bg-gray-100"><tr>"#);
    for h in headers {
        let _ = write!(out, r#"<th scope="col" class="px-3 py-2 text-left">{}</th>"#, escape(h));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str(r#"<tr class="border-t">"#);
        for cell in row {
            // Emit cell verbatim — the caller has already produced HTML.
            let _ = write!(out, r#"<td class="px-3 py-2">{}</td>"#, cell);
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
    fn table_html_emits_cells_verbatim() {
        // The 2026-05-13 sibling to `table()`: when the caller has
        // already produced safe HTML for each cell (badges, links,
        // icons), `table_html` must NOT escape it. Headers are still
        // escaped because they are always plain-text labels in this
        // codebase.
        let (_cite, _t) = crate::portal_test_ctx!(
            "packages/core-components/src/components/Table/Table.tsx",
            "TableHtml",
            "tenant-render-table-html"
        );
        let html = table_html(
            &["score"],
            &[vec![r#"<span class="px-2 rounded bg-green-100">92%</span>"#.into()]],
        );
        // The span survives intact (not double-escaped).
        assert!(html.contains(r#"<span class="px-2 rounded bg-green-100">92%</span>"#));
        // And it does NOT appear as escaped text.
        assert!(!html.contains("&lt;span"));
        assert!(!html.contains("&quot;"));
    }

    #[test]
    fn table_html_still_escapes_headers() {
        // Headers are always plain-text labels — escape them just like
        // `table()` does, even though cell content is verbatim.
        let html = table_html(&["a<b"], &[vec!["x".into()]]);
        assert!(html.contains("a&lt;b"));
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
