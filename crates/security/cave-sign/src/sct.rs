// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Signed Certificate Timestamps (SCTs).
//!
//! Maps to:
//!   * pkg/cosign/verify_sct.go    → VerifySCT
//!   * pkg/cosign/ctlog.go         → CTLog client
//!
//! Cosign embeds CT log SCTs in the leaf cert (or attaches them detached)
//! so the verifier can prove the Fulcio cert was actually CT-logged. The
//! full CT-log JSON-RPC client is **out of MVP** — what we ship here is
//! the SCT parse + envelope sanity check that the bundle carries one.

use crate::error::{Result, SignError};
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Sct {
    pub version: u8,
    /// Log ID — base64-encoded 32-byte SHA-256 of the CT log public key.
    pub log_id_b64: String,
    pub timestamp_ms: i64,
    /// Opaque extensions — usually empty.
    #[serde(default)]
    pub extensions_b64: String,
    /// Base64 signature over the cert tbs + timestamp + extensions.
    pub signature_b64: String,
}

impl Sct {
    pub fn parse_b64(input: &str) -> Result<Self> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(input.as_bytes())
            .map_err(|e| SignError::Cert(format!("sct base64: {}", e)))?;
        if bytes.is_empty() {
            return Err(SignError::Cert("empty sct".into()));
        }
        // For the cave-fulcio mock the SCT body is just `b"mock-sct"`.
        // For real CT-log SCTs we ship a structured parser in Phase 2.
        let s = String::from_utf8_lossy(&bytes);
        if s == "mock-sct" {
            return Ok(Self {
                version: 1,
                log_id_b64: "Y3R0LWxvZw==".into(),
                timestamp_ms: 1_700_000_000_000,
                extensions_b64: String::new(),
                signature_b64: "mock-sct-signature".into(),
            });
        }
        // Real SCTs are TLS-encoded; we surface them as opaque for now.
        Ok(Self {
            version: bytes[0],
            log_id_b64: base64::engine::general_purpose::STANDARD.encode(&bytes[1..]),
            timestamp_ms: 0,
            extensions_b64: String::new(),
            signature_b64: String::new(),
        })
    }
}

/// Sanity check: there *is* an SCT attached to the cert/bundle. Real
/// signature verification will land in Phase 2 (cave-ctlog).
pub fn require_present(sct_b64: Option<&str>) -> Result<()> {
    let s = sct_b64.ok_or_else(|| SignError::Cert("no SCT attached".into()))?;
    if s.is_empty() {
        return Err(SignError::Cert("empty SCT".into()));
    }
    Sct::parse_b64(s).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_sct_parses_to_known_shape() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"mock-sct");
        let sct = Sct::parse_b64(&b64).unwrap();
        assert_eq!(sct.version, 1);
        assert_eq!(sct.timestamp_ms, 1_700_000_000_000);
    }

    #[test]
    fn require_present_accepts_mock() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"mock-sct");
        require_present(Some(&b64)).unwrap();
    }

    #[test]
    fn require_present_rejects_none() {
        let err = require_present(None).expect_err("must reject");
        assert!(matches!(err, SignError::Cert(_)));
    }

    #[test]
    fn require_present_rejects_empty() {
        let err = require_present(Some("")).expect_err("must reject");
        assert!(matches!(err, SignError::Cert(_)));
    }

    #[test]
    fn parse_invalid_base64_fails() {
        let err = Sct::parse_b64("!!!").expect_err("must reject");
        assert!(matches!(err, SignError::Cert(_)));
    }

    #[test]
    fn parse_arbitrary_bytes_is_opaque() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"\x01abc");
        let sct = Sct::parse_b64(&b64).unwrap();
        assert_eq!(sct.version, 1);
    }
}
