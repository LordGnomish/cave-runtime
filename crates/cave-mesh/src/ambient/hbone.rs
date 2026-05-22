// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HBONE — HTTP-Based Overlay Network Environment.
//!
//! Mirrors `pkg/hbone/server.go` in upstream Istio v1.29.2 (and the
//! companion request-construction code in `pkg/hbone/dialer.go`).
//!
//! Wire shape: an HTTP/2 `CONNECT` request whose `:authority` is the target
//! `<host>:<port>`, with optional Istio-specific headers:
//!
//! ```text
//! :method     = CONNECT
//! :authority  = 10.0.0.42:8080
//! :path       = /                  (REQUIRED to be "/", per upstream)
//! :scheme     = https
//! Baggage      = k1=v1,k2=v2       (W3C Baggage; carries tenant + workload)
//! ```
//!
//! The peer responds `200 OK` and the body is the bidirectional L4 stream.

use crate::ambient::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

/// Header carrying W3C Baggage (used by Istio for workload identity hints).
pub const HEADER_BAGGAGE: &str = "baggage";
/// Authority pseudo-header.
pub const PSEUDO_AUTHORITY: &str = ":authority";
/// Method pseudo-header.
pub const PSEUDO_METHOD: &str = ":method";
/// Path pseudo-header — MUST be `/`.
pub const PSEUDO_PATH: &str = ":path";

/// Errors that can come out of HBONE request parsing.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HboneError {
    #[error("missing pseudo-header {0}")]
    MissingPseudo(&'static str),
    #[error("method must be CONNECT, got {0}")]
    NotConnect(String),
    #[error("authority must be host:port, got {0}")]
    BadAuthority(String),
    #[error("path must be \"/\" for CONNECT, got {0}")]
    BadPath(String),
    #[error("port {0} is not a valid TCP port")]
    BadPort(String),
    #[error("tenant {tenant} is not authorised to tunnel to {target}")]
    TenantDenied { tenant: TenantId, target: String },
}

/// A parsed HBONE CONNECT request. Borrowed shape so a parser can produce it
/// without allocating beyond what the caller already owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HboneRequest {
    pub host: String,
    pub port: u16,
    /// W3C Baggage entries (e.g. `("tenant", "acme")`). Order preserved.
    pub baggage: Vec<(String, String)>,
}

impl HboneRequest {
    /// Read the tenant out of the Baggage header, if any.
    pub fn baggage_tenant(&self) -> Option<&str> {
        self.baggage
            .iter()
            .find(|(k, _)| k == "tenant")
            .map(|(_, v)| v.as_str())
    }

    /// Concatenate `host` + `:` + `port` — what goes into `:authority`.
    pub fn target(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Parse the four HTTP/2 pseudo-headers + zero-or-more regular headers into
/// an `HboneRequest`.
///
/// The input mirrors the `(name, value)` pair representation that an HTTP/2
/// HPACK decoder emits.
pub fn parse_request(headers: &[(&str, &str)]) -> Result<HboneRequest, HboneError> {
    let mut method = None;
    let mut authority = None;
    let mut path = None;
    let mut baggage = Vec::new();

    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            PSEUDO_METHOD => method = Some(value.to_string()),
            PSEUDO_AUTHORITY => authority = Some(value.to_string()),
            PSEUDO_PATH => path = Some(value.to_string()),
            HEADER_BAGGAGE => baggage = parse_baggage(value),
            _ => {}
        }
    }

    let method = method.ok_or(HboneError::MissingPseudo(PSEUDO_METHOD))?;
    if method != "CONNECT" {
        return Err(HboneError::NotConnect(method));
    }
    let authority = authority.ok_or(HboneError::MissingPseudo(PSEUDO_AUTHORITY))?;
    let path = path.ok_or(HboneError::MissingPseudo(PSEUDO_PATH))?;
    if path != "/" {
        return Err(HboneError::BadPath(path));
    }

    let (host, port) = split_authority(&authority)?;
    Ok(HboneRequest {
        host,
        port,
        baggage,
    })
}

fn split_authority(s: &str) -> Result<(String, u16), HboneError> {
    let (h, p) = s
        .rsplit_once(':')
        .ok_or_else(|| HboneError::BadAuthority(s.into()))?;
    if h.is_empty() {
        return Err(HboneError::BadAuthority(s.into()));
    }
    let port: u16 = p.parse().map_err(|_| HboneError::BadPort(p.into()))?;
    if port == 0 {
        return Err(HboneError::BadPort(p.into()));
    }
    Ok((h.to_string(), port))
}

