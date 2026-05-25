// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0); bundle-doc format
// line-ported from pkg/common/bundleutil/marshal.go +
// https://github.com/spiffe/spiffe/blob/main/standards/SPIFFE_Trust_Domain_and_Bundle.md
//
//! SPIFFE trust-bundle serialisation — the JWKS+x5c "spiffe.io bundle"
//! profile.

use crate::error::{IdentityError, Result};
use crate::models::{Bundle, JwtAuthority, TrustDomain, X509Authority};
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Bundle document — IANA JWK Set with SPIFFE extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleDoc {
    pub keys: Vec<JwkEntry>,
    /// SPIFFE-extension: refresh hint (s).
    #[serde(rename = "spiffe_refresh_hint", default)]
    pub spiffe_refresh_hint: u64,
    #[serde(rename = "spiffe_sequence", default)]
    pub spiffe_sequence: u64,
}

/// JWK entry — covers both X.509 (use=x509-svid) and JWT (use=jwt-svid).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkEntry {
    pub kty: String,
    #[serde(rename = "use")]
    pub key_use: String,
    pub kid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crv: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub e: Option<String>,
    /// X.509 chain (base64-DER); present for `use=x509-svid` entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x5c: Option<Vec<String>>,
    /// SPIFFE-tainted marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spiffe_tainted: Option<bool>,
}

const B64: base64::engine::general_purpose::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Marshal a [`Bundle`] into the JWKS document.
pub fn marshal(b: &Bundle) -> BundleDoc {
    let mut keys = Vec::new();
    for (i, x) in b.x509_authorities.iter().enumerate() {
        keys.push(JwkEntry {
            kty: "RSA".to_string(),
            key_use: "x509-svid".to_string(),
            kid: format!("x509-{}", i),
            crv: None,
            x: None,
            y: None,
            n: None,
            e: None,
            x5c: Some(vec![B64.encode(&x.asn1_der)]),
            spiffe_tainted: if x.tainted { Some(true) } else { None },
        });
    }
    for j in &b.jwt_authorities {
        keys.push(JwkEntry {
            kty: "EC".to_string(),
            key_use: "jwt-svid".to_string(),
            kid: j.key_id.clone(),
            crv: Some("P-256".to_string()),
            x: Some(B64.encode(&j.public_key_der)),
            y: Some(B64.encode(&j.public_key_der)),
            n: None,
            e: None,
            x5c: None,
            spiffe_tainted: if j.tainted { Some(true) } else { None },
        });
    }
    BundleDoc {
        keys,
        spiffe_refresh_hint: b.refresh_hint_seconds,
        spiffe_sequence: b.sequence_number,
    }
}

/// Unmarshal a JWKS document back into a [`Bundle`].
pub fn unmarshal(td: &TrustDomain, doc: &BundleDoc) -> Result<Bundle> {
    let mut x509_authorities = Vec::new();
    let mut jwt_authorities = Vec::new();
    for k in &doc.keys {
        match k.key_use.as_str() {
            "x509-svid" => {
                let chain = k
                    .x5c
                    .as_ref()
                    .ok_or_else(|| IdentityError::Internal("x509-svid missing x5c".into()))?;
                let leaf = chain
                    .first()
                    .ok_or_else(|| IdentityError::Internal("empty x5c".into()))?;
                let der = B64
                    .decode(leaf)
                    .map_err(|e| IdentityError::Internal(format!("base64: {}", e)))?;
                x509_authorities.push(X509Authority {
                    asn1_der: der,
                    tainted: k.spiffe_tainted.unwrap_or(false),
                });
            }
            "jwt-svid" => {
                let x = k
                    .x
                    .as_ref()
                    .ok_or_else(|| IdentityError::Internal("jwt-svid missing x".into()))?;
                let pk = B64
                    .decode(x)
                    .map_err(|e| IdentityError::Internal(format!("base64: {}", e)))?;
                jwt_authorities.push(JwtAuthority {
                    key_id: k.kid.clone(),
                    public_key_der: pk,
                    expires_at: None,
                    tainted: k.spiffe_tainted.unwrap_or(false),
                });
            }
            other => {
                return Err(IdentityError::Internal(format!(
                    "unknown bundle key use: {}",
                    other
                )))
            }
        }
    }
    Ok(Bundle {
        trust_domain: td.clone(),
        x509_authorities,
        jwt_authorities,
        refresh_hint_seconds: doc.spiffe_refresh_hint,
        sequence_number: doc.spiffe_sequence,
    })
}

/// Convenience: marshal directly to a JSON string.
pub fn to_json(b: &Bundle) -> Result<String> {
    Ok(serde_json::to_string(&marshal(b))?)
}

/// Convenience: parse from a JSON string.
pub fn from_json(td: &TrustDomain, s: &str) -> Result<Bundle> {
    let doc: BundleDoc = serde_json::from_str(s)?;
    unmarshal(td, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bundle() -> Bundle {
        Bundle {
            trust_domain: TrustDomain::new("example.org"),
            x509_authorities: vec![X509Authority {
                asn1_der: vec![0x30, 0x82, 0x01, 0x22],
                tainted: false,
            }],
            jwt_authorities: vec![JwtAuthority {
                key_id: "jwt-0".to_string(),
                public_key_der: vec![0xde, 0xad, 0xbe, 0xef],
                expires_at: None,
                tainted: false,
            }],
            refresh_hint_seconds: 300,
            sequence_number: 1,
        }
    }

    #[test]
    fn round_trips_bundle() {
        let b = make_bundle();
        let s = to_json(&b).unwrap();
        let b2 = from_json(&b.trust_domain, &s).unwrap();
        assert_eq!(b2.x509_authorities.len(), 1);
        assert_eq!(b2.jwt_authorities.len(), 1);
        assert_eq!(b2.jwt_authorities[0].key_id, "jwt-0");
        assert_eq!(b2.sequence_number, 1);
        assert_eq!(b2.refresh_hint_seconds, 300);
    }

    #[test]
    fn marshal_includes_taint() {
        let mut b = make_bundle();
        b.x509_authorities[0].tainted = true;
        let doc = marshal(&b);
        assert_eq!(doc.keys[0].spiffe_tainted, Some(true));
    }

    #[test]
    fn unmarshal_rejects_unknown_use() {
        let doc = BundleDoc {
            keys: vec![JwkEntry {
                kty: "EC".into(),
                key_use: "unknown".into(),
                kid: "k".into(),
                crv: None,
                x: None,
                y: None,
                n: None,
                e: None,
                x5c: None,
                spiffe_tainted: None,
            }],
            spiffe_refresh_hint: 0,
            spiffe_sequence: 0,
        };
        assert!(unmarshal(&TrustDomain::new("example.org"), &doc).is_err());
    }
}
