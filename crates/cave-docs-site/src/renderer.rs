// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Markdown rendering, navigation tree construction, and search index generation.

use crate::models::{DocPage, SearchEntry, SearchIndex};
use chrono::Utc;
use uuid::Uuid;

/// Render markdown to HTML. Handles headings, bold/italic, inline code,
/// fenced code blocks, list items, and paragraphs.
pub fn render_markdown(markdown: &str) -> String {
    let mut html = String::new();
    let mut in_code_block = false;

    for line in markdown.lines() {
        if line.starts_with("```") {
            if in_code_block {
                html.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                let lang = line.trim_start_matches('`').trim();
                if lang.is_empty() {
                    html.push_str("<pre><code>\n");
                } else {
                    html.push_str(&format!("<pre><code class=\"language-{lang}\">\n"));
                }
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            html.push_str(&escape_html(line));
            html.push('\n');
            continue;
        }

        let rendered = render_inline(line);
        let out = if let Some(rest) = rendered.strip_prefix("#### ") {
            format!("<h4>{rest}</h4>\n")
        } else if let Some(rest) = rendered.strip_prefix("### ") {
            format!("<h3>{rest}</h3>\n")
        } else if let Some(rest) = rendered.strip_prefix("## ") {
            format!("<h2>{rest}</h2>\n")
        } else if let Some(rest) = rendered.strip_prefix("# ") {
            format!("<h1>{rest}</h1>\n")
        } else if rendered.starts_with("- ") || rendered.starts_with("* ") {
            format!("<li>{}</li>\n", &rendered[2..])
        } else if rendered.trim().is_empty() {
            "<br/>\n".to_string()
        } else {
            format!("<p>{rendered}</p>\n")
        };
        html.push_str(&out);
    }

    if in_code_block {
        html.push_str("</code></pre>\n");
    }

    html
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_inline(s: &str) -> String {
    let s = replace_delimited(s, "**", "<strong>", "</strong>");
    let s = replace_delimited(&s, "*", "<em>", "</em>");
    replace_delimited(&s, "`", "<code>", "</code>")
}

fn replace_delimited(s: &str, delim: &str, open: &str, close: &str) -> String {
    let mut result = String::new();
    let mut remaining = s;
    let mut is_open = true;
    while let Some(pos) = remaining.find(delim) {
        result.push_str(&remaining[..pos]);
        result.push_str(if is_open { open } else { close });
        remaining = &remaining[pos + delim.len()..];
        is_open = !is_open;
    }
    result.push_str(remaining);
    result
}

/// A single node in the hierarchical navigation tree.
#[derive(Debug, serde::Serialize)]
pub struct NavNode {
    pub id: Uuid,
    pub title: String,
    pub path: String,
    pub order: u32,
    pub children: Vec<NavNode>,
}

/// Build an ordered navigation tree from a flat slice of pages.
pub fn build_nav_tree(pages: &[DocPage]) -> Vec<NavNode> {
    let mut roots: Vec<NavNode> = pages
        .iter()
        .filter(|p| p.parent_id.is_none())
        .map(|p| NavNode {
            id: p.id,
            title: p.title.clone(),
            path: p.path.clone(),
            order: p.order,
            children: collect_children(p.id, pages),
        })
        .collect();
    roots.sort_by_key(|n| n.order);
    roots
}

fn collect_children(parent_id: Uuid, pages: &[DocPage]) -> Vec<NavNode> {
    let mut children: Vec<NavNode> = pages
        .iter()
        .filter(|p| p.parent_id == Some(parent_id))
        .map(|p| NavNode {
            id: p.id,
            title: p.title.clone(),
            path: p.path.clone(),
            order: p.order,
            children: collect_children(p.id, pages),
        })
        .collect();
    children.sort_by_key(|n| n.order);
    children
}

/// Generate a full-text search index for a given version of a site's pages.
pub fn generate_search_index(site_id: Uuid, version: &str, pages: &[DocPage]) -> SearchIndex {
    let entries = pages
        .iter()
        .filter(|p| p.version == version)
        .map(|p| {
            let excerpt = p
                .content
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| {
                    let plain = l.trim_start_matches('#').trim();
                    if plain.len() > 160 {
                        &plain[..160]
                    } else {
                        plain
                    }
                })
                .unwrap_or("")
                .to_string();

            let keywords: Vec<String> = p
                .content
                .split_whitespace()
                .filter(|w| w.len() > 4)
                .take(20)
                .map(|w| {
                    w.to_lowercase()
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_string()
                })
                .filter(|w| !w.is_empty())
                .collect();

            SearchEntry {
                page_id: p.id,
                title: p.title.clone(),
                path: p.path.clone(),
                excerpt,
                keywords,
            }
        })
        .collect();

    SearchIndex {
        site_id,
        version: version.to_string(),
        entries,
        built_at: Utc::now(),
    }
}

/// Stamp all pages with a new version label (used when cutting a doc release).
pub fn version_docs(pages: &mut [DocPage], new_version: &str) {
    for page in pages.iter_mut() {
        page.version = new_version.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DocPage;
    use chrono::Utc;

    fn make_page(title: &str, path: &str, order: u32, parent: Option<Uuid>) -> DocPage {
        DocPage {
            id: Uuid::new_v4(),
            site_id: Uuid::new_v4(),
            title: title.into(),
            path: path.into(),
            content: format!("# {title}\n\nSome **bold** and *italic* content with `code`."),
            order,
            parent_id: parent,
            version: "v1".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_render_heading() {
        let html = render_markdown("# Hello World");
        assert!(html.contains("<h1>Hello World</h1>"));
    }

    #[test]
    fn test_render_code_block() {
        let html = render_markdown("```rust\nlet x = 1;\n```");
        assert!(html.contains("language-rust"));
        assert!(html.contains("let x = 1;"));
    }

    #[test]
    fn test_render_bold_italic() {
        let html = render_markdown("This is **bold** and *italic*.");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn test_nav_tree_parent_child() {
        let root = make_page("Root", "/", 0, None);
        let child = make_page("Child", "/child", 0, Some(root.id));
        let pages = vec![child, root];
        let tree = build_nav_tree(&pages);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
    }

    #[test]
    fn test_nav_tree_ordering() {
        let a = make_page("A", "/a", 2, None);
        let b = make_page("B", "/b", 1, None);
        let pages = vec![a, b];
        let tree = build_nav_tree(&pages);
        assert_eq!(tree[0].title, "B");
        assert_eq!(tree[1].title, "A");
    }

    #[test]
    fn test_search_index_keywords() {
        let page = make_page("Introduction", "/intro", 0, None);
        let idx = generate_search_index(Uuid::new_v4(), "v1", &[page]);
        assert_eq!(idx.entries.len(), 1);
        assert!(!idx.entries[0].keywords.is_empty());
    }

    #[test]
    fn test_search_index_version_filter() {
        let mut page = make_page("Old", "/old", 0, None);
        page.version = "v0".into();
        let idx = generate_search_index(Uuid::new_v4(), "v1", &[page]);
        assert!(idx.entries.is_empty());
    }

    #[test]
    fn test_version_docs() {
        let mut pages = vec![make_page("Page", "/p", 0, None)];
        version_docs(&mut pages, "v2");
        assert_eq!(pages[0].version, "v2");
    }
}
