// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! `http.send` allowlist gate — OPA built-in safety wrapper.
//!
//! NOTICE: upstream is open-policy-agent/opa (Apache-2.0)
//! `topdown/http.go::builtinHTTPSend`. cave-policy models the request
//! shape, the allowlist enforcement, and the response shape. The
//! actual outbound HTTP I/O is delegated to `cave_kernel::http` per
//! ADR-RUNTIME-SANDBOX-NO-FFI-001 §1 (no direct socket use inside the
//! policy crate; outbound calls go through the kernel's allowlisted,
//! audited transport).

use crate::error::PolicyError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One `http.send({...})` invocation as seen by the policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpSendRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u32,
    /// When true, OPA caches the response by URL+method+body for the
    /// remainder of the policy evaluation.
    #[serde(default)]
    pub cache: bool,
    /// Optional max-age override (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_cache_ttl: Option<u32>,
    /// `raise_error` (OPA default true) — when false, errors land in
    /// the response object instead of aborting the eval.
    #[serde(default = "default_true")]
    pub raise_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpSendResponse {
    pub status_code: u16,
    pub status: String,
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Populated when `raise_error: false` and the request failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<HttpSendErrorObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpSendErrorObject {
    pub code: String,
    pub message: String,
}

/// Allowlist gate — what URLs / methods / header names the policy is
/// permitted to use. Mirrors the cave-kernel HTTP allowlist contract.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpSendAllowlist {
    /// Allowed host:port pairs (literal — no wildcards in the host).
    /// `example.org:443` or `api.internal:8080`.
    pub hosts: BTreeSet<String>,
    /// Allowed HTTP methods. Empty set = `GET` only.
    pub methods: BTreeSet<String>,
    /// Allowed header *names* the policy may set (case-insensitive
    /// match; canonical lowercase storage). Hop-by-hop and bearer
    /// headers are always denied.
    pub allow_request_headers: BTreeSet<String>,
    /// Hard max body size that may be sent (bytes).
    pub max_request_body_bytes: u32,
    /// Hard ceiling on `timeout_ms`.
    pub max_timeout_ms: u32,
}

impl HttpSendAllowlist {
    pub fn permissive_localhost() -> Self {
        let mut s = Self::default();
        s.hosts.insert("localhost:80".into());
        s.hosts.insert("localhost:443".into());
        s.methods.insert("GET".into());
        s.methods.insert("HEAD".into());
        s.max_request_body_bytes = 0;
        s.max_timeout_ms = 5_000;
        s
    }
}

/// Headers we ALWAYS deny — leaking auth, hop-by-hop, or session-binding.
fn always_denied_header(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "authorization"
            | "cookie"
            | "set-cookie"
            | "proxy-authorization"
            | "proxy-authenticate"
            | "connection"
            | "upgrade"
            | "keep-alive"
            | "transfer-encoding"
            | "te"
            | "trailer"
            | "host"
    )
}

/// Evaluate a `http.send` request against an allowlist. Returns Ok(())
/// if the request would be permitted to leave the runtime, Err
/// otherwise. **Does not** perform any I/O.
pub fn evaluate(req: &HttpSendRequest, allow: &HttpSendAllowlist) -> Result<(), PolicyError> {
    // 1. Method.
    let method = req.method.to_ascii_uppercase();
    let allowed_methods: BTreeSet<String> = if allow.methods.is_empty() {
        let mut s = BTreeSet::new();
        s.insert("GET".into());
        s
    } else {
        allow.methods.iter().map(|m| m.to_ascii_uppercase()).collect()
    };
    if !allowed_methods.contains(&method) {
        return Err(PolicyError::Validation(format!(
            "http.send method '{method}' not allowed (allowed={:?})", allowed_methods
        )));
    }

    // 2. URL → host:port.
    let host_port = parse_host_port(&req.url)?;
    if !allow.hosts.contains(&host_port) {
        return Err(PolicyError::Validation(format!(
            "http.send host '{host_port}' not in allowlist"
        )));
    }

    // 3. Headers — every set header must be in allow_request_headers and
    // not in the always-denied set. Case-insensitive comparison.
    let allow_lower: BTreeSet<String> = allow
        .allow_request_headers
        .iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();
    for h in req.headers.keys() {
        let l = h.to_ascii_lowercase();
        if always_denied_header(&l) {
            return Err(PolicyError::Validation(format!(
                "http.send header '{h}' is always denied (auth / hop-by-hop / session)"
            )));
        }
        if !allow_lower.contains(&l) {
            return Err(PolicyError::Validation(format!(
                "http.send header '{h}' not in allowlist"
            )));
        }
    }

    // 4. Body size + timeout caps.
    if let Some(b) = &req.body {
        if allow.max_request_body_bytes > 0 && (b.len() as u32) > allow.max_request_body_bytes {
            return Err(PolicyError::Validation(format!(
                "http.send body size {} > max {}", b.len(), allow.max_request_body_bytes
            )));
        }
    }
    if allow.max_timeout_ms > 0 && req.timeout_ms > allow.max_timeout_ms {
        return Err(PolicyError::Validation(format!(
            "http.send timeout_ms {} > max {}", req.timeout_ms, allow.max_timeout_ms
        )));
    }

    Ok(())
}

fn parse_host_port(url: &str) -> Result<String, PolicyError> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| PolicyError::Validation(format!("http.send url must be http[s]: '{url}'")))?;
    // Strip path + query.
    let host_with_port = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if host_with_port.is_empty() {
        return Err(PolicyError::Validation(format!("http.send url has empty host: '{url}'")));
    }
    if host_with_port.contains(':') {
        Ok(host_with_port.to_string())
    } else {
        let default_port = if url.starts_with("https://") { 443 } else { 80 };
        Ok(format!("{host_with_port}:{default_port}"))
    }
}