/// W3C Baggage parser (RFC editor draft `baggage-08`): comma-separated
/// `key=value` pairs, optional whitespace around the `=`. Property suffixes
/// (`;p1`) are dropped.
pub fn parse_baggage(s: &str) -> Vec<(String, String)> {
    s.split(',')
        .filter_map(|entry| {
            let entry = entry.split(';').next().unwrap_or("").trim();
            let (k, v) = entry.split_once('=')?;
            let k = k.trim();
            let v = v.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// Authorise a CONNECT for a given tenant. The tenant must match the value
/// in the Baggage header — this is the per-request tenant gate.
pub fn authorise(req: &HboneRequest, tenant: &TenantId) -> Result<(), HboneError> {
    match req.baggage_tenant() {
        Some(t) if t == tenant.as_str() => Ok(()),
        _ => Err(HboneError::TenantDenied {
            tenant: tenant.clone(),
            target: req.target(),
        }),
    }
}

/// HTTP/2 status that an HBONE server returns on success.
pub const HBONE_OK_STATUS: u16 = 200;

/// Build the response headers an HBONE server returns when accepting the
/// CONNECT. Mirrors the constant set written by upstream's `serveHTTP`.
pub fn accept_response_headers() -> Vec<(&'static str, String)> {
    vec![(":status", HBONE_OK_STATUS.to_string())]
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::istio("pkg/hbone/server.go", "HBONEServer");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn h<'a>(
        method: &'a str,
        authority: &'a str,
        path: &'a str,
        baggage: Option<&'a str>,
    ) -> Vec<(&'a str, &'a str)> {
        let mut v = vec![
            (":method", method),
            (":authority", authority),
            (":path", path),
        ];
        if let Some(b) = baggage {
            v.push(("baggage", b));
        }
        v
    }

    #[test]
    fn parses_valid_connect_request() {
        let (_cite, tenant) = ambient_test_ctx!("pkg/hbone/server.go", "ServeHTTP", "acme");
        let req = parse_request(&h(
            "CONNECT",
            "10.0.0.42:8080",
            "/",
            Some("tenant=acme,workload=web"),
        ))
        .unwrap();
        assert_eq!(req.host, "10.0.0.42");
        assert_eq!(req.port, 8080);
        assert_eq!(req.target(), "10.0.0.42:8080");
        assert_eq!(req.baggage_tenant(), Some("acme"));
        assert!(authorise(&req, &tenant).is_ok());
    }

    #[test]
    fn rejects_non_connect_method() {
        let (_cite, _t) = ambient_test_ctx!(
            "pkg/hbone/server.go",
            "ServeHTTP",
            "tenant-hbone-not-connect"
        );
        let err = parse_request(&h("GET", "10.0.0.42:8080", "/", None)).unwrap_err();
        assert!(matches!(err, HboneError::NotConnect(_)));
    }

    #[test]
    fn rejects_non_root_path() {
        let (_cite, _t) =
            ambient_test_ctx!("pkg/hbone/server.go", "ServeHTTP", "tenant-hbone-bad-path");
        let err = parse_request(&h("CONNECT", "10.0.0.42:8080", "/v1", None)).unwrap_err();
        assert!(matches!(err, HboneError::BadPath(_)));
    }

    #[test]
    fn rejects_authority_without_port() {
        let (_cite, _t) =
            ambient_test_ctx!("pkg/hbone/server.go", "ServeHTTP", "tenant-hbone-bad-auth");
        let err = parse_request(&h("CONNECT", "10.0.0.42", "/", None)).unwrap_err();
        assert!(matches!(err, HboneError::BadAuthority(_)));
    }

    #[test]
    fn rejects_zero_port() {
        let (_cite, _t) =
            ambient_test_ctx!("pkg/hbone/server.go", "ServeHTTP", "tenant-hbone-zero-port");
        let err = parse_request(&h("CONNECT", "10.0.0.42:0", "/", None)).unwrap_err();
        assert!(matches!(err, HboneError::BadPort(_)));
    }

    #[test]
    fn baggage_tolerates_whitespace_and_property_suffix() {
        let (_cite, _t) = ambient_test_ctx!(
            "pkg/hbone/server.go",
            "parseBaggage",
            "tenant-hbone-baggage-fmt"
        );
        let parsed = parse_baggage("tenant = acme ;property, workload= web ;a=b");
        assert_eq!(
            parsed,
            vec![
                ("tenant".to_string(), "acme".to_string()),
                ("workload".to_string(), "web".to_string()),
            ]
        );
    }

    #[test]
    fn authorise_refuses_when_baggage_tenant_mismatches() {
        let (_cite, attacker) =
            ambient_test_ctx!("pkg/hbone/server.go", "ServeHTTP", "tenant-attacker");
        let req = parse_request(&h("CONNECT", "10.0.0.42:8080", "/", Some("tenant=acme"))).unwrap();
        let err = authorise(&req, &attacker).unwrap_err();
        assert!(matches!(err, HboneError::TenantDenied { .. }));
    }

    #[test]
    fn accept_response_uses_status_200() {
        let (_cite, _t) =
            ambient_test_ctx!("pkg/hbone/server.go", "writeResponse", "tenant-hbone-resp");
        let r = accept_response_headers();
        assert_eq!(r.first().map(|(k, _)| *k), Some(":status"));
        assert_eq!(r.first().map(|(_, v)| v.as_str()), Some("200"));
    }
}
