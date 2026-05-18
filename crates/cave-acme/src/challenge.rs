// SPDX-License-Identifier: AGPL-3.0-or-later
//! ACMEv2 challenge types.
//!
//! Cite: RFC 8555 §8 (Challenge object), §8.3 (HTTP-01),
//! §8.4 (DNS-01), §8.5 (TLS-ALPN-01 — RFC 8737).

use crate::account::Jwk;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChallengeType {
    /// `http-01` — RFC 8555 §8.3.
    Http01,
    /// `dns-01`  — RFC 8555 §8.4.
    Dns01,
    /// `tls-alpn-01` — RFC 8737.
    TlsAlpn01,
}

impl ChallengeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http01     => "http-01",
            Self::Dns01      => "dns-01",
            Self::TlsAlpn01  => "tls-alpn-01",
        }
    }
}

/// Cite: RFC 8555 §7.1.6 (Challenge status). `valid` and `invalid` are
/// terminal; `pending → processing → valid|invalid` is the forward path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChallengeStatus { Pending, Processing, Valid, Invalid }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub id: String,
    pub kind: ChallengeType,
    pub status: ChallengeStatus,
    pub url: String,
    /// Cite: RFC 8555 §8.1 — random 256-bit token, base64url, no padding.
    pub token: String,
    pub validated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
}

impl Challenge {
    /// Cite: RFC 8555 §8.3 — HTTP-01 publishes the key authorisation
    /// verbatim at `http://<domain>/.well-known/acme-challenge/<token>`.
    pub fn http01_resource_path(&self) -> String {
        format!("/.well-known/acme-challenge/{}", self.token)
    }

    /// Cite: RFC 8555 §8.3 — the response body is the bare key
    /// authorisation, not JSON.
    pub fn http01_response_body(&self, jwk: &Jwk) -> String {
        jwk.key_authorization(&self.token)
    }

    /// Cite: RFC 8555 §8.4 — DNS-01 publishes a TXT record at
    /// `_acme-challenge.<domain>` containing the base64url-encoded
    /// SHA-256 digest of the key authorisation.
    pub fn dns01_record_name(&self, domain: &str) -> String {
        format!("_acme-challenge.{}", domain)
    }

    pub fn dns01_record_value(&self, jwk: &Jwk) -> String {
        let key_auth = jwk.key_authorization(&self.token);
        let digest = Sha256::digest(key_auth.as_bytes());
        URL_SAFE_NO_PAD.encode(digest)
    }

    /// Cite: RFC 8737 §3 — TLS-ALPN-01 places the SHA-256 of the key
    /// authorization (32 bytes) in the `id-pe-acmeIdentifier` extension
    /// of a self-signed certificate served via the ACME ALPN protocol.
    pub fn tls_alpn01_extension_value(&self, jwk: &Jwk) -> [u8; 32] {
        let key_auth = jwk.key_authorization(&self.token);
        Sha256::digest(key_auth.as_bytes()).into()
    }
}
