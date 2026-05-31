// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! HTTP transport codec — ThingsBoard HTTP device API path router.
//!
//! Ports the `transport/http` `DeviceApiController` path grammar
//! (`/api/v1/{deviceToken}/{endpoint}`) plus the tokenless
//! `/api/v1/provision` endpoint. No HTTP server — the runtime data-plane
//! supplies `(method, path, body)`; this maps it to a typed route.

use crate::{IotError, KvMap, Result};

/// The endpoint a device HTTP request targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpEndpointKind {
    /// POST telemetry.
    Telemetry,
    /// POST client-side attribute updates.
    PostAttributes,
    /// GET shared/client attributes (optionally filtered by keys).
    GetAttributes,
    /// POST an RPC reply / subscribe to RPC.
    Rpc,
    /// POST a claiming request.
    Claim,
    /// POST `/api/v1/provision` (no device token yet).
    Provision,
}

/// A routed device HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestRoute {
    pub token: String,
    pub kind: HttpEndpointKind,
}

/// Route an HTTP `(method, path)` to a typed device endpoint.
pub fn route(method: &str, path: &str) -> Result<HttpRequestRoute> {
    let p = path.split('?').next().unwrap_or(path);
    let p = p.trim_end_matches('/');

    if p == "/api/v1/provision" {
        if method != "POST" {
            return Err(IotError::Codec("provision requires POST".into()));
        }
        return Ok(HttpRequestRoute {
            token: String::new(),
            kind: HttpEndpointKind::Provision,
        });
    }

    let rest = p
        .strip_prefix("/api/v1/")
        .ok_or_else(|| IotError::Codec(format!("not a device API path '{path}'")))?;
    let (token, endpoint) = rest
        .split_once('/')
        .ok_or_else(|| IotError::Codec("missing endpoint segment".into()))?;
    if token.is_empty() {
        return Err(IotError::Codec("empty device token".into()));
    }
    let kind = match (method, endpoint) {
        ("POST", "telemetry") => HttpEndpointKind::Telemetry,
        ("POST", "attributes") => HttpEndpointKind::PostAttributes,
        ("GET", "attributes") => HttpEndpointKind::GetAttributes,
        ("POST", "rpc") => HttpEndpointKind::Rpc,
        ("POST", "claim") => HttpEndpointKind::Claim,
        (m, e) => {
            return Err(IotError::Codec(format!(
                "unsupported device endpoint {m} {e}"
            )));
        }
    };
    Ok(HttpRequestRoute {
        token: token.to_string(),
        kind,
    })
}

/// Parse the `clientKeys` / `sharedKeys` attribute-filter query string into
/// `(client_keys, shared_keys)`.
pub fn parse_attribute_keys(query: &str) -> (Vec<String>, Vec<String>) {
    let mut client = Vec::new();
    let mut shared = Vec::new();
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let vals: Vec<String> = v
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            match k {
                "clientKeys" => client = vals,
                "sharedKeys" => shared = vals,
                _ => {}
            }
        }
    }
    (client, shared)
}

/// Parse an HTTP device request body (JSON) into a KvMap, reusing the MQTT
/// telemetry decoder (flat + `{ts,values}` forms).
pub fn parse_body(body: &[u8]) -> Result<KvMap> {
    super::mqtt::parse_telemetry(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_telemetry_post() {
        let r = route("POST", "/api/v1/MYTOKEN/telemetry").unwrap();
        assert_eq!(r.token, "MYTOKEN");
        assert_eq!(r.kind, HttpEndpointKind::Telemetry);
    }

    #[test]
    fn routes_attributes_post_and_get() {
        let post = route("POST", "/api/v1/T/attributes").unwrap();
        assert_eq!(post.kind, HttpEndpointKind::PostAttributes);
        let get = route("GET", "/api/v1/T/attributes").unwrap();
        assert_eq!(get.kind, HttpEndpointKind::GetAttributes);
    }

    #[test]
    fn routes_rpc_and_claim() {
        assert_eq!(
            route("POST", "/api/v1/T/rpc").unwrap().kind,
            HttpEndpointKind::Rpc
        );
        assert_eq!(
            route("POST", "/api/v1/T/claim").unwrap().kind,
            HttpEndpointKind::Claim
        );
    }

    #[test]
    fn routes_provision_without_token() {
        let r = route("POST", "/api/v1/provision").unwrap();
        assert_eq!(r.token, "");
        assert_eq!(r.kind, HttpEndpointKind::Provision);
    }

    #[test]
    fn rejects_unknown_path_and_method() {
        assert!(route("POST", "/api/v2/T/telemetry").is_err());
        assert!(route("DELETE", "/api/v1/T/telemetry").is_err());
        assert!(route("POST", "/nope").is_err());
    }

    #[test]
    fn parses_attribute_key_query() {
        let (client, shared) = parse_attribute_keys("clientKeys=a,b&sharedKeys=c");
        assert_eq!(client, vec!["a", "b"]);
        assert_eq!(shared, vec!["c"]);
        let (c2, s2) = parse_attribute_keys("sharedKeys=x");
        assert!(c2.is_empty());
        assert_eq!(s2, vec!["x"]);
    }

    #[test]
    fn parses_body_telemetry() {
        let kv = parse_body(br#"{"speed":55}"#).unwrap();
        assert_eq!(kv.get("speed"), Some(&crate::KvValue::Long(55)));
    }
}
