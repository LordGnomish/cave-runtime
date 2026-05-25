// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/spider/Spider.java
//   zap/src/main/java/org/zaproxy/zap/spider/SpiderParam.java
//
//! ZAP-style web spider — BFS link discovery from one or more seed
//! URLs, respecting context include/exclude patterns, a `robots.txt`
//! disallow list, and a maximum depth.

use std::collections::{HashSet, VecDeque};

use crate::context::Context;
use crate::http::url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiderConfig {
    pub max_depth: u32,
    pub max_urls: usize,
    pub respect_robots_txt: bool,
}

impl Default for SpiderConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_urls: 1000,
            respect_robots_txt: true,
        }
    }
}

/// One discovered URL plus the depth it was found at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub url: String,
    pub depth: u32,
}

pub struct Spider<'c> {
    cfg: SpiderConfig,
    context: &'c Context,
    disallow: Vec<String>,
}

impl<'c> Spider<'c> {
    pub fn new(cfg: SpiderConfig, context: &'c Context) -> Self {
        Self {
            cfg,
            context,
            disallow: Vec::new(),
        }
    }

    /// Feed a parsed `robots.txt` body. Only `User-agent: *` rules are
    /// considered (ZAP's default behaviour).
    pub fn set_robots_txt(&mut self, body: &str) {
        if !self.cfg.respect_robots_txt {
            return;
        }
        self.disallow = parse_robots_disallow(body, "*");
    }

    /// Crawl using a caller-supplied fetcher closure. The fetcher takes
    /// a URL and returns the response body — link extraction happens
    /// inside the spider. Returns every in-scope, non-disallowed,
    /// within-depth URL discovered.
    pub fn crawl<F>(&self, seeds: &[&str], mut fetch: F) -> Vec<Discovered>
    where
        F: FnMut(&str) -> String,
    {
        let mut visited: HashSet<String> = HashSet::new();
        let mut found: Vec<Discovered> = Vec::new();
        let mut queue: VecDeque<(String, u32)> = VecDeque::new();
        for s in seeds {
            queue.push_back((s.to_string(), 0));
        }
        while let Some((cur, depth)) = queue.pop_front() {
            if visited.contains(&cur) {
                continue;
            }
            visited.insert(cur.clone());
            if !self.context.is_in_scope(&cur) || self.is_disallowed(&cur) {
                continue;
            }
            found.push(Discovered {
                url: cur.clone(),
                depth,
            });
            if found.len() >= self.cfg.max_urls {
                break;
            }
            if depth >= self.cfg.max_depth {
                continue;
            }
            let body = fetch(&cur);
            let base = url::parse(&cur);
            for href in extract_hrefs(&body) {
                let resolved = match &base {
                    Some(b) => url::resolve(b, &href),
                    None => Some(href.clone()),
                };
                if let Some(r) = resolved {
                    if !visited.contains(&r) {
                        queue.push_back((r, depth + 1));
                    }
                }
            }
        }
        found
    }

    fn is_disallowed(&self, candidate: &str) -> bool {
        if !self.cfg.respect_robots_txt || self.disallow.is_empty() {
            return false;
        }
        let path = url::parse(candidate)
            .map(|u| u.path)
            .unwrap_or_else(|| candidate.to_string());
        self.disallow.iter().any(|d| path.starts_with(d))
    }
}

/// Extract `href`/`src` URL attributes from raw HTML. Cheap regex-free
/// scanner — sufficient for spider link discovery.
pub fn extract_hrefs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0;
    for marker in ["href=", "src="] {
        let mut c = cursor;
        while let Some(i) = lower[c..].find(marker) {
            let abs = c + i + marker.len();
            if abs >= html.len() {
                break;
            }
            let quote = html.as_bytes()[abs];
            let (s, after) = match quote {
                b'"' | b'\'' => {
                    let q = quote as char;
                    let rest = &html[abs + 1..];
                    if let Some(end) = rest.find(q) {
                        (rest[..end].to_string(), abs + 1 + end + 1)
                    } else {
                        break;
                    }
                }
                _ => {
                    let rest = &html[abs..];
                    let end = rest
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(rest.len());
                    (rest[..end].to_string(), abs + end)
                }
            };
            if !s.is_empty() {
                out.push(s);
            }
            c = after;
        }
        cursor = 0;
    }
    out
}

