// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/network/HttpRequestBody.java
//   zap/src/main/java/org/zaproxy/zap/network/HttpResponseBody.java
//   parosproxy/paros/network/HttpMessage.java
//
//! HTTP request/response model — the core wire data structure threaded
//! through every scan rule, the spider, and the proxy. Mirrors ZAP's
//! `HttpMessage` (request header + body, response header + body).

pub mod parse;
pub mod url;

use std::collections::BTreeMap;
use std::fmt::Write;

/// HTTP method. ZAP supports the full RFC 7231 set plus WebDAV / CONNECT.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Patch,
    Connect,
    Trace,
    Other(String),
}

impl HttpMethod {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
            Self::Patch => "PATCH",
            Self::Connect => "CONNECT",
            Self::Trace => "TRACE",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "GET" => Self::Get,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            "HEAD" => Self::Head,
            "OPTIONS" => Self::Options,
            "PATCH" => Self::Patch,
            "CONNECT" => Self::Connect,
            "TRACE" => Self::Trace,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn is_safe(&self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Options | Self::Trace)
    }
}

/// Case-insensitive header map. ZAP stores headers as a Vec of pairs
/// to preserve duplicates (e.g. multiple `Set-Cookie`). We do the same
/// but expose lower-cased lookup helpers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderMap {
    pub pairs: Vec<(String, String)>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.pairs.push((name.into(), value.into()));
    }

    pub fn first(&self, name: &str) -> Option<&str> {
        let lname = name.to_ascii_lowercase();
        self.pairs
            .iter()
            .find(|(n, _)| n.to_ascii_lowercase() == lname)
            .map(|(_, v)| v.as_str())
    }

    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let lname = name.to_ascii_lowercase();
        self.pairs.iter().filter_map(move |(n, v)| {
            if n.to_ascii_lowercase() == lname {
                Some(v.as_str())
            } else {
                None
            }
        })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.first(name).is_some()
    }

    pub fn remove(&mut self, name: &str) {
        let lname = name.to_ascii_lowercase();
        self.pairs.retain(|(n, _)| n.to_ascii_lowercase() != lname);
    }

    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }
}

/// HTTP request. Mirrors ZAP's `HttpRequestHeader` + `HttpRequestBody`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub version: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn new(method: HttpMethod, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            version: "HTTP/1.1".to_string(),
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Parse `name=value&...` form bodies (RFC 1866 application/x-www-form-urlencoded).
    pub fn form_params(&self) -> BTreeMap<String, String> {
        parse_query(self.body_str().unwrap_or(""))
    }

    /// Query parameters from the URL.
    pub fn query_params(&self) -> BTreeMap<String, String> {
        let q = url::parse(&self.url).map(|u| u.query).unwrap_or_default();
        parse_query(&q)
    }

    /// Cookie key→value map from `Cookie:` header (RFC 6265 §5.4 parse).
    pub fn cookies(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for raw in self.headers.all("Cookie") {
            for crumb in raw.split(';') {
                if let Some((k, v)) = crumb.split_once('=') {
                    out.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        out
    }

    /// Re-render to wire format (used by the proxy and by recording).
    pub fn render(&self) -> String {
        let parsed = url::parse(&self.url);
        let path_and_query = match &parsed {
            Some(p) if !p.query.is_empty() => format!("{}?{}", p.path, p.query),
            Some(p) => p.path.clone(),
            None => self.url.clone(),
        };
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{} {} {}\r",
            self.method.as_str(),
            path_and_query,
            self.version
        );
        for (n, v) in &self.headers.pairs {
            let _ = writeln!(s, "{}: {}\r", n, v);
        }
        s.push_str("\r\n");
        if let Some(b) = self.body_str() {
            s.push_str(b);
        }
        s
    }
}

/// HTTP response. Mirrors ZAP's `HttpResponseHeader` + `HttpResponseBody`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub version: String,
    pub status: u16,
    pub reason: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, reason: impl Into<String>) -> Self {
        Self {
            version: "HTTP/1.1".to_string(),
            status,
            reason: reason.into(),
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Parse `Set-Cookie:` headers into key→full-attribute-string map.
    pub fn set_cookies(&self) -> Vec<SetCookie> {
        self.headers
            .all("Set-Cookie")
            .map(SetCookie::parse)
            .collect()
    }

    /// MIME type from `Content-Type:` minus parameters.
    pub fn content_type(&self) -> Option<&str> {
        self.headers
            .first("Content-Type")
            .and_then(|v| v.split(';').next())
            .map(|s| s.trim())
    }

    pub fn is_html(&self) -> bool {
        self.content_type()
            .map(|c| c.starts_with("text/html"))
            .unwrap_or(false)
    }
}

/// Parsed `Set-Cookie` attribute set (RFC 6265 §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetCookie {
    pub name: String,
    pub value: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub max_age: Option<i64>,
    pub expires: Option<String>,
}

impl SetCookie {
    pub fn parse(raw: &str) -> Self {
        let mut parts = raw.split(';');
        let first = parts.next().unwrap_or("");
        let (name, value) = first.split_once('=').unwrap_or((first, ""));
        let mut c = Self {
            name: name.trim().to_string(),
            value: value.trim().to_string(),
            ..Default::default()
        };
        for attr in parts {
            let attr = attr.trim();
            let lower = attr.to_ascii_lowercase();
            if lower == "secure" {
                c.secure = true;
            } else if lower == "httponly" {
                c.http_only = true;
            } else if let Some((k, v)) = attr.split_once('=') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "domain" => c.domain = Some(v.trim().to_string()),
                    "path" => c.path = Some(v.trim().to_string()),
                    "max-age" => c.max_age = v.trim().parse().ok(),
                    "expires" => c.expires = Some(v.trim().to_string()),
                    "samesite" => c.same_site = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }
        c
    }
}

