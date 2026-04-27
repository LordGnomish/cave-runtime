//! ext_authz plugin — Envoy `envoy.filters.http.ext_authz` parity (config layer).
//!
//! Lets the gateway delegate authorization to an external HTTP or gRPC service
//! before the request is forwarded upstream. The external service receives:
//!
//! - the request method / path / host / authority
//! - a configurable subset of headers (`allowed_headers`)
//! - optionally the body (up to `max_request_bytes`)
//!
//! and replies with 200 (allow) or 4xx (deny). When allowed, headers from the
//! `headers_to_upstream_on_allow` allowlist are merged into the upstream
//! request; when denied, headers from `headers_to_client_on_deny` are merged
//! into the response we send back to the client.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtAuthzKind {
    Http,
    Grpc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderMatcher {
    /// Exact header names to forward. Case-insensitive.
    pub exact: Vec<String>,
    /// Header-name prefixes to forward. Case-insensitive.
    pub prefix: Vec<String>,
}

impl HeaderMatcher {
    pub fn allow(exact: Vec<&str>) -> Self {
        Self {
            exact: exact.into_iter().map(|s| s.to_ascii_lowercase()).collect(),
            prefix: vec![],
        }
    }

    pub fn matches(&self, header_name: &str) -> bool {
        let lower = header_name.to_ascii_lowercase();
        if self.exact.iter().any(|e| e == &lower) {
            return true;
        }
        if self
            .prefix
            .iter()
            .any(|p| lower.starts_with(&p.to_ascii_lowercase()))
        {
            return true;
        }
        false
    }
}

