use pulldown_cmark::{html, Options, Parser};
use std::collections::HashMap;

#[allow(dead_code)]
pub struct MarkdownRenderer {
    pub options: Options,
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_FOOTNOTES);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
        Self { options }
    }

    pub fn render(&self, markdown: &str) -> String {
        let parser = Parser::new_ext(markdown, self.options);
        let mut html_output = String::new();
        html::push_html(&mut html_output, parser);
        html_output
    }

    /// Extract frontmatter (--- ... ---) from markdown.
    /// Returns (metadata, remaining_content).
    pub fn extract_frontmatter(content: &str) -> (HashMap<String, String>, &str) {
        if !content.starts_with("---") {
            return (Default::default(), content);
        }
        let after_first = &content[3..];
        // find the closing ---
        let end = after_first.find("---").map(|i| i + 6);
        if let Some(end_pos) = end {
            let fm_section = &content[3..end_pos - 3];
            let mut map = HashMap::new();
            for line in fm_section.lines() {
                if let Some((k, v)) = line.split_once(':') {
                    map.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
            (map, &content[end_pos..])
        } else {
            (Default::default(), content)
        }
    }

    /// Extract heading anchors for TOC: returns Vec<(level, text, anchor)>
    pub fn extract_headings(markdown: &str) -> Vec<(usize, String, String)> {
        let mut headings = vec![];
        for line in markdown.lines() {
            if line.starts_with('#') {
                let level = line.chars().take_while(|c| *c == '#').count();
                let text = line[level..].trim().to_string();
                let anchor = text
                    .to_lowercase()
                    .replace(' ', "-")
                    .replace(|c: char| !c.is_alphanumeric() && c != '-', "");
                headings.push((level, text, anchor));
            }
        }
        headings
    }

    /// Render with line-numbers class on pre blocks.
    #[allow(dead_code)]
    pub fn render_with_line_numbers(&self, markdown: &str) -> String {
        let html = self.render(markdown);
        html.replace("<pre>", "<pre class=\"line-numbers\">")
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_markdown_basic() {
        let renderer = MarkdownRenderer::new();
        let html = renderer.render("# Hello\n\n**bold**");
        assert!(html.contains("<h1"), "expected h1 tag, got: {html}");
        assert!(html.contains("<strong>"), "expected strong tag, got: {html}");
    }

    #[test]
    fn render_markdown_tables() {
        let renderer = MarkdownRenderer::new();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let html = renderer.render(md);
        assert!(html.contains("<table>"), "expected table tag, got: {html}");
    }

    #[test]
    fn extract_frontmatter() {
        let content = "---\ntitle: Test\nauthor: Alice\n---\n# Content";
        let (meta, rest) = MarkdownRenderer::extract_frontmatter(content);
        assert_eq!(meta.get("title").map(|s| s.as_str()), Some("Test"));
        assert!(rest.contains("# Content"), "remaining content should start with heading");
    }

    #[test]
    fn extract_headings() {
        let md = "# H1\n\nsome text\n\n## H2 Title\n\n### Deep\n";
        let headings = MarkdownRenderer::extract_headings(md);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0].0, 1);
        assert_eq!(headings[1].0, 2);
        assert_eq!(headings[2].0, 3);
        assert_eq!(headings[1].2, "h2-title");
    }
}
