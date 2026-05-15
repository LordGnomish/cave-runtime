// SPDX-License-Identifier: AGPL-3.0-or-later
//
// DPoP HTTP header parsing — RFC 9449 §4 + §7.1.
//
// The `DPoP` request header carries a compact JWS (three base64url segments
// separated by `.`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Deserialize;

use super::DpopError;

/// Split a compact-JWS DPoP header into its three base64url segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHeader {
    pub header_b64: String,
    pub payload_b64: String,
    pub signature_b64: String,
}

impl ParsedHeader {
    pub fn parse(raw: &str) -> Result<Self, DpopError> {
        let parts: Vec<&str> = raw.split('.').collect();
        if parts.len() != 3 {
            return Err(DpopError::Header("expected 3 dot-separated segments"));
        }
        if parts.iter().any(|s| s.is_empty()) {
            return Err(DpopError::Header("empty segment"));
        }
        Ok(Self {
            header_b64: parts[0].to_string(),
            payload_b64: parts[1].to_string(),
            signature_b64: parts[2].to_string(),
        })
    }

    /// `BASE64URL(header) || "." || BASE64URL(payload)` — the bytes that
    /// were signed.
    pub fn signing_input(&self) -> String {
        format!("{}.{}", self.header_b64, self.payload_b64)
    }

    pub fn decode_header(&self) -> Result<DpopJwsHeader, DpopError> {
        let raw = B64.decode(self.header_b64.as_bytes())
            .map_err(|e| DpopError::Base64(e.to_string()))?;
        serde_json::from_slice(&raw).map_err(|e| DpopError::Json(e.to_string()))
    }

    pub fn decode_payload(&self) -> Result<DpopProofClaims, DpopError> {
        let raw = B64.decode(self.payload_b64.as_bytes())
            .map_err(|e| DpopError::Base64(e.to_string()))?;
        serde_json::from_slice(&raw).map_err(|e| DpopError::Json(e.to_string()))
    }

    pub fn signature_bytes(&self) -> Result<Vec<u8>, DpopError> {
        B64.decode(self.signature_b64.as_bytes())
            .map_err(|e| DpopError::Base64(e.to_string()))
    }
}

/// RFC 9449 §4.2 — DPoP proof JWS header. `typ` MUST be `"dpop+jwt"`.
#[derive(Debug, Clone, Deserialize)]
pub struct DpopJwsHeader {
    pub typ: String,
    pub alg: String,
    pub jwk: serde_json::Value,
}

/// RFC 9449 §4.2 — DPoP proof JWS payload claim set.
#[derive(Debug, Clone, Deserialize)]
pub struct DpopProofClaims {
    pub jti: String,
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    #[serde(default)]
    pub ath: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc9449 §4.1 — header is a compact JWS (3 segments). Wrong
    // count must be rejected.
    #[test]
    fn parse_rejects_wrong_segment_count() {
        assert!(ParsedHeader::parse("a.b").is_err());
        assert!(ParsedHeader::parse("a.b.c.d").is_err());
        assert!(ParsedHeader::parse("a..c").is_err());
    }

    // upstream: rfc9449 §4.1 — round-trip a syntactically valid three-segment
    // header.
    #[test]
    fn parse_round_trips_three_segments() {
        let p = ParsedHeader::parse("hh.pp.ss").unwrap();
        assert_eq!(p.header_b64, "hh");
        assert_eq!(p.payload_b64, "pp");
        assert_eq!(p.signature_b64, "ss");
        assert_eq!(p.signing_input(), "hh.pp");
    }

    // upstream: rfc9449 §4.2 — header JSON has typ + alg + jwk.
    #[test]
    fn header_json_decodes() {
        let hdr_obj = serde_json::json!({
            "typ": "dpop+jwt",
            "alg": "ES256",
            "jwk": {"kty":"EC","crv":"P-256","x":"...","y":"..."}
        });
        let hb = B64.encode(serde_json::to_vec(&hdr_obj).unwrap());
        let p = ParsedHeader::parse(&format!("{hb}.x.y")).unwrap();
        let h = p.decode_header().unwrap();
        assert_eq!(h.typ, "dpop+jwt");
        assert_eq!(h.alg, "ES256");
        assert_eq!(h.jwk["kty"], "EC");
    }

    // upstream: rfc9449 §4.2 — payload claim shape: jti, htm, htu, iat
    // mandatory; ath and nonce optional.
    #[test]
    fn payload_with_optional_ath_decodes() {
        let pl = serde_json::json!({
            "jti": "abc",
            "htm": "POST",
            "htu": "https://example.com/token",
            "iat": 1700000000,
            "ath": "fUHyO2r2Z3DZ53EsNrWBb0xWXoaNy59IiKCAqksmQEo"
        });
        let pb = B64.encode(serde_json::to_vec(&pl).unwrap());
        let p = ParsedHeader::parse(&format!("x.{pb}.y")).unwrap();
        let c = p.decode_payload().unwrap();
        assert_eq!(c.jti, "abc");
        assert_eq!(c.htm, "POST");
        assert_eq!(c.htu, "https://example.com/token");
        assert_eq!(c.iat, 1700000000);
        assert_eq!(c.ath.as_deref(), Some("fUHyO2r2Z3DZ53EsNrWBb0xWXoaNy59IiKCAqksmQEo"));
        assert_eq!(c.nonce, None);
    }
}
