use crate::types::*;
use std::collections::HashMap;

pub struct SearchIndex {
    // term -> Vec<(page_id, frequency)>
    inverted: HashMap<String, Vec<(String, u32)>>,
    // page_id -> (title, slug, space_id, version, word_count)
    pages: HashMap<String, (String, String, String, String, u32)>,
}

impl SearchIndex {
    pub fn new() -> Self {
        SearchIndex {
            inverted: HashMap::new(),
            pages: HashMap::new(),
        }
    }

    pub fn index_page(&mut self, page: &Page) {
        let text = format!("{} {}", page.title, page.markdown_content);
        let terms = tokenize(&text);
        let word_count = terms.len() as u32;
        self.pages.insert(
            page.id.clone(),
            (
                page.title.clone(),
                page.slug.clone(),
                page.space_id.clone(),
                page.version.clone(),
                word_count,
            ),
        );
        let mut freq: HashMap<String, u32> = HashMap::new();
        for term in terms {
            *freq.entry(term).or_insert(0) += 1;
        }
        for (term, count) in freq {
            self.inverted
                .entry(term)
                .or_default()
                .push((page.id.clone(), count));
        }
    }

    pub fn remove_page(&mut self, page_id: &str) {
        self.pages.remove(page_id);
        for entries in self.inverted.values_mut() {
            entries.retain(|(id, _)| id != page_id);
        }
    }

    pub fn search(
        &self,
        query: &str,
        space_id: Option<&str>,
        version: Option<&str>,
        limit: usize,
    ) -> Vec<SearchResult> {
        let terms = tokenize(query);
        if terms.is_empty() {
            return vec![];
        }

        // TF-IDF style scoring
        let mut scores: HashMap<String, f32> = HashMap::new();
        let total_docs = self.pages.len() as f32;

        for term in &terms {
            if let Some(postings) = self.inverted.get(term) {
                let idf = (total_docs / (postings.len() as f32 + 1.0)).ln() + 1.0;
                for (page_id, freq) in postings {
                    if let Some((_, _, sid, ver, word_count)) = self.pages.get(page_id) {
                        if let Some(filter_space) = space_id {
                            if sid != filter_space {
                                continue;
                            }
                        }
                        if let Some(filter_ver) = version {
                            if ver != filter_ver {
                                continue;
                            }
                        }
                        let tf = *freq as f32 / (*word_count as f32 + 1.0);
                        *scores.entry(page_id.clone()).or_insert(0.0) += tf * idf;
                    }
                }
            }
        }

        let mut ranked: Vec<(String, f32)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(limit);

        ranked
            .into_iter()
            .filter_map(|(page_id, score)| {
                let (title, slug, sid, ver, _) = self.pages.get(&page_id)?;
                Some(SearchResult {
                    page_id: page_id.clone(),
                    space_id: sid.clone(),
                    title: title.clone(),
                    slug: slug.clone(),
                    excerpt: format!("Match for \"{}\"", query),
                    score,
                    version: ver.clone(),
                })
            })
            .collect()
    }

    pub fn clear_space(&mut self, space_id: &str) {
        let page_ids: Vec<String> = self
            .pages
            .iter()
            .filter(|(_, (_, _, sid, _, _))| sid == space_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in page_ids {
            self.remove_page(&id);
        }
    }
}

impl Default for SearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_page(space_id: &str, id: &str, slug: &str, title: &str, content: &str) -> Page {
        Page {
            id: id.to_string(),
            space_id: space_id.to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            markdown_content: content.to_string(),
            html_content: None,
            group_id: None,
            parent_id: None,
            order: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: "main".to_string(),
            metadata: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn search_basic() {
        let mut index = SearchIndex::new();
        let p1 = make_page("s1", "p1", "intro", "Introduction", "Welcome to the platform");
        let p2 = make_page(
            "s1",
            "p2",
            "guide",
            "User Guide",
            "Rust programming language basics",
        );
        let p3 = make_page(
            "s1",
            "p3",
            "ref",
            "Reference",
            "API reference documentation here",
        );
        index.index_page(&p1);
        index.index_page(&p2);
        index.index_page(&p3);

        let results = index.search("rust", None, None, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].page_id, "p2");
    }

    #[test]
    fn search_with_space_filter() {
        let mut index = SearchIndex::new();
        let p1 = make_page("space-a", "p1", "doc1", "Doc 1", "Kubernetes deployment guide");
        let p2 = make_page("space-b", "p2", "doc2", "Doc 2", "Kubernetes cluster setup");
        index.index_page(&p1);
        index.index_page(&p2);

        let results = index.search("kubernetes", Some("space-a"), None, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].space_id, "space-a");

        let all = index.search("kubernetes", None, None, 10);
        assert_eq!(all.len(), 2);
    }
}