impl Default for HeaderMatcher {
    fn default() -> Self {
        // Envoy default allowlist — host / authority / path / method / cookies.
        Self {
            exact: vec![
                "host".into(),
                "authority".into(),
                "cookie".into(),
                "authorization".into(),
                "user-agent".into(),
                "x-forwarded-for".into(),
                "x-request-id".into(),
            ],
            prefix: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtAuthzConfig {
    pub kind: ExtAuthzKind,
    /// `http(s)://host:port` for HTTP, `dns:///host:port` for gRPC.
    pub server_uri: String,
    pub timeout: Duration,
    pub max_request_bytes: u64,
    /// Send the request body to the auth server (clamped to max_request_bytes).
    pub include_request_body: bool,
    /// Continue the request even if the auth server returns an error (Envoy
    /// `failure_mode_allow`).
    pub failure_mode_allow: bool,
    /// Whether to clear the route cache after auth (so headers added by the
    /// auth server can re-trigger routing).
    pub clear_route_cache: bool,
    /// Headers forwarded from the client to the auth server.
    pub allowed_request_headers: HeaderMatcher,
    /// Allow-list of headers that the auth server may inject into the
    /// upstream request after a 200-allow response.
    pub headers_to_upstream_on_allow: HeaderMatcher,
    /// Allow-list of headers from the auth server returned to the client on
    /// 4xx-deny.
    pub headers_to_client_on_deny: HeaderMatcher,
    /// Status code to return on deny (Envoy default 403).
    pub status_on_deny: u16,
}

impl Default for ExtAuthzConfig {
    fn default() -> Self {
        Self {
            kind: ExtAuthzKind::Http,
            server_uri: "".into(),
            timeout: Duration::from_secs(2),
            max_request_bytes: 16 * 1024,
            include_request_body: false,
            failure_mode_allow: false,
            clear_route_cache: false,
            allowed_request_headers: HeaderMatcher::default(),
            headers_to_upstream_on_allow: HeaderMatcher::default(),
            headers_to_client_on_deny: HeaderMatcher::default(),
            status_on_deny: 403,
        }
    }
}

impl ExtAuthzConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.server_uri.is_empty() {
            return Err("server_uri is empty".into());
        }
        match self.kind {
            ExtAuthzKind::Http => {
                if !(self.server_uri.starts_with("http://")
                    || self.server_uri.starts_with("https://"))
                {
                    return Err("HTTP ext_authz uri must be http:// or https://".into());
                }
            }
            ExtAuthzKind::Grpc => {
                if !(self.server_uri.starts_with("dns:///")
                    || self.server_uri.starts_with("grpc://")
                    || self.server_uri.starts_with("unix:")
                    || self.server_uri.starts_with("static://"))
                {
                    return Err(
                        "gRPC ext_authz uri must use dns:///, grpc://, unix:, or static://".into(),
                    );
                }
            }
        }
        if self.timeout.is_zero() {
            return Err("timeout must be > 0".into());
        }
        if !(400..=599).contains(&self.status_on_deny) {
            return Err("status_on_deny must be 4xx/5xx".into());
        }
        Ok(())
    }
}

/// Build the auth-server-bound request: which client headers to forward, plus
/// a maybe-truncated body.
#[derive(Debug, Clone)]
pub struct AuthCheckRequest {
    pub method: String,
    pub path: String,
    pub host: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub source_ip: String,
}

impl AuthCheckRequest {
    /// Project the inbound request into the shape sent to the auth server.
    pub fn from_inbound(
        cfg: &ExtAuthzConfig,
        method: &str,
        path: &str,
        host: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        source_ip: &str,
    ) -> Self {
        let mut filtered: Vec<(String, String)> = Vec::new();
        for (name, value) in headers {
            if cfg.allowed_request_headers.matches(name) {
                filtered.push((name.to_ascii_lowercase(), (*value).to_string()));
            }
        }
        // Always carry x-original-method / x-original-path / x-original-host
        // for the auth server (Envoy injects these as :method / :path).
        filtered.push((":method".into(), method.into()));
        filtered.push((":path".into(), path.into()));
        filtered.push((":authority".into(), host.into()));

        let body = if cfg.include_request_body {
            body.map(|b| {
                let max = cfg.max_request_bytes as usize;
                if b.len() > max { b[..max].to_vec() } else { b.to_vec() }
            })
        } else {
            None
        };

        Self {
            method: method.into(),
            path: path.into(),
            host: host.into(),
            headers: filtered,
            body,
            source_ip: source_ip.into(),
        }
    }
}

/// What the auth server returned. The transport layer parses the wire reply
/// (HTTP status code + response headers, or gRPC `CheckResponse`) into this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCheckResponse {
    Allow {
        /// Headers to merge into the upstream request (filtered later).
        headers_to_upstream: Vec<(String, String)>,
    },
    Deny {
        status: u16,
        /// Headers to merge into the response we return to the client.
        headers_to_client: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    },
    /// Auth server itself failed to respond. The caller decides allow vs deny
    /// using `failure_mode_allow`.
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    AllowWithUpstreamHeaders(Vec<(String, String)>),
    Deny {
        status: u16,
        headers_to_client: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    },
}

/// Apply the ext_authz response according to the config: filter the upstream
/// allow-headers and the deny-client-headers; honour failure_mode_allow.
pub fn apply(
    cfg: &ExtAuthzConfig,
    response: AuthCheckResponse,
) -> AuthDecision {
    match response {
        AuthCheckResponse::Allow { headers_to_upstream } => {
            let filtered: Vec<(String, String)> = headers_to_upstream
                .into_iter()
                .filter(|(k, _)| cfg.headers_to_upstream_on_allow.matches(k))
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect();
            AuthDecision::AllowWithUpstreamHeaders(filtered)
        }
        AuthCheckResponse::Deny { status, headers_to_client, body } => {
            let filtered: Vec<(String, String)> = headers_to_client
                .into_iter()
                .filter(|(k, _)| cfg.headers_to_client_on_deny.matches(k))
                .map(|(k, v)| (k.to_ascii_lowercase(), v))
                .collect();
            AuthDecision::Deny { status, headers_to_client: filtered, body }
        }
        AuthCheckResponse::Error(_) => {
            if cfg.failure_mode_allow {
                AuthDecision::AllowWithUpstreamHeaders(vec![])
            } else {
                AuthDecision::Deny {
                    status: cfg.status_on_deny,
                    headers_to_client: vec![],
                    body: None,
                }
            }
        }
    }
}

