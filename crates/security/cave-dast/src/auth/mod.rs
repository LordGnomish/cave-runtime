// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/authentication/AuthenticationMethodType.java
//   FormBasedAuthenticationMethodType.java + HttpAuthenticationMethodType.java
//
//! ZAP authentication strategies. We expose the two methods that matter
//! for headless API scans:
//!
//! * `FormBased` — POST a credential pair to a login URL, capture the
//!   session cookie from the response.
//! * `BearerToken` — pre-configured static bearer; injected as the
//!   `Authorization` header on every request.

use crate::http::{HeaderMap, HttpMethod, HttpRequest, HttpResponse, SetCookie};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    None,
    BearerToken(String),
    FormBased(FormAuthConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormAuthConfig {
    pub login_url: String,
    pub username_field: String,
    pub password_field: String,
    pub username: String,
    pub password: String,
    pub session_cookie_name: String,
}

impl FormAuthConfig {
    pub fn login_request(&self) -> HttpRequest {
        let mut req = HttpRequest::new(HttpMethod::Post, self.login_url.clone());
        req.headers
            .insert("Content-Type", "application/x-www-form-urlencoded");
        let body = format!(
            "{}={}&{}={}",
            self.username_field, self.username, self.password_field, self.password
        );
        req.body = body.into_bytes();
        req
    }

    /// Extract the session cookie from the login response. Returns the
    /// raw `name=value` pair suitable for re-injection in a `Cookie:`
    /// header on subsequent requests.
    pub fn capture_session(&self, resp: &HttpResponse) -> Option<SetCookie> {
        resp.set_cookies()
            .into_iter()
            .find(|c| c.name == self.session_cookie_name)
    }
}

/// Apply auth to a request. For Bearer this sets `Authorization`. For
/// Form-based with a captured cookie this sets `Cookie:`.
pub fn apply_auth(method: &AuthMethod, captured: Option<&SetCookie>, req: &mut HttpRequest) {
    match method {
        AuthMethod::None => {}
        AuthMethod::BearerToken(t) => {
            // Remove any pre-existing Authorization header to keep the apply idempotent.
            req.headers.remove("Authorization");
            req.headers.insert("Authorization", format!("Bearer {}", t));
        }
        AuthMethod::FormBased(_) => {
            if let Some(c) = captured {
                let cookie_hdr = format!("{}={}", c.name, c.value);
                replace_or_insert_cookie(&mut req.headers, &cookie_hdr);
            }
        }
    }
}

fn replace_or_insert_cookie(headers: &mut HeaderMap, kv: &str) {
    headers.remove("Cookie");
    headers.insert("Cookie", kv);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_sets_authorization_header() {
        let m = AuthMethod::BearerToken("abc123".to_string());
        let mut req = HttpRequest::new(HttpMethod::Get, "http://x/");
        apply_auth(&m, None, &mut req);
        assert_eq!(req.headers.first("Authorization"), Some("Bearer abc123"));
    }

    #[test]
    fn bearer_replaces_existing_authorization() {
        let m = AuthMethod::BearerToken("new".to_string());
        let mut req = HttpRequest::new(HttpMethod::Get, "http://x/");
        req.headers.insert("Authorization", "Bearer stale");
        apply_auth(&m, None, &mut req);
        let all: Vec<_> = req.headers.all("Authorization").collect();
        assert_eq!(all, vec!["Bearer new"]);
    }

    #[test]
    fn form_login_request_shape() {
        let cfg = FormAuthConfig {
            login_url: "https://x.test/login".to_string(),
            username_field: "u".to_string(),
            password_field: "p".to_string(),
            username: "alice".to_string(),
            password: "s3cr3t".to_string(),
            session_cookie_name: "SID".to_string(),
        };
        let req = cfg.login_request();
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.url, "https://x.test/login");
        assert_eq!(
            req.headers.first("Content-Type"),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(req.body_str(), Some("u=alice&p=s3cr3t"));
    }

    #[test]
    fn capture_session_cookie_by_name() {
        let cfg = FormAuthConfig {
            login_url: String::new(),
            username_field: "u".to_string(),
            password_field: "p".to_string(),
            username: String::new(),
            password: String::new(),
            session_cookie_name: "SID".to_string(),
        };
        let mut resp = HttpResponse::new(200, "OK");
        resp.headers.insert("Set-Cookie", "trace=on");
        resp.headers
            .insert("Set-Cookie", "SID=xyz; Secure; HttpOnly");
        let cap = cfg.capture_session(&resp).unwrap();
        assert_eq!(cap.name, "SID");
        assert_eq!(cap.value, "xyz");
    }

    #[test]
    fn form_apply_attaches_cookie() {
        let cfg = FormAuthConfig {
            login_url: String::new(),
            username_field: "u".to_string(),
            password_field: "p".to_string(),
            username: String::new(),
            password: String::new(),
            session_cookie_name: "SID".to_string(),
        };
        let m = AuthMethod::FormBased(cfg);
        let mut sc = SetCookie::default();
        sc.name = "SID".to_string();
        sc.value = "xyz".to_string();
        let mut req = HttpRequest::new(HttpMethod::Get, "https://x.test/api");
        apply_auth(&m, Some(&sc), &mut req);
        assert_eq!(req.headers.first("Cookie"), Some("SID=xyz"));
    }

    #[test]
    fn none_method_no_change() {
        let mut req = HttpRequest::new(HttpMethod::Get, "https://x.test/");
        apply_auth(&AuthMethod::None, None, &mut req);
        assert!(req.headers.first("Authorization").is_none());
        assert!(req.headers.first("Cookie").is_none());
    }
}
