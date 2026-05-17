// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/kerberos/src/main/java/org/keycloak/federation/kerberos/impl/SPNEGOAuthenticator.java + RFC 4559 (HTTP Negotiate)

//! HTTP `Negotiate` Authorization scheme. RFC 4559 wires SPNEGO
//! into HTTP — server returns `401 Unauthorized` with
//! `WWW-Authenticate: Negotiate`, the client retries with
//! `Authorization: Negotiate <base64 GSS token>`. This module
//! produces the challenge string + decodes the request header.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

use super::gssapi::InitialContextToken;
use super::spnego::NegTokenInit;
use super::KerberosError;

/// HTTP-side handler. Stateless — the caller decides what to do
/// when [`decode_request`] returns `Ok`.
pub struct NegotiateHandler;

impl Default for NegotiateHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl NegotiateHandler {
    pub fn new() -> Self {
        NegotiateHandler
    }
    /// Build the `WWW-Authenticate` 401-challenge value.
    pub fn challenge_header(&self) -> &'static str {
        "Negotiate"
    }

    /// Parse an inbound `Authorization` header. Returns the
    /// decoded SPNEGO `NegTokenInit` payload — caller can then
    /// extract the mechToken and feed it to a real GSSAPI
    /// accept_sec_context.
    ///
    /// Accepts either `"Negotiate <b64>"` or just the base64
    /// payload — RFC 4559 specifies the prefix but real clients
    /// (curl-spnego, browser SSO) both send variants. The
    /// scheme name is case-insensitive per RFC 7235 §2.1.
    pub fn decode_request(&self, header: &str) -> Result<DecodedNegotiate, KerberosError> {
        // Case-insensitive `Negotiate ` prefix strip — RFC 7235 §2.1.
        let payload = match strip_negotiate_prefix(header) {
            Some(rest) => rest.trim(),
            None => header.trim(),
        };
        if payload.is_empty() {
            return Err(KerberosError::Spnego(
                "Negotiate header carries empty token".into(),
            ));
        }
        let bytes = B64
            .decode(payload)
            .map_err(|e| KerberosError::Spnego(format!("base64 decode: {e}")))?;
        // Upstream Keycloak `KerberosUtil.MAX_TOKEN_SIZE`.
        const MAX_NEGOTIATE_TOKEN_SIZE: usize = 64 * 1024;
        if bytes.len() > MAX_NEGOTIATE_TOKEN_SIZE {
            return Err(KerberosError::Spnego(format!(
                "Negotiate token exceeds {MAX_NEGOTIATE_TOKEN_SIZE} byte limit (got {} bytes)",
                bytes.len()
            )));
        }
        // Path A: bytes start with GSSAPI wrapper (0x60). Then
        // inner is a NegTokenInit.
        if bytes.first() == Some(&0x60) {
            let gss = InitialContextToken::parse(&bytes)?;
            if !gss.is_spnego() {
                return Err(KerberosError::Spnego(
                    "GSS token mechanism is not SPNEGO".into(),
                ));
            }
            let init = NegTokenInit::parse(gss.inner)?;
            return Ok(DecodedNegotiate {
                mech_types: init.mech_types.iter().map(|s| s.to_vec()).collect(),
                mech_token: init.mech_token.map(|s| s.to_vec()),
            });
        }
        // Path B: raw NegTokenInit choice (a0 ...).
        if bytes.first() == Some(&0xa0) {
            let init = NegTokenInit::parse(&bytes)?;
            return Ok(DecodedNegotiate {
                mech_types: init.mech_types.iter().map(|s| s.to_vec()).collect(),
                mech_token: init.mech_token.map(|s| s.to_vec()),
            });
        }
        Err(KerberosError::Spnego(format!(
            "Negotiate payload has neither GSS (0x60) nor raw NegTokenInit (0xa0) outer tag — got {:#04x}",
            bytes.first().copied().unwrap_or(0)
        )))
    }

    /// Build the body of a 401-response. RFC 4559 leaves the
    /// 401 status itself to the caller (HTTP layer); we own the
    /// header value.
    pub fn unauthorized_response(&self) -> (u16, [(String, String); 1]) {
        (
            401,
            [(
                "WWW-Authenticate".to_string(),
                self.challenge_header().to_string(),
            )],
        )
    }
}

