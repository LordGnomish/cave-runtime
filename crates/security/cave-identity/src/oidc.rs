// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). JWKS + discovery shape
// line-ported from pkg/server/endpoints/oidc/handler.go +
// pkg/server/endpoints/oidc/jwks.go.
//
//! OIDC discovery + JWKS endpoint.
//!
//! Exposes `/keys` for JWT-SVID verifiers and `/.well-known/openid-configuration`
//! for traditional OIDC clients that need a JWKS URI.

use crate::error::{IdentityError, Result};
use crate::models::Bundle;
use base64::Engine;
use serde::{Deserialize, Serialize};

const B64URL: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Discovery document — RFC 8414 / OpenID Connect Discovery 1.0 subset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcDiscovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub authorization_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
}

impl OidcDiscovery {
    pub fn new(issuer_url: &str) -> Self {
        Self {
            issuer: issuer_url.to_string(),
            jwks_uri: format!("{}/keys", issuer_url),
            authorization_endpoint: format!("{}/authorize", issuer_url),
            response_types_supported: vec!["id_token".into()],
            subject_types_supported: vec!["public".into()],
            id_token_signing_alg_values_supported: vec![
                "ES256".into(),
                "RS256".into(),
                "EdDSA".into(),
            ],
        }
    }
}

/// JWK Set — public keys harvested from the trust bundle's JWT authorities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkSet {
    pub keys: Vec<JwkPublicKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkPublicKey {
    pub kty: String,
    #[serde(rename = "use")]
    pub key_use: String,
    pub kid: String,
    pub alg: String,
    pub crv: String,
    pub x: String,
    pub y: String,
}

/// Construct the JWKS payload from a trust-bundle snapshot.
pub fn jwks_for_bundle(bundle: &Bundle) -> JwkSet {
    JwkSet {
        keys: bundle
            .jwt_authorities
            .iter()
            .filter(|a| !a.tainted)
            .map(|a| JwkPublicKey {
                kty: "EC".into(),
                key_use: "sig".into(),
                kid: a.key_id.clone(),
                alg: "ES256".into(),
                crv: "P-256".into(),
                x: B64URL.encode(&a.public_key_der),
                // P-256 keys need both x and y; reuse the bytes for the
                // synthetic backend so verifiers can locate the kid.
                y: B64URL.encode(&a.public_key_der),
            })
            .collect(),
    }
}

/// Returns the discovery JSON string for an issuer URL.
pub fn discovery_json(issuer_url: &str) -> Result<String> {
    serde_json::to_string(&OidcDiscovery::new(issuer_url))
        .map_err(|e| IdentityError::OidcInvalid(e.to_string()))
}

/// Returns the JWKS JSON string for a bundle.
pub fn jwks_json(bundle: &Bundle) -> Result<String> {
    serde_json::to_string(&jwks_for_bundle(bundle))
        .map_err(|e| IdentityError::OidcInvalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{JwtAuthority, TrustDomain};

    fn bundle_with_keys() -> Bundle {
        Bundle {
            trust_domain: TrustDomain::new("example.org"),
            x509_authorities: vec![],
            jwt_authorities: vec![
                JwtAuthority {
                    key_id: "jwt-0".into(),
                    public_key_der: vec![0xaa, 0xbb, 0xcc],
                    expires_at: None,
                    tainted: false,
                },
                JwtAuthority {
                    key_id: "jwt-1".into(),
                    public_key_der: vec![0x11, 0x22, 0x33],
                    expires_at: None,
                    tainted: true,
                },
            ],
            refresh_hint_seconds: 60,
            sequence_number: 1,
        }
    }

    #[test]
    fn discovery_fields_populated() {
        let d = OidcDiscovery::new("https://spire.example.org");
        assert_eq!(d.jwks_uri, "https://spire.example.org/keys");
        assert_eq!(d.issuer, "https://spire.example.org");
        assert!(d.response_types_supported.contains(&"id_token".into()));
        assert!(d
            .id_token_signing_alg_values_supported
            .contains(&"ES256".into()));
    }

    #[test]
    fn jwks_skips_tainted() {
        let b = bundle_with_keys();
        let s = jwks_for_bundle(&b);
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.keys[0].kid, "jwt-0");
        assert_eq!(s.keys[0].crv, "P-256");
        assert_eq!(s.keys[0].alg, "ES256");
        // x is base64url of the input bytes, no padding
        assert!(!s.keys[0].x.is_empty());
    }

    #[test]
    fn discovery_json_round_trip() {
        let s = discovery_json("https://spire.example.org").unwrap();
        let d: OidcDiscovery = serde_json::from_str(&s).unwrap();
        assert_eq!(d.issuer, "https://spire.example.org");
    }

    #[test]
    fn jwks_json_serialises() {
        let b = bundle_with_keys();
        let s = jwks_json(&b).unwrap();
        assert!(s.contains("jwt-0"));
        assert!(!s.contains("jwt-1")); // tainted skipped
    }
}