fn default_timeout() -> u32 { 5_000 }
fn default_true() -> bool { true }

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: &str, url: &str) -> HttpSendRequest {
        HttpSendRequest {
            method: method.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 1000,
            cache: false,
            force_cache_ttl: None,
            raise_error: true,
        }
    }

    fn allow_basic() -> HttpSendAllowlist {
        let mut a = HttpSendAllowlist::default();
        a.hosts.insert("api.internal:443".into());
        a.methods.insert("GET".into());
        a.methods.insert("POST".into());
        a.allow_request_headers.insert("Content-Type".into());
        a.max_request_body_bytes = 1024;
        a.max_timeout_ms = 10_000;
        a
    }

    #[test]
    fn permissive_localhost_defaults_to_get_head() {
        let a = HttpSendAllowlist::permissive_localhost();
        assert!(a.methods.contains("GET"));
        assert!(a.methods.contains("HEAD"));
        assert!(a.hosts.contains("localhost:443"));
    }

    #[test]
    fn parse_host_port_inserts_default_port() {
        assert_eq!(parse_host_port("https://example.org/foo").unwrap(), "example.org:443");
        assert_eq!(parse_host_port("http://example.org/foo").unwrap(), "example.org:80");
    }

    #[test]
    fn parse_host_port_keeps_explicit_port() {
        assert_eq!(parse_host_port("https://api.internal:8443/v1/x").unwrap(), "api.internal:8443");
    }

    #[test]
    fn parse_host_port_rejects_non_http_scheme() {
        assert!(parse_host_port("file:///etc/passwd").is_err());
        assert!(parse_host_port("//bare-rel").is_err());
    }

    #[test]
    fn evaluate_method_allowed_passes() {
        let r = req("GET", "https://api.internal/v1/data");
        evaluate(&r, &allow_basic()).unwrap();
    }

    #[test]
    fn evaluate_method_denied_blocks() {
        let r = req("DELETE", "https://api.internal/v1/data");
        assert!(evaluate(&r, &allow_basic()).is_err());
    }

    #[test]
    fn evaluate_method_default_to_get_when_allow_empty() {
        let mut a = allow_basic();
        a.methods.clear();
        evaluate(&req("GET", "https://api.internal/v1/data"), &a).unwrap();
        assert!(evaluate(&req("POST", "https://api.internal/v1/data"), &a).is_err());
    }

    #[test]
    fn evaluate_host_not_in_allowlist_blocks() {
        let r = req("GET", "https://evil.example/data");
        assert!(evaluate(&r, &allow_basic()).is_err());
    }

    #[test]
    fn evaluate_header_always_denied_authorization() {
        let mut r = req("GET", "https://api.internal/v1/data");
        r.headers.insert("Authorization".into(), "Bearer xxx".into());
        let mut a = allow_basic();
        a.allow_request_headers.insert("Authorization".into());
        // Even when explicitly allowed, Authorization is always denied.
        assert!(evaluate(&r, &a).is_err());
    }

    #[test]
    fn evaluate_header_always_denied_cookie_set_cookie_host() {
        for name in ["Cookie", "Set-Cookie", "Host", "Connection"] {
            let mut r = req("GET", "https://api.internal/v1/data");
            r.headers.insert(name.into(), "x".into());
            assert!(evaluate(&r, &allow_basic()).is_err(), "{name} should be denied");
        }
    }

    #[test]
    fn evaluate_header_not_in_allowlist_blocks() {
        let mut r = req("GET", "https://api.internal/v1/data");
        r.headers.insert("X-Custom".into(), "y".into());
        assert!(evaluate(&r, &allow_basic()).is_err());
    }

    #[test]
    fn evaluate_header_case_insensitive_match() {
        let mut r = req("POST", "https://api.internal/v1/data");
        r.headers.insert("content-type".into(), "application/json".into());
        evaluate(&r, &allow_basic()).unwrap();
    }

    #[test]
    fn evaluate_body_size_cap() {
        let mut r = req("POST", "https://api.internal/v1/data");
        r.body = Some("a".repeat(2048));
        assert!(evaluate(&r, &allow_basic()).is_err());
    }

    #[test]
    fn evaluate_body_size_unlimited_when_max_zero() {
        let mut a = allow_basic();
        a.max_request_body_bytes = 0;
        let mut r = req("POST", "https://api.internal/v1/data");
        r.body = Some("a".repeat(2048));
        evaluate(&r, &a).unwrap();
    }

    #[test]
    fn evaluate_timeout_cap() {
        let mut r = req("GET", "https://api.internal/v1/data");
        r.timeout_ms = 30_000;
        assert!(evaluate(&r, &allow_basic()).is_err());
    }

    #[test]
    fn evaluate_default_port_inferred_from_scheme() {
        let mut a = HttpSendAllowlist::default();
        a.hosts.insert("api.x:443".into());
        a.methods.insert("GET".into());
        a.max_timeout_ms = 5000;
        // URL without explicit port should be matched at :443.
        evaluate(&req("GET", "https://api.x/v"), &a).unwrap();
    }

    #[test]
    fn request_serde_round_trip() {
        let r = req("POST", "https://api.internal/v");
        let j = serde_json::to_string(&r).unwrap();
        let r2: HttpSendRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn response_serde_round_trip_with_error_object() {
        let resp = HttpSendResponse {
            status_code: 502,
            status: "Bad Gateway".into(),
            headers: BTreeMap::new(),
            body: None,
            error: Some(HttpSendErrorObject { code: "EIO".into(), message: "upstream down".into() }),
        };
        let j = serde_json::to_string(&resp).unwrap();
        let r2: HttpSendResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(r2.error.as_ref().unwrap().code, "EIO");
    }
}
