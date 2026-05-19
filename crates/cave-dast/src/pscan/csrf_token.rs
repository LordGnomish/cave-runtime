// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zap-extensions@addOns/pscanrules/.../CsrfCountermeasuresScanRule.java
//
//! CSRF-token absence passive rule. Flags HTML forms whose `method` is
//! a state-changing verb (POST/PUT/DELETE) and that don't carry a
//! recognisable CSRF-token hidden field.

use super::{PassiveScanRule, PluginId};
use crate::alert::Alert;
use crate::http::{HttpRequest, HttpResponse};
use crate::models::RiskLevel;

pub struct CsrfTokenAbsenceRule;

const TOKEN_NAMES: &[&str] = &[
    "csrf",
    "csrf_token",
    "csrftoken",
    "_csrf",
    "_csrf_token",
    "authenticity_token",
    "__RequestVerificationToken",
    "xsrf",
    "anti-csrf-token",
    "anticsrf",
];

impl PassiveScanRule for CsrfTokenAbsenceRule {
    fn id(&self) -> PluginId {
        10202
    }
    fn name(&self) -> &'static str {
        "Absence of Anti-CSRF Token"
    }
    fn risk(&self) -> RiskLevel {
        RiskLevel::Medium
    }
    fn cwe_id(&self) -> u32 {
        352
    }

    fn scan(&self, _req: &HttpRequest, resp: &HttpResponse) -> Vec<Alert> {
        if !resp.is_html() {
            return Vec::new();
        }
        let body = resp.body_str().unwrap_or("");
        let mut alerts = Vec::new();
        for (idx, form) in find_forms(body).iter().enumerate() {
            let method_state_changing =
                form.method.eq_ignore_ascii_case("post")
                    || form.method.eq_ignore_ascii_case("put")
                    || form.method.eq_ignore_ascii_case("delete");
            if !method_state_changing {
                continue;
            }
            let has_token = TOKEN_NAMES.iter().any(|name| {
                form.body
                    .to_ascii_lowercase()
                    .contains(&name.to_ascii_lowercase())
            });
            if !has_token {
                alerts.push(Alert {
                    name: self.name().to_string(),
                    risk: self.risk(),
                    cwe_id: self.cwe_id(),
                    url: String::new(),
                    description: format!(
                        "Form #{} uses {} but does not include a recognisable CSRF token.",
                        idx + 1,
                        form.method.to_ascii_uppercase()
                    ),
                    solution:
                        "Include a per-session CSRF token in every state-changing form (synchronizer-token pattern)."
                            .to_string(),
                    evidence: Some(format!("<form method=\"{}\">", form.method)),
                    plugin_id: self.id(),
                });
            }
        }
        alerts
    }
}

struct ParsedForm {
    method: String,
    body: String,
}

fn find_forms(html: &str) -> Vec<ParsedForm> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(open) = lower[cursor..].find("<form") {
        let abs_open = cursor + open;
        let tag_end = lower[abs_open..]
            .find('>')
            .map(|i| abs_open + i + 1)
            .unwrap_or(lower.len());
        let close = lower[tag_end..]
            .find("</form>")
            .map(|i| tag_end + i)
            .unwrap_or(lower.len());

        let tag = &html[abs_open..tag_end];
        let method = extract_attr(tag, "method").unwrap_or_else(|| "get".to_string());
        let body = html[tag_end..close].to_string();
        out.push(ParsedForm { method, body });
        cursor = close + "</form>".len();
        if cursor >= html.len() {
            break;
        }
    }
    out
}

fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{}=", name);
    let i = lower.find(&needle)?;
    let after = &tag[i + needle.len()..];
    let val = match after.chars().next() {
        Some('"') => after[1..].split('"').next()?,
        Some('\'') => after[1..].split('\'').next()?,
        _ => after.split(|c: char| c.is_whitespace() || c == '>').next()?,
    };
    Some(val.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpMethod;

    fn html(body: &str) -> HttpResponse {
        let mut r = HttpResponse::new(200, "OK");
        r.headers.insert("Content-Type", "text/html");
        r.body = body.as_bytes().to_vec();
        r
    }

    #[test]
    fn flags_post_form_without_token() {
        let r = CsrfTokenAbsenceRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html(r#"<form method="POST" action="/x"><input name="u"></form>"#);
        let alerts = r.scan(&req, &resp);
        assert_eq!(alerts.len(), 1);
    }

    #[test]
    fn skip_get_form() {
        let r = CsrfTokenAbsenceRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html(r#"<form method="get" action="/q"><input name="q"></form>"#);
        assert!(r.scan(&req, &resp).is_empty());
    }

    #[test]
    fn recognises_csrf_token_field() {
        let r = CsrfTokenAbsenceRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html(
            r#"<form method="POST"><input name="csrf_token" value="abc"><input name="u"></form>"#,
        );
        assert!(r.scan(&req, &resp).is_empty());
    }

    #[test]
    fn extract_attr_handles_quotes() {
        let v = extract_attr(r#"<form method="POST" action='/x'>"#, "method");
        assert_eq!(v.as_deref(), Some("POST"));
        let v = extract_attr(r#"<form method='POST'>"#, "method");
        assert_eq!(v.as_deref(), Some("POST"));
        let v = extract_attr(r#"<form method=POST>"#, "method");
        assert_eq!(v.as_deref(), Some("POST"));
    }

    #[test]
    fn finds_multiple_forms() {
        let r = CsrfTokenAbsenceRule;
        let req = HttpRequest::new(HttpMethod::Get, "http://x/");
        let resp = html(
            r#"<form method="post">a</form><form method="put">b</form><form method="get">c</form>"#,
        );
        let alerts = r.scan(&req, &resp);
        assert_eq!(alerts.len(), 2);
    }
}