/// Strip an `"Negotiate "` prefix in any letter-case. Returns the
/// remainder when matched, `None` otherwise. The whitespace after
/// `Negotiate` is mandatory per RFC 4559 — a header that's just
/// `"Negotiate"` with no payload is structurally invalid.
fn strip_negotiate_prefix(header: &str) -> Option<&str> {
    const SCHEME: &str = "negotiate";
    let bytes = header.as_bytes();
    if bytes.len() < SCHEME.len() + 1 {
        return None;
    }
    for (i, sb) in SCHEME.as_bytes().iter().enumerate() {
        if bytes[i].to_ascii_lowercase() != *sb {
            return None;
        }
    }
    // Mandatory single space (or tab) between scheme and token.
    let sep = bytes[SCHEME.len()];
    if sep != b' ' && sep != b'\t' {
        return None;
    }
    Some(&header[SCHEME.len() + 1..])
}

/// What a successful header parse hands to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedNegotiate {
    /// SPNEGO `mechTypes` advertised by the client.
    pub mech_types: Vec<Vec<u8>>,
    /// Optional `mechToken` — typically a Kerberos AP-REQ. If
    /// present, ready to be handed to `gss_accept_sec_context`.
    pub mech_token: Option<Vec<u8>>,
}

impl DecodedNegotiate {
    /// True if at least one advertised mech is recognised as
    /// SPNEGO or Kerberos v5.
    pub fn has_known_mech(&self) -> bool {
        use super::gssapi::{OID_KRB5, OID_MS_KRB5, OID_SPNEGO};
        self.mech_types
            .iter()
            .any(|o| o == OID_SPNEGO || o == OID_KRB5 || o == OID_MS_KRB5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kerberos::gssapi::{wrap_initial_context_token, OID_KRB5, OID_SPNEGO};
    use crate::kerberos::spnego::build_neg_token_init;

    fn encode_challenge(token: &[u8]) -> String {
        let inner = build_neg_token_init(&[OID_KRB5], Some(token));
        let wrapped = wrap_initial_context_token(OID_SPNEGO, &inner);
        format!("Negotiate {}", B64.encode(wrapped))
    }

    #[test]
    fn challenge_header_value_is_negotiate() {
        let h = NegotiateHandler::new();
        assert_eq!(h.challenge_header(), "Negotiate");
    }

    #[test]
    fn unauthorized_response_returns_401_with_negotiate() {
        let h = NegotiateHandler::new();
        let (code, headers) = h.unauthorized_response();
        assert_eq!(code, 401);
        assert_eq!(headers[0].0, "WWW-Authenticate");
        assert_eq!(headers[0].1, "Negotiate");
    }

    #[test]
    fn decode_request_parses_gss_wrapped_spnego_init() {
        let h = NegotiateHandler::new();
        let header = encode_challenge(&[0xde, 0xad, 0xbe, 0xef]);
        let decoded = h.decode_request(&header).unwrap();
        assert_eq!(decoded.mech_token.as_deref(), Some(&[0xde, 0xad, 0xbe, 0xef][..]));
        assert!(decoded.has_known_mech());
    }

    #[test]
    fn decode_request_accepts_raw_negtokeninit() {
        let h = NegotiateHandler::new();
        let raw = build_neg_token_init(&[OID_KRB5], Some(&[0xff]));
        let header = format!("Negotiate {}", B64.encode(raw));
        let decoded = h.decode_request(&header).unwrap();
        assert!(decoded.has_known_mech());
    }

    #[test]
    fn decode_request_strips_lowercase_negotiate_prefix() {
        let h = NegotiateHandler::new();
        let raw = build_neg_token_init(&[OID_KRB5], None);
        let header = format!("negotiate {}", B64.encode(raw));
        let _ = h.decode_request(&header).unwrap();
    }

    #[test]
    fn decode_request_accepts_bare_base64_without_prefix() {
        let h = NegotiateHandler::new();
        let raw = build_neg_token_init(&[OID_KRB5], None);
        let header = B64.encode(raw);
        let _ = h.decode_request(&header).unwrap();
    }

    #[test]
    fn decode_request_rejects_non_base64_payload() {
        let h = NegotiateHandler::new();
        assert!(h.decode_request("Negotiate not!!base64").is_err());
    }

    #[test]
    fn decode_request_rejects_unknown_outer_tag() {
        let h = NegotiateHandler::new();
        // SEQUENCE — not SPNEGO and not GSS wrapper.
        let header = format!("Negotiate {}", B64.encode([0x30, 0x00]));
        assert!(h.decode_request(&header).is_err());
    }
}
