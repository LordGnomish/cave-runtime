// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ACMEv2 Account model.
//!
//! Cite: RFC 8555 §7.1.2 (Account object), §7.3 (newAccount workflow),
//! §7.3.4 (External Account Binding), §8.1 (key authorization
//! computation = `<token>.<jwk-thumbprint-base64url>`).

use crate::error::{AcmeError, AcmeResult};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Cite: RFC 8555 §7.1.2 (Account.status).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountStatus { Valid, Deactivated, Revoked }

/// Minimal JWK shape covering the two key types ACME servers MUST support
/// (RFC 8555 §6.2). We only carry the fields used for the JWK thumbprint
/// (RFC 7638) so order in the canonical JSON is deterministic.
///
/// Cite: RFC 7638 §3 — the JWK thumbprint is the base64url(SHA-256(
///   canonical_json_with_sorted_required_keys )).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kty")]
pub enum Jwk {
    /// EC keys: `kty=EC`, required members are crv, x, y.
    EC {
        crv: String,
        x: String,
        y: String,
    },
    /// RSA keys: `kty=RSA`, required members are e, n.
    RSA {
        e: String,
        n: String,
    },
    /// Ed25519 keys: `kty=OKP`, `crv=Ed25519`, `x=...`.
    OKP {
        crv: String,
        x: String,
    },
}

impl Jwk {
    /// Cite: RFC 7638 §3.2 — the canonical representation lists the
    /// required members in alphabetical order with no extra whitespace.
    pub fn thumbprint(&self) -> String {
        let mut map: BTreeMap<&str, String> = BTreeMap::new();
        match self {
            Jwk::EC { crv, x, y } => {
                map.insert("crv", crv.clone());
                map.insert("kty", "EC".into());
                map.insert("x",   x.clone());
                map.insert("y",   y.clone());
            }
            Jwk::RSA { e, n } => {
                map.insert("e",   e.clone());
                map.insert("kty", "RSA".into());
                map.insert("n",   n.clone());
            }
            Jwk::OKP { crv, x } => {
                map.insert("crv", crv.clone());
                map.insert("kty", "OKP".into());
                map.insert("x",   x.clone());
            }
        }
        let canonical = serde_json::to_string(&map).expect("BTreeMap serialises");
        let digest = Sha256::digest(canonical.as_bytes());
        URL_SAFE_NO_PAD.encode(digest)
    }

    /// Cite: RFC 8555 §8.1 — key authorization = `<token>.<jwk-thumbprint>`.
    /// This pair is what the challenge solver publishes (DNS TXT record,
    /// HTTP `.well-known` file, or TLS-ALPN-01 SAN).
    pub fn key_authorization(&self, token: &str) -> String {
        format!("{}.{}", token, self.thumbprint())
    }
}

/// Cite: RFC 8555 §7.3.4 (External Account Binding) — when the server
/// requires a pre-existing operator approval, the new-account request
/// carries a JWS over a binding payload signed with an EAB MAC key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAccountBinding {
    /// Operator-issued key id (HMAC key reference).
    pub kid: String,
    /// HMAC algorithm; cave only supports HS256 today.
    pub alg: String,
    /// base64url-encoded HMAC of the new-account JWS payload.
    pub mac: String,
}

impl ExternalAccountBinding {
    /// `urn:ietf:params:acme:error:malformed` when alg is not the one
    /// supported algorithm.
    pub fn validate_algorithm(&self) -> AcmeResult<()> {
        if self.alg != "HS256" {
            return Err(AcmeError::Malformed(format!(
                "unsupported EAB alg '{}', only HS256 is supported",
                self.alg
            )));
        }
        if self.kid.trim().is_empty() {
            return Err(AcmeError::Malformed("EAB kid is empty".into()));
        }
        if self.mac.trim().is_empty() {
            return Err(AcmeError::Malformed("EAB mac is empty".into()));
        }
        Ok(())
    }
}

/// Cite: RFC 8555 §7.1.2 — Account object stored server-side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub tenant_id: String,
    pub status: AccountStatus,
    pub contact: Vec<String>,
    pub terms_of_service_agreed: bool,
    pub jwk: Jwk,
    pub eab: Option<ExternalAccountBinding>,
    pub created_at: DateTime<Utc>,
}

impl Account {
    /// Cite: RFC 8555 §7.3 — new-account validates contact URLs (mailto:
    /// scheme), ToS agreement, and (when required) the EAB.
    pub fn validate(&self) -> AcmeResult<()> {
        for c in &self.contact {
            if !c.starts_with("mailto:") {
                return Err(AcmeError::Malformed(format!(
                    "contact URL '{}' must use mailto: scheme",
                    c,
                )));
            }
        }
        if !self.terms_of_service_agreed {
            return Err(AcmeError::Malformed("terms of service must be agreed".into()));
        }
        if let Some(eab) = &self.eab {
            eab.validate_algorithm()?;
        }
        Ok(())
    }
}
