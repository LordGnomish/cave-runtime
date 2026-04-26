//! TechDocs plugin — per-service docs index + search.
//!
//! Each service registers a doc tree (markdown pages keyed by path). The
//! portal renders the tree in a sidebar, the page in the main area, and
//! supports text-substring search across all pages of a tenant.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocPage {
    pub service: String,
    pub tenant: String,
    pub path: String,
    pub title: String,
    pub body: String,
    pub updated_at: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TechDocsError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("page not found")]
    NotFound,
    #[error("body too large: {0} bytes (max {1})")]
    TooLarge(usize, usize),
}

const MAX_DOC_BYTES: usize = 1_000_000;

fn validate_path(p: &str) -> Result<(), TechDocsError> {
    if p.is_empty() || p.starts_with('/') || p.contains("..") || p.contains("//") {
        return Err(TechDocsError::InvalidPath(p.into()));
    }
    if p.len() > 256 {
        return Err(TechDocsError::InvalidPath("too long".into()));
    }
    Ok(())
}

#[derive(Debug, Default)]
pub struct TechDocsPlugin {
    pages: Vec<DocPage>,
}

impl TechDocsPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, page: DocPage) -> Result<(), TechDocsError> {
        validate_path(&page.path)?;
        if page.body.len() > MAX_DOC_BYTES {
            return Err(TechDocsError::TooLarge(page.body.len(), MAX_DOC_BYTES));
        }
        if let Some(idx) = self.pages.iter().position(|x| {
            x.tenant == page.tenant && x.service == page.service && x.path == page.path
        }) {
            self.pages[idx] = page;
        } else {
            self.pages.push(page);
        }
        Ok(())
    }

    pub fn get(&self, tenant: &str, service: &str, path: &str) -> Result<&DocPage, TechDocsError> {
        self.pages
            .iter()
            .find(|p| p.tenant == tenant && p.service == service && p.path == path)
            .ok_or(TechDocsError::NotFound)
    }

    pub fn tree(&self, tenant: &str, service: &str) -> Vec<&DocPage> {
        let mut out: Vec<&DocPage> = self
            .pages
            .iter()
            .filter(|p| p.tenant == tenant && p.service == service)
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    pub fn search(&self, tenant: &str, needle: &str, limit: usize) -> Vec<&DocPage> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle_lc = needle.to_lowercase();
        let mut out: Vec<&DocPage> = self
            .pages
            .iter()
            .filter(|p| p.tenant == tenant)
            .filter(|p| {
                p.title.to_lowercase().contains(&needle_lc)
                    || p.body.to_lowercase().contains(&needle_lc)
                    || p.path.to_lowercase().contains(&needle_lc)
            })
            .take(limit)
            .collect();
        out.sort_by(|a, b| a.title.cmp(&b.title));
        out
    }

    pub fn count(&self) -> usize {
        self.pages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(tenant: &str, svc: &str, path: &str, title: &str, body: &str) -> DocPage {
        DocPage {
            tenant: tenant.into(),
            service: svc.into(),
            path: path.into(),
            title: title.into(),
            body: body.into(),
            updated_at: "1970-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn put_inserts() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "intro.md", "Intro", "hello")).unwrap();
        assert_eq!(t.count(), 1);
    }

    #[test]
    fn put_replaces_same_key() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "intro.md", "Old", "x")).unwrap();
        t.put(page("acme", "web", "intro.md", "New", "y")).unwrap();
        assert_eq!(t.count(), 1);
        assert_eq!(t.get("acme", "web", "intro.md").unwrap().title, "New");
    }

    #[test]
    fn put_invalid_path_empty() {
        let mut t = TechDocsPlugin::new();
        let err = t.put(page("acme", "web", "", "T", "b")).unwrap_err();
        assert!(matches!(err, TechDocsError::InvalidPath(_)));
    }

    #[test]
    fn put_invalid_path_traversal() {
        let mut t = TechDocsPlugin::new();
        let err = t.put(page("acme", "web", "../etc", "T", "b")).unwrap_err();
        assert!(matches!(err, TechDocsError::InvalidPath(_)));
    }

    #[test]
    fn put_invalid_path_double_slash() {
        let mut t = TechDocsPlugin::new();
        let err = t.put(page("acme", "web", "a//b", "T", "b")).unwrap_err();
        assert!(matches!(err, TechDocsError::InvalidPath(_)));
    }

    #[test]
    fn put_invalid_path_leading_slash() {
        let mut t = TechDocsPlugin::new();
        let err = t.put(page("acme", "web", "/abs", "T", "b")).unwrap_err();
        assert!(matches!(err, TechDocsError::InvalidPath(_)));
    }

    #[test]
    fn put_too_large_rejected() {
        let mut t = TechDocsPlugin::new();
        let body = "x".repeat(MAX_DOC_BYTES + 1);
        let err = t.put(page("acme", "web", "x.md", "T", &body)).unwrap_err();
        assert!(matches!(err, TechDocsError::TooLarge(_, _)));
    }

    #[test]
    fn get_not_found() {
        let t = TechDocsPlugin::new();
        let err = t.get("acme", "web", "x.md").unwrap_err();
        assert_eq!(err, TechDocsError::NotFound);
    }

    #[test]
    fn tree_sorted_by_path() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "z.md", "Z", "")).unwrap();
        t.put(page("acme", "web", "a.md", "A", "")).unwrap();
        t.put(page("acme", "web", "m.md", "M", "")).unwrap();
        let paths: Vec<&str> = t.tree("acme", "web").iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, vec!["a.md", "m.md", "z.md"]);
    }

    #[test]
    fn tree_filters_service_and_tenant() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "a.md", "A", "")).unwrap();
        t.put(page("acme", "api", "b.md", "B", "")).unwrap();
        t.put(page("globex", "web", "c.md", "C", "")).unwrap();
        assert_eq!(t.tree("acme", "web").len(), 1);
        assert_eq!(t.tree("acme", "api").len(), 1);
        assert_eq!(t.tree("globex", "web").len(), 1);
    }

    #[test]
    fn search_finds_in_title() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "a.md", "Authentication", "body")).unwrap();
        let out = t.search("acme", "auth", 10);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn search_finds_in_body() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "a.md", "T", "OAuth flow notes")).unwrap();
        let out = t.search("acme", "oauth", 10);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn search_case_insensitive() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "a.md", "RBAC Guide", "")).unwrap();
        assert_eq!(t.search("acme", "rbac", 10).len(), 1);
        assert_eq!(t.search("acme", "RBAC", 10).len(), 1);
        assert_eq!(t.search("acme", "Rbac", 10).len(), 1);
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "a.md", "T", "")).unwrap();
        assert!(t.search("acme", "", 10).is_empty());
    }

    #[test]
    fn search_respects_limit() {
        let mut t = TechDocsPlugin::new();
        for i in 0..10 {
            t.put(page("acme", "web", &format!("doc{i}.md"), &format!("Title {i}"), "auth")).unwrap();
        }
        let out = t.search("acme", "auth", 3);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn search_filters_by_tenant() {
        let mut t = TechDocsPlugin::new();
        t.put(page("acme", "web", "a.md", "auth", "")).unwrap();
        t.put(page("globex", "web", "b.md", "auth", "")).unwrap();
        assert_eq!(t.search("acme", "auth", 10).len(), 1);
    }

    #[test]
    fn page_round_trips_json() {
        let p = page("a", "s", "p.md", "T", "body");
        let s = serde_json::to_string(&p).unwrap();
        let back: DocPage = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }
}
