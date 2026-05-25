// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/anticsrf/
//
//! Anti-CSRF token replay engine — parity with
//! `ExtensionAntiCSRF.java` (ZAP 2.14.0).
//!
//! Many sites embed a CSRF token in a hidden form input. To scan a
//! protected POST endpoint, ZAP first issues a GET to harvest the
//! token, then injects the harvested token into the test request.
//! This module covers token extraction (HTML form scanning), token
//! storage (named registry), and request rewriting.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AntiCsrfToken {
    pub name: String,
    pub value: String,
    /// URL where the token was originally observed — used to refresh
    /// before each request that needs it.
    pub source_url: String,
}

#[derive(Debug, Default)]
pub struct AntiCsrfRegistry {
    /// Token name → most-recently-seen value + source.
    tokens: HashMap<String, AntiCsrfToken>,
    /// Configured token names to look for during HTML harvest.
    known_names: Vec<String>,
}

impl AntiCsrfRegistry {
    pub fn new() -> Self {
        Self {
            tokens: HashMap::new(),
            known_names: default_token_names(),
        }
    }

    pub fn set_known_names(&mut self, names: Vec<String>) {
        self.known_names = names;
    }

    pub fn known_names(&self) -> &[String] {
        &self.known_names
    }

    pub fn register(&mut self, token: AntiCsrfToken) {
        self.tokens.insert(token.name.clone(), token);
    }

    pub fn get(&self, name: &str) -> Option<&AntiCsrfToken> {
        self.tokens.get(name)
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Scan an HTML body for `<input type="hidden" name="..." value="...">`
    /// tags whose name matches any configured known-name. Adds matches
    /// to the registry and returns the names found.
    pub fn harvest_from_html(&mut self, html: &str, source_url: &str) -> Vec<String> {
        let mut found = Vec::new();
        for tok in scan_hidden_inputs(html) {
            if self
                .known_names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&tok.0))
            {
                let acsr = AntiCsrfToken {
                    name: tok.0.clone(),
                    value: tok.1,
                    source_url: source_url.to_string(),
                };
                self.register(acsr);
                found.push(tok.0);
            }
        }
        found
    }

    /// Substitute the URL-encoded `name=value` of each registered
    /// token into a body that already contains `name=PLACEHOLDER`.
    /// Returns the rewritten body and how many substitutions occurred.
    pub fn rewrite_body(&self, body: &str) -> (String, usize) {
        let mut out = body.to_string();
        let mut count = 0;
        for tok in self.tokens.values() {
            let needle = format!("{}=", tok.name);
            if let Some(start) = out.find(&needle) {
                let after = start + needle.len();
                let end = out[after..]
                    .find('&')
                    .map(|i| after + i)
                    .unwrap_or(out.len());
                let replacement = format!("{}={}", tok.name, urlenc(&tok.value));
                out.replace_range(start..end, &replacement);
                count += 1;
            }
        }
        (out, count)
    }
}

fn default_token_names() -> Vec<String> {
    vec![
        "csrf_token".into(),
        "csrfmiddlewaretoken".into(),
        "__RequestVerificationToken".into(),
        "authenticity_token".into(),
        "_csrf".into(),
        "XSRF-TOKEN".into(),
    ]
}

/// Extract `(name, value)` pairs from each `<input type="hidden" …>`
/// tag in an HTML snippet. Tag/attribute parsing is intentionally
/// minimal — sufficient for the well-formed forms ZAP rule expects.
pub fn scan_hidden_inputs(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let lower = html.to_lowercase();
    let mut cursor = 0;
    while let Some(pos) = lower[cursor..].find("<input") {
        let start = cursor + pos;
        let end = html[start..]
            .find('>')
            .map(|i| start + i + 1)
            .unwrap_or(html.len());
        let tag = &html[start..end];
        if !tag.to_lowercase().contains("type=\"hidden\"")
            && !tag.to_lowercase().contains("type='hidden'")
            && !tag.to_lowercase().contains("type=hidden")
        {
            cursor = end;
            continue;
        }
        let name = attr_value(tag, "name");
        let value = attr_value(tag, "value");
        if let (Some(n), Some(v)) = (name, value) {
            out.push((n, v));
        }
        cursor = end;
    }
    out
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let needle = format!("{}=", attr.to_lowercase());
    let pos = lower.find(&needle)?;
    let rest = &tag[pos + needle.len()..];
    let bytes = rest.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (start, quote) = match bytes[0] {
        b'"' => (1, b'"'),
        b'\'' => (1, b'\''),
        _ => (0, b' '),
    };
    let mut end = start;
    while end < bytes.len() && bytes[end] != quote && (quote != b' ' || !bytes[end].is_ascii_whitespace())
    {
        if quote == b' ' && bytes[end] == b'>' {
            break;
        }
        end += 1;
    }
    Some(rest[start..end].to_string())
}

fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_include_django_and_dotnet() {
        let r = AntiCsrfRegistry::new();
        let names: Vec<&str> = r.known_names().iter().map(|s| s.as_str()).collect();
        assert!(names.contains(&"csrfmiddlewaretoken"));
        assert!(names.contains(&"__RequestVerificationToken"));
        assert!(names.contains(&"authenticity_token"));
    }

    #[test]
    fn harvest_extracts_django_token() {
        let html = r#"<form><input type="hidden" name="csrfmiddlewaretoken" value="abc123"></form>"#;
        let mut r = AntiCsrfRegistry::new();
        let found = r.harvest_from_html(html, "https://x/login");
        assert_eq!(found, vec!["csrfmiddlewaretoken".to_string()]);
        let stored = r.get("csrfmiddlewaretoken").unwrap();
        assert_eq!(stored.value, "abc123");
    }

    #[test]
    fn harvest_ignores_unknown_names() {
        let html = r#"<input type="hidden" name="not_a_csrf" value="ignored">"#;
        let mut r = AntiCsrfRegistry::new();
        let found = r.harvest_from_html(html, "https://x");
        assert!(found.is_empty());
    }

    #[test]
    fn harvest_ignores_non_hidden_inputs() {
        let html = r#"<input type="text" name="csrf_token" value="not-hidden">"#;
        let mut r = AntiCsrfRegistry::new();
        let found = r.harvest_from_html(html, "https://x");
        assert!(found.is_empty());
    }

    #[test]
    fn rewrite_body_replaces_placeholder() {
        let mut r = AntiCsrfRegistry::new();
        r.register(AntiCsrfToken {
            name: "csrf_token".into(),
            value: "new-value".into(),
            source_url: "https://x".into(),
        });
        let (out, n) = r.rewrite_body("a=1&csrf_token=PLACEHOLDER&b=2");
        assert_eq!(n, 1);
        assert!(out.contains("csrf_token=new-value"));
        assert!(out.contains("a=1"));
        assert!(out.contains("b=2"));
    }

    #[test]
    fn rewrite_body_url_encodes_special_chars() {
        let mut r = AntiCsrfRegistry::new();
        r.register(AntiCsrfToken {
            name: "_csrf".into(),
            value: "a b/c".into(),
            source_url: "https://x".into(),
        });
        let (out, _) = r.rewrite_body("_csrf=stale");
        assert!(out.contains("_csrf=a%20b%2Fc"));
    }

    #[test]
    fn rewrite_body_no_match_returns_zero() {
        let r = AntiCsrfRegistry::new();
        let (out, n) = r.rewrite_body("no=token&here=now");
        assert_eq!(n, 0);
        assert_eq!(out, "no=token&here=now");
    }

    #[test]
    fn case_insensitive_name_match() {
        let html = r#"<input type="hidden" name="XSRF-TOKEN" value="xt">"#;
        let mut r = AntiCsrfRegistry::new();
        let mut names = default_token_names();
        names.push("xsrf-token".into());
        r.set_known_names(names);
        let found = r.harvest_from_html(html, "https://x");
        assert!(!found.is_empty());
    }

    #[test]
    fn multiple_hidden_inputs_extracted() {
        let html = r#"<input type="hidden" name="csrf_token" value="t1">
                      <input type="hidden" name="authenticity_token" value="t2">
                      <input type="hidden" name="not_known" value="t3">"#;
        let mut r = AntiCsrfRegistry::new();
        let found = r.harvest_from_html(html, "https://x");
        assert_eq!(found.len(), 2);
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn scan_hidden_inputs_parses_attributes() {
        let html = r#"<input type="hidden" name="a" value="1">
                      <input name="b" value="2" type='hidden'>
                      <input type="text" name="c" value="3">"#;
        let inputs = scan_hidden_inputs(html);
        assert_eq!(inputs.len(), 2);
        assert!(inputs.contains(&("a".to_string(), "1".to_string())));
        assert!(inputs.contains(&("b".to_string(), "2".to_string())));
    }
}