/// Strip duplicate headers (case-insensitive) keeping the last write wins.
pub fn dedupe_headers(input: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<(String, String)> = Vec::with_capacity(input.len());
    // Walk from end so the *last* occurrence wins.
    for (k, v) in input.into_iter().rev() {
        let lower = k.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            out.push((lower, v));
        }
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ExtAuthzConfig {
        ExtAuthzConfig {
            kind: ExtAuthzKind::Http,
            server_uri: "http://authz:9000/check".into(),
            ..Default::default()
        }
    }

    #[test]
    fn header_matcher_exact_case_insensitive() {
        let m = HeaderMatcher::allow(vec!["Authorization"]);
        assert!(m.matches("authorization"));
        assert!(m.matches("Authorization"));
        assert!(!m.matches("X-Forwarded-For"));
    }

    #[test]
    fn header_matcher_prefix() {
        let m = HeaderMatcher {
            exact: vec![],
            prefix: vec!["x-cave-".into()],
        };
        assert!(m.matches("X-Cave-User"));
        assert!(m.matches("x-cave-tenant"));
        assert!(!m.matches("authorization"));
    }

    #[test]
    fn config_default_invalid_uri_rejects() {
        let c = ExtAuthzConfig::default();
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_http_uri_validates() {
        cfg().validate().unwrap();
    }

    #[test]
    fn config_https_uri_validates() {
        let mut c = cfg();
        c.server_uri = "https://authz:443/check".into();
        c.validate().unwrap();
    }

    #[test]
    fn config_grpc_dns_uri_validates() {
        let mut c = cfg();
        c.kind = ExtAuthzKind::Grpc;
        c.server_uri = "dns:///authz:9000".into();
        c.validate().unwrap();
    }

    #[test]
    fn config_grpc_unix_uri_validates() {
        let mut c = cfg();
        c.kind = ExtAuthzKind::Grpc;
        c.server_uri = "unix:/var/run/authz.sock".into();
        c.validate().unwrap();
    }

    #[test]
    fn config_grpc_invalid_scheme_rejects() {
        let mut c = cfg();
        c.kind = ExtAuthzKind::Grpc;
        c.server_uri = "http://authz:9000".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_http_invalid_scheme_rejects() {
        let mut c = cfg();
        c.server_uri = "ftp://authz".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_zero_timeout_rejects() {
        let mut c = cfg();
        c.timeout = Duration::ZERO;
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_invalid_deny_status_rejects() {
        let mut c = cfg();
        c.status_on_deny = 200;
        assert!(c.validate().is_err());
    }

    #[test]
    fn from_inbound_filters_to_allowlist() {
        let mut c = cfg();
        c.allowed_request_headers = HeaderMatcher::allow(vec!["Authorization"]);
        let req = AuthCheckRequest::from_inbound(
            &c,
            "GET",
            "/api/x",
            "example.com",
            &[("Authorization", "Bearer abc"), ("X-Skip", "1")],
            None,
            "1.2.3.4",
        );
        assert!(req.headers.iter().any(|(k, v)| k == "authorization" && v == "Bearer abc"));
        assert!(!req.headers.iter().any(|(k, _)| k == "x-skip"));
    }

    #[test]
    fn from_inbound_injects_pseudo_headers() {
        let req = AuthCheckRequest::from_inbound(
            &cfg(),
            "POST",
            "/api/x",
            "example.com",
            &[],
            None,
            "1.2.3.4",
        );
        assert!(req.headers.iter().any(|(k, v)| k == ":method" && v == "POST"));
        assert!(req.headers.iter().any(|(k, v)| k == ":path" && v == "/api/x"));
        assert!(req.headers.iter().any(|(k, v)| k == ":authority" && v == "example.com"));
    }

    #[test]
    fn from_inbound_omits_body_when_disabled() {
        let req = AuthCheckRequest::from_inbound(
            &cfg(),
            "POST",
            "/api",
            "x",
            &[],
            Some(b"hello"),
            "1.2.3.4",
        );
        assert!(req.body.is_none());
    }

    #[test]
    fn from_inbound_truncates_body_to_max() {
        let mut c = cfg();
        c.include_request_body = true;
        c.max_request_bytes = 4;
        let req = AuthCheckRequest::from_inbound(
            &c,
            "POST",
            "/api",
            "x",
            &[],
            Some(b"abcdefgh"),
            "1.2.3.4",
        );
        assert_eq!(req.body.as_deref(), Some(&b"abcd"[..]));
    }

    #[test]
    fn from_inbound_keeps_short_body() {
        let mut c = cfg();
        c.include_request_body = true;
        c.max_request_bytes = 16;
        let req = AuthCheckRequest::from_inbound(
            &c,
            "POST",
            "/api",
            "x",
            &[],
            Some(b"hi"),
            "1.2.3.4",
        );
        assert_eq!(req.body.as_deref(), Some(&b"hi"[..]));
    }

    #[test]
    fn apply_allow_filters_upstream_headers() {
        let mut c = cfg();
        c.headers_to_upstream_on_allow = HeaderMatcher::allow(vec!["x-user-id"]);
        let r = AuthCheckResponse::Allow {
            headers_to_upstream: vec![
                ("X-User-Id".into(), "alice".into()),
                ("X-Skip".into(), "1".into()),
            ],
        };
        match apply(&c, r) {
            AuthDecision::AllowWithUpstreamHeaders(headers) => {
                assert_eq!(headers.len(), 1);
                assert_eq!(headers[0].0, "x-user-id");
            }
            _ => panic!("expected allow"),
        }
    }

    #[test]
    fn apply_deny_filters_client_headers() {
        let mut c = cfg();
        c.headers_to_client_on_deny = HeaderMatcher::allow(vec!["www-authenticate"]);
        let r = AuthCheckResponse::Deny {
            status: 401,
            headers_to_client: vec![
                ("WWW-Authenticate".into(), "Bearer".into()),
                ("X-Internal".into(), "leak".into()),
            ],
            body: None,
        };
        match apply(&c, r) {
            AuthDecision::Deny { headers_to_client, .. } => {
                assert_eq!(headers_to_client.len(), 1);
                assert_eq!(headers_to_client[0].0, "www-authenticate");
            }
            _ => panic!("expected deny"),
        }
    }

    #[test]
    fn apply_error_failure_mode_allow_passes_through() {
        let mut c = cfg();
        c.failure_mode_allow = true;
        let r = AuthCheckResponse::Error("connection refused".into());
        match apply(&c, r) {
            AuthDecision::AllowWithUpstreamHeaders(h) => assert!(h.is_empty()),
            _ => panic!("expected allow"),
        }
    }

    #[test]
    fn apply_error_default_denies_with_status() {
        let c = cfg();
        let r = AuthCheckResponse::Error("blew up".into());
        match apply(&c, r) {
            AuthDecision::Deny { status, .. } => assert_eq!(status, 403),
            _ => panic!("expected deny"),
        }
    }

    #[test]
    fn dedupe_keeps_last_write() {
        let v = vec![
            ("X-Foo".into(), "first".into()),
            ("X-Foo".into(), "second".into()),
            ("X-Bar".into(), "yes".into()),
        ];
        let out = dedupe_headers(v);
        assert_eq!(out.len(), 2);
        let foo = out.iter().find(|(k, _)| k == "x-foo").unwrap();
        assert_eq!(foo.1, "second");
    }
}
