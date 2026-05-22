// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gateway API filters — URLRewrite, RequestRedirect,
//! RequestHeaderModifier, ResponseHeaderModifier.
//!
//! Mirrors `pkg/gateway-api/translation/filter.go`. These filters
//! transform the request (or response) before / after the proxy
//! forwards it. Each filter is declarative; composition order is
//! left-to-right inside an HTTPRouteRule.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlRewrite {
    pub hostname: Option<String>,
    /// `Some("/api")` rewrites prefix; `Some("")` strips prefix entirely.
    pub replace_prefix: Option<String>,
    /// Replace the full path with this string.
    pub replace_full_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedirectScheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestRedirect {
    pub scheme: Option<RedirectScheme>,
    pub hostname: Option<String>,
    pub port: Option<u16>,
    pub status_code: u16, // 301, 302, 303, 307, 308
    pub replace_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderMutation {
    pub set: Vec<(String, String)>,
    pub add: Vec<(String, String)>,
    pub remove: Vec<String>,
}

impl HeaderMutation {
    pub fn apply(&self, headers: &mut Vec<(String, String)>) {
        for (k, v) in &self.set {
            headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
            headers.push((k.clone(), v.clone()));
        }
        for (k, v) in &self.add {
            headers.push((k.clone(), v.clone()));
        }
        for k in &self.remove {
            headers.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpFilter {
    UrlRewrite(UrlRewrite),
    RequestRedirect(RequestRedirect),
    RequestHeaderModifier(HeaderMutation),
    ResponseHeaderModifier(HeaderMutation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub scheme: RedirectScheme,
    pub hostname: String,
    pub port: u16,
    pub path: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOutcome {
    /// Forward the (possibly modified) request to the upstream.
    Forward(HttpRequest),
    /// Reply with a redirect (Location header + status code).
    Redirect { location: String, status: u16 },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FilterError {
    #[error("invalid redirect status {0} (must be 301/302/303/307/308)")]
    BadRedirectStatus(u16),
    #[error("URLRewrite must specify replace_prefix or replace_full_path or hostname")]
    EmptyUrlRewrite,
    #[error("tenant {tenant} cannot mutate filter chain owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Apply a request-side filter chain. The first `RequestRedirect` filter
/// short-circuits the chain.
pub fn apply_request_chain(
    filters: &[HttpFilter],
    req: HttpRequest,
) -> Result<FilterOutcome, FilterError> {
    let mut current = req;
    for f in filters {
        match f {
            HttpFilter::RequestRedirect(r) => {
                if !matches!(r.status_code, 301 | 302 | 303 | 307 | 308) {
                    return Err(FilterError::BadRedirectStatus(r.status_code));
                }
                let scheme = r.scheme.unwrap_or(current.scheme);
                let host = r
                    .hostname
                    .clone()
                    .unwrap_or_else(|| current.hostname.clone());
                let port = r.port.unwrap_or(current.port);
                let path = if let Some(prefix) = &r.replace_prefix {
                    rewrite_prefix(&current.path, prefix)
                } else {
                    current.path.clone()
                };
                let scheme_str = match scheme {
                    RedirectScheme::Http => "http",
                    RedirectScheme::Https => "https",
                };
                let location = format!("{scheme_str}://{host}:{port}{path}");
                return Ok(FilterOutcome::Redirect {
                    location,
                    status: r.status_code,
                });
            }
            HttpFilter::UrlRewrite(rw) => {
                if rw.hostname.is_none()
                    && rw.replace_prefix.is_none()
                    && rw.replace_full_path.is_none()
                {
                    return Err(FilterError::EmptyUrlRewrite);
                }
                if let Some(h) = &rw.hostname {
                    current.hostname = h.clone();
                }
                if let Some(p) = &rw.replace_full_path {
                    current.path = p.clone();
                } else if let Some(prefix) = &rw.replace_prefix {
                    current.path = rewrite_prefix(&current.path, prefix);
                }
            }
            HttpFilter::RequestHeaderModifier(hm) => {
                hm.apply(&mut current.headers);
            }
            HttpFilter::ResponseHeaderModifier(_) => {
                // Skip — applies on the response path.
            }
        }
    }
    Ok(FilterOutcome::Forward(current))
}

/// Apply a response-side filter chain. Only `ResponseHeaderModifier`
/// participates.
pub fn apply_response_chain(filters: &[HttpFilter], mut resp: HttpResponse) -> HttpResponse {
    for f in filters {
        if let HttpFilter::ResponseHeaderModifier(hm) = f {
            hm.apply(&mut resp.headers);
        }
    }
    resp
}

fn rewrite_prefix(path: &str, replacement: &str) -> String {
    // Cilium / Gateway API URLRewrite semantics: the `replace_prefix`
    // replaces the **matched** prefix portion. Here we mimic by replacing
    // the *first segment* matched against the longest path-prefix the
    // route's match rule accepted. Without that context we simply set
    // the prefix to `replacement`.
    let trimmed = replacement.trim_end_matches('/');
    if let Some(stripped) = path.strip_prefix('/') {
        if let Some(rest) = stripped.split_once('/') {
            return format!("{}/{}", trimmed, rest.1);
        }
    }
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/gateway-api/translation/filter.go", "ApplyFilters");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn req(path: &str) -> HttpRequest {
        HttpRequest {
            scheme: RedirectScheme::Http,
            hostname: "api.example.com".into(),
            port: 80,
            path: path.into(),
            headers: vec![("user-agent".into(), "test/1.0".into())],
        }
    }

    fn resp() -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
        }
    }

    // ── HeaderMutation ───────────────────────────────────────────────────────

    #[test]
    fn header_set_replaces_existing_value() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Header.Set",
            "tenant-gw-set"
        );
        let h = HeaderMutation {
            set: vec![("x-custom".into(), "new".into())],
            add: vec![],
            remove: vec![],
        };
        let mut headers = vec![("x-custom".into(), "old".into())];
        h.apply(&mut headers);
        assert_eq!(headers, vec![("x-custom".to_string(), "new".to_string())]);
    }

    #[test]
    fn header_add_appends_value() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Header.Add",
            "tenant-gw-add"
        );
        let h = HeaderMutation {
            set: vec![],
            add: vec![("x-new".into(), "v".into())],
            remove: vec![],
        };
        let mut headers = vec![("x-existing".into(), "z".into())];
        h.apply(&mut headers);
        assert!(headers.iter().any(|(k, v)| k == "x-new" && v == "v"));
        assert!(headers.iter().any(|(k, v)| k == "x-existing" && v == "z"));
    }

    #[test]
    fn header_remove_drops_existing() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Header.Remove",
            "tenant-gw-rm"
        );
        let h = HeaderMutation {
            set: vec![],
            add: vec![],
            remove: vec!["x-bad".into()],
        };
        let mut headers = vec![("x-bad".into(), "v".into()), ("x-good".into(), "z".into())];
        h.apply(&mut headers);
        assert_eq!(headers, vec![("x-good".to_string(), "z".to_string())]);
    }

    #[test]
    fn header_remove_case_insensitive() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Header.Remove.Ci",
            "tenant-gw-rmci"
        );
        let h = HeaderMutation {
            set: vec![],
            add: vec![],
            remove: vec!["X-Bad".into()],
        };
        let mut headers = vec![("x-bad".into(), "v".into())];
        h.apply(&mut headers);
        assert!(headers.is_empty());
    }

    // ── URLRewrite ──────────────────────────────────────────────────────────

    #[test]
    fn url_rewrite_replaces_hostname() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "URLRewrite.Hostname",
            "tenant-gw-rwh"
        );
        let f = HttpFilter::UrlRewrite(UrlRewrite {
            hostname: Some("upstream.local".into()),
            replace_prefix: None,
            replace_full_path: None,
        });
        let out = apply_request_chain(&[f], req("/v1/users")).unwrap();
        match out {
            FilterOutcome::Forward(r) => assert_eq!(r.hostname, "upstream.local"),
            _ => panic!(),
        }
    }

    #[test]
    fn url_rewrite_replace_full_path() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "URLRewrite.FullPath",
            "tenant-gw-rwfp"
        );
        let f = HttpFilter::UrlRewrite(UrlRewrite {
            hostname: None,
            replace_prefix: None,
            replace_full_path: Some("/v2".into()),
        });
        let out = apply_request_chain(&[f], req("/v1/users")).unwrap();
        match out {
            FilterOutcome::Forward(r) => assert_eq!(r.path, "/v2"),
            _ => panic!(),
        }
    }

    #[test]
    fn url_rewrite_replace_prefix_keeps_suffix() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "URLRewrite.PrefixKeep",
            "tenant-gw-rwpk"
        );
        let f = HttpFilter::UrlRewrite(UrlRewrite {
            hostname: None,
            replace_prefix: Some("/api".into()),
            replace_full_path: None,
        });
        let out = apply_request_chain(&[f], req("/v1/users")).unwrap();
        match out {
            FilterOutcome::Forward(r) => assert_eq!(r.path, "/api/users"),
            _ => panic!(),
        }
    }

    #[test]
    fn url_rewrite_empty_filter_rejected() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "URLRewrite.Empty",
            "tenant-gw-rwe"
        );
        let f = HttpFilter::UrlRewrite(UrlRewrite {
            hostname: None,
            replace_prefix: None,
            replace_full_path: None,
        });
        let err = apply_request_chain(&[f], req("/v1")).unwrap_err();
        assert_eq!(err, FilterError::EmptyUrlRewrite);
    }

    // ── RequestRedirect ─────────────────────────────────────────────────────

    #[test]
    fn redirect_returns_location_with_status() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Redirect.Basic",
            "tenant-gw-rdb"
        );
        let f = HttpFilter::RequestRedirect(RequestRedirect {
            scheme: Some(RedirectScheme::Https),
            hostname: Some("secure.example.com".into()),
            port: Some(443),
            status_code: 301,
            replace_prefix: None,
        });
        let out = apply_request_chain(&[f], req("/v1/users")).unwrap();
        match out {
            FilterOutcome::Redirect { location, status } => {
                assert!(location.starts_with("https://secure.example.com:443"));
                assert!(location.ends_with("/v1/users"));
                assert_eq!(status, 301);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn redirect_short_circuits_subsequent_filters() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Redirect.ShortCircuit",
            "tenant-gw-rdsc"
        );
        let chain = vec![
            HttpFilter::RequestRedirect(RequestRedirect {
                scheme: Some(RedirectScheme::Https),
                hostname: None,
                port: None,
                status_code: 308,
                replace_prefix: None,
            }),
            // Should never run.
            HttpFilter::UrlRewrite(UrlRewrite {
                hostname: Some("never".into()),
                replace_prefix: None,
                replace_full_path: None,
            }),
        ];
        let out = apply_request_chain(&chain, req("/v1")).unwrap();
        assert!(matches!(out, FilterOutcome::Redirect { .. }));
    }

    #[test]
    fn redirect_with_invalid_status_rejected() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Redirect.BadStatus",
            "tenant-gw-rdbs"
        );
        let f = HttpFilter::RequestRedirect(RequestRedirect {
            scheme: None,
            hostname: None,
            port: None,
            status_code: 200,
            replace_prefix: None,
        });
        let err = apply_request_chain(&[f], req("/v1")).unwrap_err();
        assert_eq!(err, FilterError::BadRedirectStatus(200));
    }

    #[test]
    fn redirect_inherits_request_scheme_when_unset() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Redirect.SchemeInherit",
            "tenant-gw-rdsi"
        );
        let f = HttpFilter::RequestRedirect(RequestRedirect {
            scheme: None,
            hostname: Some("other.example.com".into()),
            port: Some(8080),
            status_code: 302,
            replace_prefix: None,
        });
        let out = apply_request_chain(&[f], req("/v1")).unwrap();
        match out {
            FilterOutcome::Redirect { location, .. } => {
                assert!(location.starts_with("http://other.example.com:8080"))
            }
            _ => panic!(),
        }
    }

    // ── Request header chain ────────────────────────────────────────────────

    #[test]
    fn request_header_modifier_modifies_outgoing_headers() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "RequestHeader.Apply",
            "tenant-gw-rh"
        );
        let f = HttpFilter::RequestHeaderModifier(HeaderMutation {
            set: vec![("x-tenant".into(), "acme".into())],
            add: vec![],
            remove: vec!["user-agent".into()],
        });
        let out = apply_request_chain(&[f], req("/v1")).unwrap();
        match out {
            FilterOutcome::Forward(r) => {
                assert!(r
                    .headers
                    .iter()
                    .any(|(k, v)| k == "x-tenant" && v == "acme"));
                assert!(!r.headers.iter().any(|(k, _)| k == "user-agent"));
            }
            _ => panic!(),
        }
    }

    // ── Response header chain ───────────────────────────────────────────────

    #[test]
    fn response_header_modifier_runs_on_response_chain() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "ResponseHeader.Apply",
            "tenant-gw-rsp"
        );
        let f = HttpFilter::ResponseHeaderModifier(HeaderMutation {
            set: vec![("x-frame-options".into(), "DENY".into())],
            add: vec![],
            remove: vec![],
        });
        let r = apply_response_chain(&[f], resp());
        assert!(r
            .headers
            .iter()
            .any(|(k, v)| k == "x-frame-options" && v == "DENY"));
    }

    #[test]
    fn response_chain_skipped_in_request_chain() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "ResponseHeader.SkipReq",
            "tenant-gw-rsr"
        );
        let f = HttpFilter::ResponseHeaderModifier(HeaderMutation {
            set: vec![("never".into(), "v".into())],
            add: vec![],
            remove: vec![],
        });
        let out = apply_request_chain(&[f], req("/v1")).unwrap();
        match out {
            FilterOutcome::Forward(r) => assert!(!r.headers.iter().any(|(k, _)| k == "never")),
            _ => panic!(),
        }
    }

    #[test]
    fn request_chain_skips_response_header_modifier() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "RequestHeader.SkipRsp",
            "tenant-gw-rsr2"
        );
        let f = HttpFilter::RequestHeaderModifier(HeaderMutation {
            set: vec![("x-req".into(), "v".into())],
            add: vec![],
            remove: vec![],
        });
        let r = apply_response_chain(&[f], resp());
        assert!(!r.headers.iter().any(|(k, _)| k == "x-req"));
    }

    // ── Composition ────────────────────────────────────────────────────────

    #[test]
    fn composition_url_rewrite_then_header_set() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Compose.RewriteThenHeader",
            "tenant-gw-comp"
        );
        let chain = vec![
            HttpFilter::UrlRewrite(UrlRewrite {
                hostname: Some("upstream.local".into()),
                replace_prefix: None,
                replace_full_path: None,
            }),
            HttpFilter::RequestHeaderModifier(HeaderMutation {
                set: vec![("x-host".into(), "upstream.local".into())],
                add: vec![],
                remove: vec![],
            }),
        ];
        let out = apply_request_chain(&chain, req("/v1")).unwrap();
        match out {
            FilterOutcome::Forward(r) => {
                assert_eq!(r.hostname, "upstream.local");
                assert!(r
                    .headers
                    .iter()
                    .any(|(k, v)| k == "x-host" && v == "upstream.local"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn composition_redirect_after_url_rewrite_uses_rewritten_url() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Compose.RewriteThenRedirect",
            "tenant-gw-cmpr"
        );
        let chain = vec![
            HttpFilter::UrlRewrite(UrlRewrite {
                hostname: None,
                replace_prefix: None,
                replace_full_path: Some("/canonical".into()),
            }),
            HttpFilter::RequestRedirect(RequestRedirect {
                scheme: Some(RedirectScheme::Https),
                hostname: Some("secure.example.com".into()),
                port: Some(443),
                status_code: 308,
                replace_prefix: None,
            }),
        ];
        let out = apply_request_chain(&chain, req("/v1")).unwrap();
        match out {
            FilterOutcome::Redirect { location, .. } => assert!(location.ends_with("/canonical")),
            _ => panic!(),
        }
    }

    // ── Empty chain ────────────────────────────────────────────────────────

    #[test]
    fn empty_chain_forwards_unchanged() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "EmptyChain",
            "tenant-gw-empt"
        );
        let r = req("/v1");
        let out = apply_request_chain(&[], r.clone()).unwrap();
        match out {
            FilterOutcome::Forward(out_r) => assert_eq!(out_r, r),
            _ => panic!(),
        }
    }

    #[test]
    fn empty_response_chain_returns_unchanged() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "EmptyResponseChain",
            "tenant-gw-empr"
        );
        let r = resp();
        let out = apply_response_chain(&[], r.clone());
        assert_eq!(out, r);
    }

    // ── Header set is case-insensitive on existing key ──────────────────────

    #[test]
    fn header_set_overwrites_case_insensitive_match() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Header.Set.Ci",
            "tenant-gw-setci"
        );
        let h = HeaderMutation {
            set: vec![("X-Custom".into(), "new".into())],
            add: vec![],
            remove: vec![],
        };
        let mut headers = vec![("x-custom".into(), "old".into())];
        h.apply(&mut headers);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].1, "new");
    }

    // ── Status code coverage ────────────────────────────────────────────────

    #[test]
    fn redirect_accepts_all_documented_statuses() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Redirect.Statuses",
            "tenant-gw-rds"
        );
        for code in [301u16, 302, 303, 307, 308] {
            let f = HttpFilter::RequestRedirect(RequestRedirect {
                scheme: None,
                hostname: None,
                port: None,
                status_code: code,
                replace_prefix: None,
            });
            assert!(
                apply_request_chain(&[f], req("/v1")).is_ok(),
                "code {code} should be valid"
            );
        }
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn http_filter_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "HttpFilter.Serde",
            "tenant-gw-fserde"
        );
        let f = HttpFilter::RequestHeaderModifier(HeaderMutation {
            set: vec![("k".into(), "v".into())],
            add: vec![("a".into(), "b".into())],
            remove: vec!["x".into()],
        });
        let s = serde_json::to_string(&f).unwrap();
        let back: HttpFilter = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn url_rewrite_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "URLRewrite.Serde",
            "tenant-gw-uwserde"
        );
        let r = UrlRewrite {
            hostname: Some("upstream".into()),
            replace_prefix: Some("/api".into()),
            replace_full_path: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: UrlRewrite = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn redirect_scheme_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "Scheme.Serde",
            "tenant-gw-rsserde"
        );
        for s in [RedirectScheme::Http, RedirectScheme::Https] {
            let j = serde_json::to_string(&s).unwrap();
            let back: RedirectScheme = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn header_mutation_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/gateway-api/translation/filter.go",
            "HeaderMutation.Serde",
            "tenant-gw-hmserde"
        );
        let h = HeaderMutation {
            set: vec![("k".into(), "v".into())],
            add: vec![],
            remove: vec!["x".into()],
        };
        let s = serde_json::to_string(&h).unwrap();
        let back: HeaderMutation = serde_json::from_str(&s).unwrap();
        assert_eq!(back, h);
    }
}
