// SPDX-License-Identifier: AGPL-3.0-or-later
//
// JWE Compact Serialization — RFC 7516 §3.1 / §7.1.
//
// Five base64url segments separated by `.`:
//   BASE64URL(UTF8(JWE Protected Header)) . BASE64URL(JWE Encrypted Key)
//   . BASE64URL(JWE Initialization Vector) . BASE64URL(JWE Ciphertext)
//   . BASE64URL(JWE Authentication Tag)
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/jose/jwe/JWE.java#serialize / deserialize

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

use crate::jwe::{JweError, ProtectedHeader};

/// Encoded segments of a compact JWE. Each field is base64url (no padding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactJwe {
    pub header_b64: String,
    pub encrypted_key_b64: String,
    pub iv_b64: String,
    pub ciphertext_b64: String,
    pub tag_b64: String,
}

impl CompactJwe {
    pub fn to_string(&self) -> String {
        format!(
            "{}.{}.{}.{}.{}",
            self.header_b64, self.encrypted_key_b64, self.iv_b64, self.ciphertext_b64, self.tag_b64
        )
    }

    /// Parse a compact serialized JWE.
    ///
    /// RFC 7516 §9 — the receiver verifies five segments before any crypto.
    pub fn parse(s: &str) -> Result<Self, JweError> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 5 {
            return Err(JweError::Malformed("expected 5 segments"));
        }
        Ok(Self {
            header_b64: parts[0].to_string(),
            encrypted_key_b64: parts[1].to_string(),
            iv_b64: parts[2].to_string(),
            ciphertext_b64: parts[3].to_string(),
            tag_b64: parts[4].to_string(),
        })
    }

    /// Decode the protected header JSON.
    pub fn decode_header(&self) -> Result<ProtectedHeader, JweError> {
        let raw = B64.decode(self.header_b64.as_bytes())?;
        let s = std::str::from_utf8(&raw)?;
        let hdr: ProtectedHeader = serde_json::from_str(s)?;
        Ok(hdr)
    }

    /// AAD for AEAD modes (RFC 7516 §5.1 step 14) = ASCII(BASE64URL(header)).
    pub fn aad(&self) -> Vec<u8> {
        self.header_b64.as_bytes().to_vec()
    }
}

pub fn b64url_encode(b: &[u8]) -> String {
    B64.encode(b)
}

pub fn b64url_decode(s: &str) -> Result<Vec<u8>, JweError> {
    B64.decode(s.as_bytes()).map_err(JweError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwe::{EncAlg, KeyAlg};

    // upstream: rfc7516 §3.1 — a compact JWE MUST have exactly five
    // base64url-encoded segments separated by `.`.
    #[test]
    fn parse_rejects_wrong_segment_count() {
        assert!(CompactJwe::parse("a.b.c").is_err());
        assert!(CompactJwe::parse("a.b.c.d").is_err());
        assert!(CompactJwe::parse("a.b.c.d.e.f").is_err());
    }

    // upstream: rfc7516 §3.1 — a five-segment string parses without crypto.
    #[test]
    fn parse_round_trips_five_segments() {
        let jwe = CompactJwe {
            header_b64: "hdr".into(),
            encrypted_key_b64: "k".into(),
            iv_b64: "iv".into(),
            ciphertext_b64: "ct".into(),
            tag_b64: "tag".into(),
        };
        let s = jwe.to_string();
        assert_eq!(s, "hdr.k.iv.ct.tag");
        let back = CompactJwe::parse(&s).unwrap();
        assert_eq!(back, jwe);
    }

    // upstream: rfc7516 §4 — header is the BASE64URL of UTF-8 JSON.
    #[test]
    fn decode_header_round_trip() {
        let hdr = ProtectedHeader::new(KeyAlg::RsaOaep256, EncAlg::A256Gcm);
        let json = serde_json::to_string(&hdr).unwrap();
        let header_b64 = b64url_encode(json.as_bytes());

        let jwe = CompactJwe {
            header_b64,
            encrypted_key_b64: "".into(),
            iv_b64: "".into(),
            ciphertext_b64: "".into(),
            tag_b64: "".into(),
        };
        let decoded = jwe.decode_header().unwrap();
        assert_eq!(decoded.alg, KeyAlg::RsaOaep256);
        assert_eq!(decoded.enc, EncAlg::A256Gcm);
    }

    // upstream: rfc7516 §5.1 step 14 — AAD is ASCII(BASE64URL(header)).
    #[test]
    fn aad_matches_header_b64_bytes() {
        let jwe = CompactJwe {
            header_b64: "eyJhbGciOiJSU0EtT0FFUC0yNTYifQ".into(),
            encrypted_key_b64: "".into(),
            iv_b64: "".into(),
            ciphertext_b64: "".into(),
            tag_b64: "".into(),
        };
        assert_eq!(jwe.aad(), b"eyJhbGciOiJSU0EtT0FFUC0yNTYifQ");
    }

    // upstream: rfc7516 §4.1 — invalid base64 in the header segment surfaces
    // as a parse error, not a panic.
    #[test]
    fn decode_header_rejects_garbage() {
        let jwe = CompactJwe {
            header_b64: "!!!not base64!!!".into(),
            encrypted_key_b64: "".into(),
            iv_b64: "".into(),
            ciphertext_b64: "".into(),
            tag_b64: "".into(),
        };
        assert!(jwe.decode_header().is_err());
    }
}
