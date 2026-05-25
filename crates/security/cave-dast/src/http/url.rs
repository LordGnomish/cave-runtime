// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: parosproxy/paros/network/HtmlParameter.java (URI handling)
//! Minimal URL parser sufficient for ZAP's needs (`scheme://host[:port][/path][?query][#fragment]`).
//! We don't pull a heavy URL crate in
//! because the scan engine only needs to split parts, normalize, and
//! resolve relative refs — not full IRI support.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedUrl {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

impl ParsedUrl {
    pub fn is_https(&self) -> bool {
        self.scheme.eq_ignore_ascii_case("https")
    }

    pub fn effective_port(&self) -> u16 {
        self.port
            .unwrap_or_else(|| if self.is_https() { 443 } else { 80 })
    }

    /// `scheme://host[:port]` — the origin (RFC 6454).
    pub fn origin(&self) -> String {
        match self.port {
            Some(p) => format!("{}://{}:{}", self.scheme, self.host, p),
            None => format!("{}://{}", self.scheme, self.host),
        }
    }

    pub fn render(&self) -> String {
        let mut s = self.origin();
        s.push_str(&self.path);
        if !self.query.is_empty() {
            s.push('?');
            s.push_str(&self.query);
        }
        if !self.fragment.is_empty() {
            s.push('#');
            s.push_str(&self.fragment);
        }
        s
    }
}

pub fn parse(input: &str) -> Option<ParsedUrl> {
    let scheme_end = input.find("://")?;
    let scheme = input[..scheme_end].to_string();
    let rest = &input[scheme_end + 3..];

    // Split off fragment first (only meaningful after path/query).
    let (rest, fragment) = match rest.find('#') {
        Some(i) => (&rest[..i], rest[i + 1..].to_string()),
        None => (rest, String::new()),
    };
    // Then query.
    let (rest, query) = match rest.find('?') {
        Some(i) => (&rest[..i], rest[i + 1..].to_string()),
        None => (rest, String::new()),
    };
    // Then path.
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h.to_string(), p.parse().ok())
        }
        _ => (authority.to_string(), None),
    };

    Some(ParsedUrl {
        scheme,
        host,
        port,
        path,
        query,
        fragment,
    })
}

/// Resolve a (possibly relative) href against a base URL (RFC 3986 §5).
pub fn resolve(base: &ParsedUrl, href: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    if href.starts_with("//") {
        // Network-path reference.
        return Some(format!("{}:{}", base.scheme, href));
    }
    if let Some(stripped) = href.strip_prefix('/') {
        return Some(format!("{}/{}", base.origin(), stripped));
    }
    if href.starts_with('#') || href.is_empty() {
        return None; // Same-document — ignore for spider purposes.
    }
    // Relative — drop last path segment from base.
    let mut path = base.path.clone();
    if let Some(i) = path.rfind('/') {
        path.truncate(i + 1);
    } else {
        path = "/".to_string();
    }
    path.push_str(href);
    let combined = format!("{}{}", base.origin(), path);
    Some(combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_path_query() {
        let u = parse("https://example.com:8443/api/v1?token=abc#frag").unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, Some(8443));
        assert_eq!(u.path, "/api/v1");
        assert_eq!(u.query, "token=abc");
        assert_eq!(u.fragment, "frag");
        assert!(u.is_https());
    }

    #[test]
    fn parse_default_port_inference() {
        let u = parse("https://x.test/").unwrap();
        assert_eq!(u.effective_port(), 443);
        let u = parse("http://x.test/").unwrap();
        assert_eq!(u.effective_port(), 80);
    }

    #[test]
    fn parse_no_path() {
        let u = parse("https://api.example.com").unwrap();
        assert_eq!(u.path, "/");
        assert_eq!(u.query, "");
    }

    #[test]
    fn parse_rejects_missing_scheme() {
        assert!(parse("example.com/foo").is_none());
    }

    #[test]
    fn origin_omits_default_port() {
        let u = parse("http://x.test/foo").unwrap();
        assert_eq!(u.origin(), "http://x.test");
    }

    #[test]
    fn render_roundtrip() {
        let s = "https://x.test:444/a/b?c=d#e";
        let u = parse(s).unwrap();
        assert_eq!(u.render(), s);
    }

    #[test]
    fn resolve_absolute_passes_through() {
        let base = parse("http://x.test/").unwrap();
        assert_eq!(
            resolve(&base, "https://other.test/x").as_deref(),
            Some("https://other.test/x")
        );
    }

    #[test]
    fn resolve_root_path() {
        let base = parse("http://x.test/old").unwrap();
        assert_eq!(resolve(&base, "/new").as_deref(), Some("http://x.test/new"));
    }

    #[test]
    fn resolve_relative_path() {
        let base = parse("http://x.test/a/b/c").unwrap();
        assert_eq!(resolve(&base, "d").as_deref(), Some("http://x.test/a/b/d"));
    }

    #[test]
    fn resolve_network_path() {
        let base = parse("https://x.test/").unwrap();
        assert_eq!(
            resolve(&base, "//cdn.test/asset.js").as_deref(),
            Some("https://cdn.test/asset.js")
        );
    }

    #[test]
    fn resolve_fragment_ignored() {
        let base = parse("http://x.test/").unwrap();
        assert!(resolve(&base, "#section").is_none());
    }
}