/// Parse `a=b&c=d` form/query into a map.
pub fn parse_query(q: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for kv in q.split('&') {
        if kv.is_empty() {
            continue;
        }
        if let Some((k, v)) = kv.split_once('=') {
            out.insert(percent_decode(k), percent_decode(v));
        } else {
            out.insert(percent_decode(kv), String::new());
        }
    }
    out
}

/// Minimal percent-decoder (RFC 3986). ASCII-only; non-percent bytes pass through.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            match (hi, lo) {
                (Some(h), Some(l)) => {
                    out.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_roundtrip() {
        for m in ["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD", "CONNECT"] {
            assert_eq!(HttpMethod::parse(m).as_str(), m);
        }
        // Custom method preserved.
        assert_eq!(HttpMethod::parse("PROPFIND").as_str(), "PROPFIND");
    }

    #[test]
    fn method_safe_set() {
        assert!(HttpMethod::Get.is_safe());
        assert!(HttpMethod::Head.is_safe());
        assert!(!HttpMethod::Post.is_safe());
        assert!(!HttpMethod::Delete.is_safe());
    }

    #[test]
    fn header_case_insensitive_lookup() {
        let mut h = HeaderMap::new();
        h.insert("Content-Type", "text/html");
        h.insert("X-Frame-Options", "DENY");
        assert_eq!(h.first("content-type"), Some("text/html"));
        assert_eq!(h.first("X-FRAME-OPTIONS"), Some("DENY"));
    }

    #[test]
    fn header_duplicates_preserved() {
        let mut h = HeaderMap::new();
        h.insert("Set-Cookie", "a=1");
        h.insert("Set-Cookie", "b=2");
        let all: Vec<_> = h.all("Set-Cookie").collect();
        assert_eq!(all, vec!["a=1", "b=2"]);
    }

    #[test]
    fn cookie_parse_simple() {
        let c = SetCookie::parse("sid=abc123; Secure; HttpOnly; Path=/; SameSite=Lax");
        assert_eq!(c.name, "sid");
        assert_eq!(c.value, "abc123");
        assert!(c.secure);
        assert!(c.http_only);
        assert_eq!(c.path.as_deref(), Some("/"));
        assert_eq!(c.same_site.as_deref(), Some("Lax"));
    }

    #[test]
    fn cookie_parse_insecure() {
        let c = SetCookie::parse("trace=on");
        assert!(!c.secure);
        assert!(!c.http_only);
        assert!(c.same_site.is_none());
    }

    #[test]
    fn cookie_parse_max_age() {
        let c = SetCookie::parse("k=v; Max-Age=3600");
        assert_eq!(c.max_age, Some(3600));
    }

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%21%40%23"), "!@#");
    }

    #[test]
    fn percent_decode_invalid_passthrough() {
        // Truncated %X — leave as-is.
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[test]
    fn parse_query_basic() {
        let q = parse_query("a=1&b=2&c=");
        assert_eq!(q.get("a"), Some(&"1".to_string()));
        assert_eq!(q.get("b"), Some(&"2".to_string()));
        assert_eq!(q.get("c"), Some(&"".to_string()));
    }

    #[test]
    fn request_form_params() {
        let mut req = HttpRequest::new(HttpMethod::Post, "http://x.com/login");
        req.body = b"user=alice&pw=secret".to_vec();
        let f = req.form_params();
        assert_eq!(f.get("user"), Some(&"alice".to_string()));
        assert_eq!(f.get("pw"), Some(&"secret".to_string()));
    }

    #[test]
    fn request_query_params() {
        let req = HttpRequest::new(HttpMethod::Get, "http://x.com/search?q=cats&n=10");
        let q = req.query_params();
        assert_eq!(q.get("q"), Some(&"cats".to_string()));
        assert_eq!(q.get("n"), Some(&"10".to_string()));
    }

    #[test]
    fn request_cookies_parse() {
        let mut req = HttpRequest::new(HttpMethod::Get, "http://x.com/");
        req.headers.insert("Cookie", "sid=abc; theme=dark");
        let c = req.cookies();
        assert_eq!(c.get("sid"), Some(&"abc".to_string()));
        assert_eq!(c.get("theme"), Some(&"dark".to_string()));
    }

    #[test]
    fn request_render_wire_format() {
        let mut req = HttpRequest::new(HttpMethod::Get, "http://x.com/api?a=1");
        req.headers.insert("Host", "x.com");
        let s = req.render();
        assert!(s.starts_with("GET /api?a=1 HTTP/1.1\r\n"));
        assert!(s.contains("Host: x.com\r\n"));
    }

    #[test]
    fn response_content_type() {
        let mut r = HttpResponse::new(200, "OK");
        r.headers.insert("Content-Type", "text/html; charset=utf-8");
        assert_eq!(r.content_type(), Some("text/html"));
        assert!(r.is_html());
    }

    #[test]
    fn response_set_cookies_multi() {
        let mut r = HttpResponse::new(200, "OK");
        r.headers.insert("Set-Cookie", "a=1; Secure");
        r.headers.insert("Set-Cookie", "b=2; HttpOnly");
        let cookies = r.set_cookies();
        assert_eq!(cookies.len(), 2);
        assert!(cookies[0].secure);
        assert!(cookies[1].http_only);
    }
}
