// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTML rendering helpers.
//!
//! The web shell deliberately avoids template-engine dependencies — the
//! Backstage-style UI needs HTML output, not server-side reactivity, so we
//! escape user-supplied strings with [`escape_html`] and concatenate. This
//! keeps the dependency surface small and makes XSS audits trivial.

use crate::page::{Page, PageRequest, PageResponse};

/// Escape a string for safe insertion into HTML body text or attribute values.
///
/// Escapes `& < > " '` — sufficient for both element content and double-quoted
/// attribute values.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Render a page wrapped in a minimal HTML shell. The caller supplies the
/// inner [`PageResponse`]; this function adds the surrounding `<html>` /
/// `<body>` / nav scaffold.
pub fn render_page(page: &Page, _req: &PageRequest, resp: &PageResponse) -> String {
    let title = escape_html(&resp.title);
    let body = &resp.body; // body is plugin-rendered HTML — already trusted
    let path = escape_html(&page.path);
    let icon = escape_html(&page.icon);
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head><meta charset=\"utf-8\"><title>{title}</title></head>\n\
         <body>\n\
           <header><nav data-path=\"{path}\" data-icon=\"{icon}\"></nav></header>\n\
           <main>{body}</main>\n\
         </body>\n\
         </html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Scope;

    #[test]
    fn escape_html_passes_plain_text_through() {
        assert_eq!(escape_html("hello world"), "hello world");
    }

    #[test]
    fn escape_html_escapes_ampersand() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn escape_html_escapes_lt_gt() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
    }

    #[test]
    fn escape_html_escapes_double_quote() {
        assert_eq!(escape_html("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn escape_html_escapes_single_quote() {
        assert_eq!(escape_html("it's"), "it&#39;s");
    }

    #[test]
    fn escape_html_escapes_xss_payload() {
        let s = escape_html("<img src=x onerror=alert('x')>");
        assert!(!s.contains('<'));
        assert!(!s.contains('>'));
        assert!(s.contains("&lt;"));
    }

    #[test]
    fn escape_html_preserves_unicode() {
        assert_eq!(escape_html("naïve résumé"), "naïve résumé");
    }

    #[test]
    fn escape_html_empty_string() {
        assert_eq!(escape_html(""), "");
    }

    #[test]
    fn render_page_includes_title() {
        let p = Page::builder("a", "/a")
            .title("Home")
            .scope(Scope::Public)
            .build();
        let resp = PageResponse::ok("Home", "<p>hi</p>");
        let html = render_page(&p, &PageRequest::new("/a"), &resp);
        assert!(html.contains("<title>Home</title>"));
    }

    #[test]
    fn render_page_includes_body() {
        let p = Page::builder("a", "/a").scope(Scope::Public).build();
        let resp = PageResponse::ok("T", "<p>greetings</p>");
        let html = render_page(&p, &PageRequest::new("/a"), &resp);
        assert!(html.contains("<p>greetings</p>"));
    }

    #[test]
    fn render_page_includes_doctype() {
        let p = Page::builder("a", "/a").scope(Scope::Public).build();
        let html = render_page(&p, &PageRequest::new("/a"), &PageResponse::ok("T", ""));
        assert!(html.starts_with("<!doctype html>"));
    }

    #[test]
    fn render_page_escapes_title() {
        let resp = PageResponse::ok("<bad>", "");
        let p = Page::builder("a", "/a").scope(Scope::Public).build();
        let html = render_page(&p, &PageRequest::new("/a"), &resp);
        assert!(!html.contains("<title><bad>"));
        assert!(html.contains("&lt;bad&gt;"));
    }

    #[test]
    fn render_page_escapes_path_and_icon_in_attrs() {
        let p = Page::builder("a", "/a\"x")
            .icon("ic\"on")
            .scope(Scope::Public)
            .build();
        let html = render_page(&p, &PageRequest::new("/a\"x"), &PageResponse::ok("T", ""));
        assert!(html.contains("data-path=\"/a&quot;x\""));
        assert!(html.contains("data-icon=\"ic&quot;on\""));
    }

    #[test]
    fn render_page_includes_nav_element() {
        let p = Page::builder("a", "/a").scope(Scope::Public).build();
        let html = render_page(&p, &PageRequest::new("/a"), &PageResponse::ok("T", ""));
        assert!(html.contains("<nav"));
        assert!(html.contains("</nav>"));
    }

    #[test]
    fn render_page_includes_main_element() {
        let p = Page::builder("a", "/a").scope(Scope::Public).build();
        let html = render_page(&p, &PageRequest::new("/a"), &PageResponse::ok("T", "x"));
        assert!(html.contains("<main>x</main>"));
    }
}
