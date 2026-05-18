// SPDX-License-Identifier: AGPL-3.0-or-later
//! Path → page router.
//!
//! Routes are registered as patterns: literal segments and `:name`
//! placeholders. The router returns the matched [`Page`] plus the captured
//! parameters.
//!
//! Matching is greedy left-to-right; the first registered route that matches
//! wins. Plugins should register more-specific routes (more literal segments)
//! before less-specific ones.

use crate::page::Page;

#[derive(Debug, Clone)]
pub struct Route {
    pub pattern: String,
    pub segments: Vec<Segment>,
    pub page: Page,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Literal(String),
    Param(String),
}

#[derive(Debug, Clone)]
pub struct RouteMatch<'a> {
    pub page: &'a Page,
    pub params: Vec<(String, String)>,
}

#[derive(Debug, Default)]
pub struct Router {
    routes: Vec<Route>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, page: Page) -> &mut Self {
        let segments = parse_pattern(&page.path);
        self.routes.push(Route { pattern: page.path.clone(), segments, page });
        self
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    pub fn r#match(&self, path: &str) -> Option<RouteMatch<'_>> {
        let request_segments: Vec<&str> = split_path(path);
        for route in &self.routes {
            if let Some(params) = match_segments(&route.segments, &request_segments) {
                return Some(RouteMatch { page: &route.page, params });
            }
        }
        None
    }
}

fn split_path(path: &str) -> Vec<&str> {
    path.trim_matches('/').split('/').filter(|s| !s.is_empty()).collect()
}

pub(crate) fn parse_pattern(path: &str) -> Vec<Segment> {
    split_path(path)
        .into_iter()
        .map(|seg| {
            if let Some(name) = seg.strip_prefix(':') {
                Segment::Param(name.to_string())
            } else {
                Segment::Literal(seg.to_string())
            }
        })
        .collect()
}

fn match_segments(pattern: &[Segment], request: &[&str]) -> Option<Vec<(String, String)>> {
    if pattern.len() != request.len() {
        return None;
    }
    let mut params = Vec::new();
    for (p, r) in pattern.iter().zip(request.iter()) {
        match p {
            Segment::Literal(s) => {
                if s != r {
                    return None;
                }
            }
            Segment::Param(name) => {
                params.push((name.clone(), (*r).to_string()));
            }
        }
    }
    Some(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::Scope;

    fn page(path: &str) -> Page {
        Page::builder(path, path).scope(Scope::Public).build()
    }

    #[test]
    fn router_starts_empty() {
        let r = Router::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn router_register_increases_len() {
        let mut r = Router::new();
        r.register(page("/a"));
        assert_eq!(r.len(), 1);
        r.register(page("/b"));
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn parse_pattern_literal_segments() {
        let segs = parse_pattern("/users/all");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], Segment::Literal("users".into()));
        assert_eq!(segs[1], Segment::Literal("all".into()));
    }

    #[test]
    fn parse_pattern_extracts_params() {
        let segs = parse_pattern("/users/:id");
        assert_eq!(segs[1], Segment::Param("id".into()));
    }

    #[test]
    fn parse_pattern_handles_leading_and_trailing_slash() {
        let a = parse_pattern("/users/");
        let b = parse_pattern("users");
        assert_eq!(a, b);
    }

    #[test]
    fn parse_pattern_empty_path_yields_no_segments() {
        assert!(parse_pattern("/").is_empty());
        assert!(parse_pattern("").is_empty());
    }

    #[test]
    fn router_match_exact_literal() {
        let mut r = Router::new();
        r.register(page("/a"));
        let m = r.r#match("/a").unwrap();
        assert_eq!(m.page.path, "/a");
        assert!(m.params.is_empty());
    }

    #[test]
    fn router_match_returns_none_for_unknown_path() {
        let mut r = Router::new();
        r.register(page("/a"));
        assert!(r.r#match("/b").is_none());
    }

    #[test]
    fn router_match_captures_param() {
        let mut r = Router::new();
        r.register(page("/users/:id"));
        let m = r.r#match("/users/42").unwrap();
        assert_eq!(m.params, vec![("id".to_string(), "42".to_string())]);
    }

    #[test]
    fn router_match_captures_multiple_params() {
        let mut r = Router::new();
        r.register(page("/tenants/:tid/users/:uid"));
        let m = r.r#match("/tenants/acme/users/7").unwrap();
        assert_eq!(m.params.len(), 2);
        assert_eq!(m.params[0], ("tid".into(), "acme".into()));
        assert_eq!(m.params[1], ("uid".into(), "7".into()));
    }

    #[test]
    fn router_match_does_not_match_different_segment_count() {
        let mut r = Router::new();
        r.register(page("/users/:id"));
        assert!(r.r#match("/users").is_none());
        assert!(r.r#match("/users/1/extra").is_none());
    }

    #[test]
    fn router_match_first_registered_wins() {
        let mut r = Router::new();
        r.register(Page::builder("a", "/x").title("first").scope(Scope::Public).build());
        r.register(Page::builder("b", "/x").title("second").scope(Scope::Public).build());
        let m = r.r#match("/x").unwrap();
        assert_eq!(m.page.title, "first");
    }

    #[test]
    fn router_match_handles_trailing_slash_in_request() {
        let mut r = Router::new();
        r.register(page("/a"));
        assert!(r.r#match("/a/").is_some());
    }

    #[test]
    fn router_match_handles_root_path() {
        let mut r = Router::new();
        r.register(page("/"));
        let m = r.r#match("/");
        assert!(m.is_some());
    }

    #[test]
    fn router_routes_returns_registered() {
        let mut r = Router::new();
        r.register(page("/a"));
        r.register(page("/b"));
        let paths: Vec<&str> = r.routes().iter().map(|r| r.pattern.as_str()).collect();
        assert_eq!(paths, vec!["/a", "/b"]);
    }

    #[test]
    fn router_match_literal_takes_priority_over_param_when_registered_first() {
        // Real Backstage-like behaviour: order matters.
        let mut r = Router::new();
        r.register(Page::builder("specific", "/users/me").scope(Scope::Public).build());
        r.register(Page::builder("generic", "/users/:id").scope(Scope::Public).build());
        let m = r.r#match("/users/me").unwrap();
        assert_eq!(m.page.id, "specific");
    }

    #[test]
    fn router_match_falls_through_to_param_when_no_literal() {
        let mut r = Router::new();
        r.register(Page::builder("specific", "/users/me").scope(Scope::Public).build());
        r.register(Page::builder("generic", "/users/:id").scope(Scope::Public).build());
        let m = r.r#match("/users/42").unwrap();
        assert_eq!(m.page.id, "generic");
    }

    #[test]
    fn router_match_tolerates_double_slashes() {
        let mut r = Router::new();
        r.register(page("/a/b"));
        assert!(r.r#match("/a//b").is_some());
    }
}
