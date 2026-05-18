// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::renderer::MarkdownRenderer;
use crate::types::*;

pub struct TocGenerator;

impl TocGenerator {
    /// Build full TOC for a space+version: groups → pages → in-page headings
    pub fn build(groups: &[PageGroup], pages: &[Page]) -> Vec<TocEntry> {
        let mut toc = vec![];

        // Ungrouped pages first
        let mut ungrouped: Vec<&Page> = pages.iter().filter(|p| p.group_id.is_none()).collect();
        ungrouped.sort_by_key(|p| p.order);
        for page in ungrouped {
            let mut entry = Self::page_entry(page);
            entry.children = Self::page_headings(&page.markdown_content);
            toc.push(entry);
        }

        // Then groups with their pages
        let mut sorted_groups = groups.to_vec();
        sorted_groups.sort_by_key(|g| g.order);
        for group in &sorted_groups {
            let mut group_entry = TocEntry {
                id: group.id.clone(),
                title: group.title.clone(),
                slug: slug_from_title(&group.title),
                level: 0,
                children: vec![],
                group_id: Some(group.id.clone()),
                page_id: None,
            };
            let mut group_pages: Vec<&Page> = pages
                .iter()
                .filter(|p| p.group_id.as_deref() == Some(&group.id))
                .collect();
            group_pages.sort_by_key(|p| p.order);
            for page in group_pages {
                let mut entry = Self::page_entry(page);
                entry.children = Self::page_headings(&page.markdown_content);
                group_entry.children.push(entry);
            }
            toc.push(group_entry);
        }
        toc
    }

    fn page_entry(page: &Page) -> TocEntry {
        TocEntry {
            id: page.id.clone(),
            title: page.title.clone(),
            slug: page.slug.clone(),
            level: 1,
            children: vec![],
            group_id: page.group_id.clone(),
            page_id: Some(page.id.clone()),
        }
    }

    fn page_headings(markdown: &str) -> Vec<TocEntry> {
        MarkdownRenderer::extract_headings(markdown)
            .into_iter()
            .map(|(level, text, anchor)| TocEntry {
                id: anchor.clone(),
                title: text,
                slug: anchor.clone(),
                level: level + 1,
                children: vec![],
                group_id: None,
                page_id: None,
            })
            .collect()
    }

    /// Build a flat list of pages for prev/next navigation
    pub fn flat_pages(groups: &[PageGroup], pages: &[Page]) -> Vec<Page> {
        let mut result: Vec<Page> = vec![];

        // Ungrouped first
        let mut ungrouped: Vec<Page> = pages
            .iter()
            .filter(|p| p.group_id.is_none())
            .cloned()
            .collect();
        ungrouped.sort_by_key(|p| p.order);
        result.extend(ungrouped);

        // Then grouped
        let mut sorted_groups = groups.to_vec();
        sorted_groups.sort_by_key(|g| g.order);
        for group in &sorted_groups {
            let mut group_pages: Vec<Page> = pages
                .iter()
                .filter(|p| p.group_id.as_deref() == Some(&group.id))
                .cloned()
                .collect();
            group_pages.sort_by_key(|p| p.order);
            result.extend(group_pages);
        }
        result
    }

    /// Generate HTML `<ul>` TOC
    pub fn to_html(entries: &[TocEntry]) -> String {
        if entries.is_empty() {
            return String::new();
        }
        let mut html = String::from("<ul>\n");
        for entry in entries {
            html.push_str(&format!(
                "<li><a href=\"#{slug}\">{title}</a>",
                slug = entry.slug,
                title = entry.title
            ));
            if !entry.children.is_empty() {
                html.push('\n');
                html.push_str(&Self::to_html(&entry.children));
            }
            html.push_str("</li>\n");
        }
        html.push_str("</ul>\n");
        html
    }
}

fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .replace(' ', "-")
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::DocsStore;

    #[test]
    fn toc_build() {
        let store = DocsStore::new();
        let space = store.create_space("toc-space", "Toc Space", "").unwrap();

        let g1 = store
            .create_group(&space.id, "Getting Started", 0, "main")
            .unwrap();
        let g2 = store
            .create_group(&space.id, "Reference", 1, "main")
            .unwrap();

        store
            .create_page(
                &space.id,
                "install",
                "Installation",
                "# Install\n",
                Some(g1.id.clone()),
                0,
                "main",
            )
            .unwrap();
        store
            .create_page(
                &space.id,
                "quickstart",
                "Quickstart",
                "# Quick\n",
                Some(g1.id.clone()),
                1,
                "main",
            )
            .unwrap();
        store
            .create_page(
                &space.id,
                "api",
                "API",
                "# API\n",
                Some(g2.id.clone()),
                0,
                "main",
            )
            .unwrap();
        store
            .create_page(
                &space.id,
                "config",
                "Config",
                "# Config\n",
                Some(g2.id.clone()),
                1,
                "main",
            )
            .unwrap();

        let groups = store.list_groups(&space.id, "main");
        let pages = store.list_pages(&space.id, "main");

        let toc = TocGenerator::build(&groups, &pages);

        // Should have 2 group entries (no ungrouped pages)
        assert_eq!(toc.len(), 2, "expected 2 top-level groups");
        assert_eq!(toc[0].children.len(), 2);
        assert_eq!(toc[1].children.len(), 2);
    }
}