/// Parse `robots.txt`, returning Disallow paths for the given UA.
pub fn parse_robots_disallow(body: &str, ua: &str) -> Vec<String> {
    let mut disallow = Vec::new();
    let mut current_matches = false;
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (k, v) = match line.split_once(':') {
            Some(p) => p,
            None => continue,
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        match key.as_str() {
            "user-agent" => current_matches = val == ua,
            "disallow" if current_matches && !val.is_empty() => {
                disallow.push(val.to_string());
            }
            _ => {}
        }
    }
    disallow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;

    #[test]
    fn extract_hrefs_double_and_single_quotes() {
        let html = r#"<a href="/a">A</a><a href='/b'>B</a><script src="/c.js"></script>"#;
        let mut hrefs = extract_hrefs(html);
        hrefs.sort();
        assert_eq!(hrefs, vec!["/a", "/b", "/c.js"]);
    }

    #[test]
    fn extract_hrefs_unquoted() {
        let html = "<a href=/d>D</a>";
        let hrefs = extract_hrefs(html);
        assert_eq!(hrefs, vec!["/d"]);
    }

    #[test]
    fn robots_disallow_simple() {
        let body = "User-agent: *\nDisallow: /admin/\nDisallow: /api/private\n";
        let rules = parse_robots_disallow(body, "*");
        assert_eq!(rules, vec!["/admin/", "/api/private"]);
    }

    #[test]
    fn robots_only_matching_ua() {
        let body =
            "User-agent: Googlebot\nDisallow: /no-google\nUser-agent: *\nDisallow: /everyone\n";
        let rules = parse_robots_disallow(body, "*");
        assert_eq!(rules, vec!["/everyone"]);
    }

    #[test]
    fn crawl_bfs_within_depth_and_scope() {
        let mut ctx = Context::new("c");
        ctx.include(r"^http://x\.test/").unwrap();
        let cfg = SpiderConfig {
            max_depth: 2,
            max_urls: 100,
            respect_robots_txt: false,
        };
        let spider = Spider::new(cfg, &ctx);
        let pages = |u: &str| match u {
            "http://x.test/" => r#"<a href="/a">A</a><a href="/b">B</a>"#.to_string(),
            "http://x.test/a" => r#"<a href="/c">C</a>"#.to_string(),
            _ => "".to_string(),
        };
        let found = spider.crawl(&["http://x.test/"], pages);
        let urls: Vec<_> = found.iter().map(|d| d.url.clone()).collect();
        assert!(urls.contains(&"http://x.test/".to_string()));
        assert!(urls.contains(&"http://x.test/a".to_string()));
        assert!(urls.contains(&"http://x.test/b".to_string()));
        assert!(urls.contains(&"http://x.test/c".to_string()));
    }

    #[test]
    fn crawl_obeys_max_depth() {
        let ctx = Context::new("c");
        let cfg = SpiderConfig {
            max_depth: 1,
            max_urls: 100,
            respect_robots_txt: false,
        };
        let spider = Spider::new(cfg, &ctx);
        let pages = |u: &str| match u {
            "http://x.test/" => r#"<a href="/a">A</a>"#.to_string(),
            "http://x.test/a" => r#"<a href="/b">B</a>"#.to_string(),
            _ => "".to_string(),
        };
        let found = spider.crawl(&["http://x.test/"], pages);
        let depths: Vec<_> = found.iter().map(|d| d.depth).collect();
        assert!(depths.iter().all(|d| *d <= 1));
    }

    #[test]
    fn crawl_obeys_max_urls() {
        let ctx = Context::new("c");
        let cfg = SpiderConfig {
            max_depth: 10,
            max_urls: 2,
            respect_robots_txt: false,
        };
        let spider = Spider::new(cfg, &ctx);
        let pages = |_: &str| r#"<a href="/a">A</a><a href="/b">B</a>"#.to_string();
        let found = spider.crawl(&["http://x.test/"], pages);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn crawl_respects_robots_disallow() {
        let ctx = Context::new("c");
        let cfg = SpiderConfig::default();
        let mut spider = Spider::new(cfg, &ctx);
        spider.set_robots_txt("User-agent: *\nDisallow: /admin/\n");
        let pages = |u: &str| match u {
            "http://x.test/" => {
                r#"<a href="/admin/secret">A</a><a href="/public">P</a>"#.to_string()
            }
            _ => "".to_string(),
        };
        let found = spider.crawl(&["http://x.test/"], pages);
        let urls: Vec<_> = found.iter().map(|d| d.url.clone()).collect();
        assert!(urls.contains(&"http://x.test/public".to_string()));
        assert!(!urls.contains(&"http://x.test/admin/secret".to_string()));
    }
}
