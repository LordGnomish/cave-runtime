// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
    use crate::admin::layout::shell::{ShellOptions, shell_v2};
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
    use crate::admin::layout::shell::{ShellOptions, shell_v2};
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
        let _ = write!(out, r#"<th class="px-3 py-2 text-left">{}</th>"#, escape(h));
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

// ── UX primitives (2026-05-22) ────────────────────────────────────────
//
// Small, composable HTML fragments that every admin handler can reach
// for instead of hand-rolling its own empty / error / loading markup.
// Each primitive ships with a matching `.cave-*` class defined in
// crates/cave-portal/assets/tailwind-light.css so the visual treatment
// stays consistent across pages. Tested in tests/render_primitives.rs.

/// Render a "no data yet" placeholder card. Used in place of an empty
/// table or list, with a glanceable icon, a short title, and an
/// optional hint that explains how the data would appear.
///
/// `icon` is rendered verbatim (caller picks an emoji or inline SVG —
/// keep it short, it goes inside an `aria-hidden="true"` span).
/// `title` and `hint` are escaped.
pub fn empty_state(icon: &str, title: &str, hint: &str) -> String {
    format!(
        r#"<div class="cave-empty" role="status">
  <div class="cave-empty-icon" aria-hidden="true">{icon}</div>
  <div class="cave-empty-title">{title}</div>
  <div class="cave-empty-hint">{hint}</div>
</div>"#,
        icon = icon,
        title = escape(title),
        hint = escape(hint),
    )
}

/// Render a recoverable error notice. Distinct from
/// [`permission_denied`] (which is a 403 specifically). Use for
/// upstream-fetch failures, parsing errors, etc.
pub fn error_state(title: &str, detail: &str) -> String {
    format!(
        r#"<div class="cave-error" role="alert">
  <strong>{title}:</strong> {detail}
</div>"#,
        title = escape(title),
        detail = escape(detail),
    )
}

/// Render a skeleton loader placeholder — `count` shimmering bars at
/// roughly table-row height. Replace with real content once the data
/// loads (e.g. via the htmx swap target). Pure CSS animation, no JS.
pub fn skeleton(rows: usize) -> String {
    let mut out = String::from(r#"<div aria-busy="true" aria-label="Loading">"#);
    for _ in 0..rows {
        out.push_str(r#"<div class="cave-skeleton h-3 w-full" style="margin-bottom:0.5rem"></div>"#);
    }
    out.push_str("</div>");
    out
}

/// Render a coloured badge / pill. `tone` picks the colour family —
/// `ok`, `warn`, `bad`, `info`, `neutral`. `text` is escaped.
pub fn badge(tone: &str, text: &str) -> String {
    let cls = match tone {
        "ok" => "bg-green-100 text-green-900",
        "warn" => "bg-yellow-100 text-yellow-900",
        "bad" => "bg-red-100 text-red-900",
        "info" => "bg-blue-100 text-blue-900",
        _ => "bg-zinc-100 text-zinc-700",
    };
    format!(
        r#"<span class="px-2 py-0.5 rounded text-xs {cls}">{text}</span>"#,
        cls = cls,
        text = escape(text),
    )
}

/// Render a client-side search input bound to a sibling table.
/// `target` is the CSS id of the table whose `<tbody><tr>` rows
/// will be filtered. A small inline script wires the input to the
/// rows — no htmx round-trip. Matches against the visible text of
/// every cell in the row, case-insensitively.
///
/// `placeholder` and `target` are escaped.
pub fn search_box(target: &str, placeholder: &str) -> String {
    let target_e = escape(target);
    let placeholder_e = escape(placeholder);
    // Deterministic id derived from target so multiple search boxes
    // on the same page don't collide.
    let input_id = format!("cave-search-{}", target.replace('#', "").replace('.', "_"));
    let input_id_e = escape(&input_id);
    format!(
        r#"<div class="mb-2">
  <input type="search" id="{id}" class="cave-search" placeholder="{ph}" autocomplete="off" aria-controls="{tgt}" />
  <script>(function() {{
    var input = document.getElementById('{id}');
    var table = document.querySelector('{tgt}');
    if (!input || !table) return;
    var rows = table.querySelectorAll('tbody tr');
    var counter = document.getElementById('{id}-count');
    function apply() {{
      var q = input.value.trim().toLowerCase();
      var shown = 0;
      rows.forEach(function (r) {{
        var hay = (r.textContent || '').toLowerCase();
        var hit = q === '' || hay.indexOf(q) !== -1;
        r.style.display = hit ? '' : 'none';
        if (hit) shown++;
      }});
      if (counter) counter.textContent = String(shown);
    }}
    input.addEventListener('input', apply);
  }})();</script>
  <span class="text-xs text-zinc-500 ml-2">showing <span id="{id}-count">{init_count}</span> rows</span>
</div>"#,
        id = input_id_e,
        ph = placeholder_e,
        tgt = target_e,
        init_count = "—",
    )
}

/// Render a client-side sortable table. Headers are emitted with
/// `data-sort="<kind>"` so the inline script can compare cells
/// correctly. `kind` for each column is one of `"text"` (default),
/// `"num"` (parses leading numeric value), `"date"` (ISO-8601
/// lexical sort works).
///
/// `rows` cells are emitted verbatim — caller has already produced
/// safe HTML (matches [`table_html`] contract). Headers + sort
/// kinds are escaped.
pub fn sortable_table(
    table_id: &str,
    headers: &[(&str, &str)],
    rows: &[Vec<String>],
) -> String {
    let table_id_e = escape(table_id);
    let mut out = format!(
        r#"<div class="cave-table-wrap"><table id="{id}" class="min-w-full text-sm border-collapse" data-sortable><thead class="bg-gray-100"><tr>"#,
        id = table_id_e,
    );
    for (h, kind) in headers {
        let _ = write!(
            out,
            r#"<th class="px-3 py-2 text-left" scope="col" data-sort="{kind}" tabindex="0" role="button" aria-sort="none">{label}</th>"#,
            kind = escape(kind),
            label = escape(h),
        );
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str(r#"<tr class="border-t">"#);
        for cell in row {
            let _ = write!(out, r#"<td class="px-3 py-2">{}</td>"#, cell);
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    // Inline sort script — once per table, idempotent.
    out.push_str(&format!(
        r#"<script>(function() {{
  var t = document.getElementById('{id}');
  if (!t || t.__caveSorted) return;
  t.__caveSorted = true;
  var ths = t.querySelectorAll('thead th[data-sort]');
  ths.forEach(function (th, idx) {{
    function activate() {{
      var kind = th.getAttribute('data-sort') || 'text';
      var dir = th.getAttribute('aria-sort');
      var asc = dir !== 'ascending';
      ths.forEach(function (o) {{ o.setAttribute('aria-sort', 'none'); }});
      th.setAttribute('aria-sort', asc ? 'ascending' : 'descending');
      var tbody = t.querySelector('tbody');
      var rs = Array.prototype.slice.call(tbody.querySelectorAll('tr'));
      rs.sort(function (a, b) {{
        var av = (a.cells[idx] && a.cells[idx].textContent || '').trim();
        var bv = (b.cells[idx] && b.cells[idx].textContent || '').trim();
        var cmp = 0;
        if (kind === 'num') {{
          var an = parseFloat(av.replace(/[^0-9.\-]/g, '')); if (isNaN(an)) an = -Infinity;
          var bn = parseFloat(bv.replace(/[^0-9.\-]/g, '')); if (isNaN(bn)) bn = -Infinity;
          cmp = an - bn;
        }} else {{
          cmp = av.localeCompare(bv, undefined, {{ numeric: true, sensitivity: 'base' }});
        }}
        return asc ? cmp : -cmp;
      }});
      rs.forEach(function (r) {{ tbody.appendChild(r); }});
    }}
    th.addEventListener('click', activate);
    th.addEventListener('keydown', function (e) {{
      if (e.key === 'Enter' || e.key === ' ') {{ e.preventDefault(); activate(); }}
    }});
  }});
}})();</script>"#,
        id = table_id_e,
    ));
    out
}

/// Render lightweight markdown — enough for ADR detail bodies and
/// release notes. Supports:
///   * # / ## / ### / #### headings (only at line start)
///   * fenced ```code``` blocks
///   * inline `code`
///   * `**bold**` and `*italic*`
///   * `- ` / `* ` / `N. ` lists
///   * `> ` blockquote
///   * `[text](url)` links
///   * `---` horizontal rules
///   * paragraphs separated by blank lines
///
/// Any HTML inside the source is escaped first; only the recognised
/// markdown is upgraded back into HTML. The caller wraps the output
/// in a `<div class="cave-md">` to pick up the cave-brand typography.
pub fn markdown_lite(src: &str) -> String {
    let escaped = escape(src);
    let mut out = String::with_capacity(escaped.len() + 64);
    let lines: Vec<&str> = escaped.lines().collect();
    let mut i = 0;
    let mut in_para = false;
    let mut in_list = false;
    let mut list_tag = "ul";

    fn close_para(out: &mut String, in_para: &mut bool) {
        if *in_para {
            out.push_str("</p>");
            *in_para = false;
        }
    }
    fn close_list(out: &mut String, in_list: &mut bool, list_tag: &str) {
        if *in_list {
            out.push_str(&format!("</{list_tag}>"));
            *in_list = false;
        }
    }

    while i < lines.len() {
        let raw = lines[i];
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        // Fenced code
        if trimmed.starts_with("```") {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            out.push_str("<pre><code>");
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                out.push_str(lines[i]);
                out.push('\n');
                i += 1;
            }
            out.push_str("</code></pre>");
            if i < lines.len() { i += 1; } // skip closing fence
            continue;
        }

        // Horizontal rule
        if line == "---" || line == "***" {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            out.push_str("<hr>");
            i += 1;
            continue;
        }

        // Blank line — close paragraph / list
        if line.is_empty() {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            i += 1;
            continue;
        }

        // Headings
        if let Some(rest) = trimmed.strip_prefix("#### ") {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            out.push_str(&format!("<h4>{}</h4>", markdown_inline(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("### ") {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            out.push_str(&format!("<h3>{}</h3>", markdown_inline(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            out.push_str(&format!("<h2>{}</h2>", markdown_inline(rest)));
            i += 1;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            close_para(&mut out, &mut in_para);
            close_list(&mut out, &mut in_list, list_tag);
            out.push_str(&format!("<h1>{}</h1>", markdown_inline(rest)));
            i += 1;
            continue;
        }

        // Blockquote
        if let Some(rest) = trimmed.strip_prefix("&gt; ") {
            close_list(&mut out, &mut in_list, list_tag);
            close_para(&mut out, &mut in_para);
            out.push_str(&format!("<blockquote>{}</blockquote>", markdown_inline(rest)));
            i += 1;
            continue;
        }

        // List items (unordered)
        let bullet = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* "));
        if let Some(rest) = bullet {
            close_para(&mut out, &mut in_para);
            if !in_list || list_tag != "ul" {
                close_list(&mut out, &mut in_list, list_tag);
                list_tag = "ul";
                out.push_str("<ul>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", markdown_inline(rest)));
            i += 1;
            continue;
        }

        // Ordered list — `N. text`
        let ord = trimmed
            .find('.')
            .filter(|&dot| dot > 0 && dot < 4 && trimmed[..dot].chars().all(|c| c.is_ascii_digit()))
            .and_then(|dot| trimmed.get(dot + 1..))
            .and_then(|after| after.strip_prefix(' '));
        if let Some(rest) = ord {
            close_para(&mut out, &mut in_para);
            if !in_list || list_tag != "ol" {
                close_list(&mut out, &mut in_list, list_tag);
                list_tag = "ol";
                out.push_str("<ol>");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>", markdown_inline(rest)));
            i += 1;
            continue;
        }

        // Plain paragraph line — concatenate
        close_list(&mut out, &mut in_list, list_tag);
        if !in_para {
            out.push_str("<p>");
            in_para = true;
        } else {
            out.push(' ');
        }
        out.push_str(&markdown_inline(line));
        i += 1;
    }

    close_para(&mut out, &mut in_para);
    close_list(&mut out, &mut in_list, list_tag);
    out
}

/// Inline-level markdown: `code`, **bold**, *italic*, [text](url).
/// Operates on already-HTML-escaped input.
fn markdown_inline(input: &str) -> String {
    // Inline code first (so its body isn't treated as bold/italic).
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '`' {
            if let Some(end_rel) = input[i + 1..].find('`') {
                let body = &input[i + 1..i + 1 + end_rel];
                out.push_str("<code>");
                out.push_str(body);
                out.push_str("</code>");
                i += 1 + end_rel + 1;
                continue;
            }
        }
        // Links: [text](http(s)://…)
        if c == '[' {
            if let Some(close_rel) = input[i + 1..].find(']') {
                let after = i + 1 + close_rel + 1;
                if after < bytes.len() && bytes[after] as char == '(' {
                    if let Some(url_end_rel) = input[after + 1..].find(')') {
                        let text = &input[i + 1..i + 1 + close_rel];
                        let url = &input[after + 1..after + 1 + url_end_rel];
                        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with('/') {
                            out.push_str(&format!(
                                r#"<a href="{url}" target="_blank" rel="noopener">{text}</a>"#,
                            ));
                            i = after + 1 + url_end_rel + 1;
                            continue;
                        }
                    }
                }
            }
        }
        // Bold ** … **
        if c == '*' && i + 1 < bytes.len() && bytes[i + 1] as char == '*' {
            if let Some(end_rel) = input[i + 2..].find("**") {
                let body = &input[i + 2..i + 2 + end_rel];
                out.push_str("<strong>");
                out.push_str(body);
                out.push_str("</strong>");
                i = i + 2 + end_rel + 2;
                continue;
            }
        }
        // Italic * … *  (single)
        if c == '*' {
            if let Some(end_rel) = input[i + 1..].find('*') {
                // Skip an adjacent '*' (would be bold open we missed).
                let body = &input[i + 1..i + 1 + end_rel];
                if !body.is_empty() && !body.starts_with('*') {
                    out.push_str("<em>");
                    out.push_str(body);
                    out.push_str("</em>");
                    i = i + 1 + end_rel + 1;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
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
            &[vec![
                r#"<span class="px-2 rounded bg-green-100">92%</span>"#.into(),
            ]],
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
        let html = table(&["key", "value"], &[vec!["a&b".into(), r#"<x>"#.into()]]);
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

    // ── UX primitives (2026-05-22) ─────────────────────────────────

    #[test]
    fn empty_state_renders_card_with_role_status() {
        let html = empty_state("📭", "No events", "Daemon hasn't seen any yet.");
        assert!(html.contains(r#"class="cave-empty""#));
        assert!(html.contains(r#"role="status""#));
        assert!(html.contains("📭"));
        assert!(html.contains("No events"));
        assert!(html.contains("Daemon hasn&#39;t seen any yet."));
    }

    #[test]
    fn empty_state_escapes_user_content() {
        let html = empty_state("<x>", "a<b", "c<d");
        // Icon is verbatim — caller controls it. Title + hint escaped.
        assert!(html.contains("<x>"));
        assert!(html.contains("a&lt;b"));
        assert!(html.contains("c&lt;d"));
    }

    #[test]
    fn error_state_renders_alert_role() {
        let html = error_state("Fetch failed", "connection refused");
        assert!(html.contains(r#"role="alert""#));
        assert!(html.contains(r#"class="cave-error""#));
        assert!(html.contains("Fetch failed"));
        assert!(html.contains("connection refused"));
    }

    #[test]
    fn skeleton_emits_aria_busy_and_n_bars() {
        let html = skeleton(4);
        assert!(html.contains(r#"aria-busy="true""#));
        assert_eq!(html.matches("cave-skeleton").count(), 4);
    }

    #[test]
    fn badge_picks_color_class_from_tone() {
        let ok = badge("ok", "synced");
        let bad = badge("bad", "pending");
        let neutral = badge("xyz", "n/a");
        assert!(ok.contains("bg-green-100") && ok.contains("synced"));
        assert!(bad.contains("bg-red-100"));
        assert!(neutral.contains("bg-zinc-100"));
    }

    #[test]
    fn badge_escapes_text() {
        let html = badge("info", "<x>");
        assert!(html.contains("&lt;x&gt;"));
    }

    #[test]
    fn search_box_targets_id_and_wires_input_handler() {
        let html = search_box("#parity-table", "Filter by crate…");
        assert!(html.contains(r##"id="cave-search-parity-table""##));
        assert!(html.contains(r##"aria-controls="#parity-table""##));
        assert!(html.contains(r##"placeholder="Filter by crate…""##));
        assert!(html.contains("addEventListener('input', apply)"));
    }

    #[test]
    fn sortable_table_emits_data_sortable_and_sort_kinds() {
        let html = sortable_table(
            "t1",
            &[("name", "text"), ("score", "num")],
            &[vec!["alpha".into(), r#"<span>92</span>"#.into()]],
        );
        assert!(html.contains(r#"id="t1""#));
        assert!(html.contains("data-sortable"));
        assert!(html.contains(r#"data-sort="text""#));
        assert!(html.contains(r#"data-sort="num""#));
        assert!(html.contains(r#"aria-sort="none""#));
        // Cells verbatim — caller controls HTML safety.
        assert!(html.contains("<span>92</span>"));
    }

    #[test]
    fn sortable_table_script_is_idempotent_per_id() {
        let html = sortable_table("only-one", &[("h", "text")], &[]);
        // The wired flag check inside the inline script.
        assert!(html.contains("__caveSorted"));
    }

    // ── markdown_lite ────────────────────────────────────────────

    #[test]
    fn markdown_renders_headings() {
        let html = markdown_lite("# H1\n\n## H2\n\n### H3");
        assert!(html.contains("<h1>H1</h1>"));
        assert!(html.contains("<h2>H2</h2>"));
        assert!(html.contains("<h3>H3</h3>"));
    }

    #[test]
    fn markdown_renders_lists() {
        let html = markdown_lite("- a\n- b\n- c");
        assert!(html.contains("<ul><li>a</li><li>b</li><li>c</li></ul>"));
        let ord = markdown_lite("1. one\n2. two");
        assert!(ord.contains("<ol><li>one</li><li>two</li></ol>"));
    }

    #[test]
    fn markdown_renders_fenced_code_and_keeps_html_escaped() {
        let html = markdown_lite("```\nfn main() { <x> }\n```");
        assert!(html.contains("<pre><code>"));
        assert!(html.contains("fn main() { &lt;x&gt; }"));
    }

    #[test]
    fn markdown_renders_inline_code_bold_italic() {
        let html = markdown_lite("text with `code` and **bold** and *em*.");
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>em</em>"));
    }

    #[test]
    fn markdown_renders_links_with_noopener() {
        let html = markdown_lite("[upstream](https://github.com/foo/bar)");
        assert!(html.contains(r#"href="https://github.com/foo/bar""#));
        assert!(html.contains(r#"rel="noopener""#));
        assert!(html.contains(">upstream</a>"));
    }

    #[test]
    fn markdown_escapes_html_in_source() {
        let html = markdown_lite("paragraph with <script>alert(1)</script>");
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn markdown_renders_blockquote_and_hr() {
        let html = markdown_lite("> a quote\n\n---\n\nnext");
        assert!(html.contains("<blockquote>a quote</blockquote>"));
        assert!(html.contains("<hr>"));
    }
}
