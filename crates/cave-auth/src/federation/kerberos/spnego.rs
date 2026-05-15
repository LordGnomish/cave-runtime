// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/kerberos/src/main/java/org/keycloak/federation/kerberos/impl/SPNEGOAuthenticator.java
//
// SPNEGO (RFC 4178) state machine for HTTP `WWW-Authenticate:
// Negotiate` exchanges.  We parse the GSS-API InitialContextToken
// envelope (RFC 2743 §3.1) and the inner NegTokenInit / NegTokenResp.
//
// We *do not* perform cryptographic verification of the wrapped
// AP-REQ ticket — that requires linking libgssapi.  Instead, we
// surface an [`AuthState::AwaitingExternalVerify`] state with the
// raw token bytes attached so a higher-level integration can hand
// it to `gss_init_sec_context()` once linked.
//
// Token shape:
//
//   InitialContextToken ::=
//       [APPLICATION 0] IMPLICIT SEQUENCE {
//           thisMech       MechType,
//           innerContextToken ANY DEFINED BY thisMech }
//
//   NegTokenInit ::= SEQUENCE {
//       mechTypes      [0] MechTypeList,
//       reqFlags       [1] ContextFlags                  OPTIONAL,
//       mechToken      [2] OCTET STRING                  OPTIONAL,
//       mechListMIC    [3] OCTET STRING                  OPTIONAL }
//
//   NegTokenResp ::= SEQUENCE {
//       negState       [0] ENUMERATED {…}                OPTIONAL,
//       supportedMech  [1] MechType                      OPTIONAL,
//       responseToken  [2] OCTET STRING                  OPTIONAL,
//       mechListMIC    [3] OCTET STRING                  OPTIONAL }

use crate::federation::ldap::ber::{self, Decoder, Element, Form, Tag};
use crate::federation::provider::FederationError;

/// Well-known mechanism OIDs.
pub mod oid {
    pub const SPNEGO: &[u8] = &[0x06, 0x06, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x02];
    /// 1.2.840.113554.1.2.2 — Kerberos v5.
    pub const KRB5: &[u8] = &[0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x12, 0x01, 0x02, 0x02];
    /// 1.2.840.48018.1.2.2 — Microsoft KRB5 (legacy alias).
    pub const MS_KRB5: &[u8] = &[0x06, 0x09, 0x2a, 0x86, 0x48, 0x82, 0xf7, 0x12, 0x01, 0x02, 0x02];
    /// 1.3.6.1.4.1.311.2.2.10 — NTLM SSP.
    pub const NTLM_SSP: &[u8] = &[0x06, 0x0a, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0a];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTokenInit {
    pub mech_types: Vec<Vec<u8>>,
    pub mech_token: Option<Vec<u8>>,
    pub mech_list_mic: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTokenResp {
    pub neg_state: Option<NegState>,
    pub supported_mech: Option<Vec<u8>>,
    pub response_token: Option<Vec<u8>>,
    pub mech_list_mic: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NegState {
    AcceptCompleted = 0,
    AcceptIncomplete = 1,
    Reject = 2,
    RequestMic = 3,
}

impl NegState {
    pub fn from_i64(v: i64) -> Option<Self> {
        Some(match v {
            0 => NegState::AcceptCompleted,
            1 => NegState::AcceptIncomplete,
            2 => NegState::Reject,
            3 => NegState::RequestMic,
            _ => return None,
        })
    }
}

/// Top-level SPNEGO state.
pub enum Spnego {}

/// Outcome of a single round-trip exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    /// Token parsed; cryptographic verification deferred to a
    /// higher-level GSSAPI shim.  `mech_token` is the AP-REQ.
    AwaitingExternalVerify {
        mech_token: Vec<u8>,
        chosen_mech: Vec<u8>,
    },
    /// Client offered no Kerberos mech; SPNEGO will fall back to
    /// NTLM (or whatever else).  Caller may emit a 401 with
    /// `Negotiate` + `NTLM` to challenge further.
    UnsupportedMechs,
    /// `negState=accept_completed` was received.  Authentication
    /// finished successfully.
    Completed,
    /// `negState=reject`.  Caller should return 401.
    Rejected,
    /// `negState=request-mic` — server demands a MIC over the
    /// mech list.  We surface the token unchanged.
    NeedsMic { token: Vec<u8> },
}

impl Spnego {
    /// Decode the value of a `Negotiate <b64>` HTTP header
    /// (post-base64 — caller must already have decoded).
    pub fn parse_initial(bytes: &[u8]) -> Result<NegTokenInit, FederationError> {
        // InitialContextToken: [APPLICATION 0] IMPLICIT SEQUENCE
        //   { thisMech OID, innerContextToken ANY }
        let mut d = Decoder::new(bytes);
        let outer = d.read_expected(Tag::application(0, Form::Constructed))
            .map_err(|e| FederationError::Spnego(format!("outer tag: {e}")))?;
        let mut o = Decoder::new(outer);
        // Read the mechanism OID — must be SPNEGO.
        let (_, oid_payload) = o.read_tlv()
            .map_err(|e| FederationError::Spnego(format!("mech oid: {e}")))?;
        // Reconstruct the full OID element so we can byte-compare.
        let mut oid_full = Vec::with_capacity(2 + oid_payload.len());
        oid_full.push(0x06);
        oid_full.push(oid_payload.len() as u8);
        oid_full.extend_from_slice(oid_payload);
        if oid_full != oid::SPNEGO {
            return Err(FederationError::Spnego("not SPNEGO mech".into()));
        }
        // Inner NegotiationToken — CHOICE wrapping [0] NegTokenInit.
        let (inner_tag, inner_payload) = o.read_tlv()
            .map_err(|e| FederationError::Spnego(format!("inner: {e}")))?;
        if inner_tag != Tag::context(0, Form::Constructed) {
            return Err(FederationError::Spnego("expected NegTokenInit choice".into()));
        }
        // NegTokenInit is a SEQUENCE.
        let mut sd = Decoder::new(inner_payload);
        let seq = sd
            .read_expected(Tag::universal(16, Form::Constructed))
            .map_err(|e| FederationError::Spnego(format!("seq: {e}")))?;
        let mut s = Decoder::new(seq);
        let mut nti = NegTokenInit {
            mech_types: Vec::new(),
            mech_token: None,
            mech_list_mic: None,
        };
        while !s.eof() {
            let (tag, payload) = s
                .read_tlv()
                .map_err(|e| FederationError::Spnego(format!("field: {e}")))?;
            match tag.number {
                0 if tag.class == ber::Class::Context => {
                    // mechTypes [0] MechTypeList = SEQUENCE OF OID
                    let mut md = Decoder::new(payload);
                    let seq = md
                        .read_expected(Tag::universal(16, Form::Constructed))
                        .map_err(|e| FederationError::Spnego(format!("mechTypes seq: {e}")))?;
                    let mut ms = Decoder::new(seq);
                    while !ms.eof() {
                        let (_oid_tag, oid_payload) = ms
                            .read_tlv()
                            .map_err(|e| FederationError::Spnego(format!("oid: {e}")))?;
                        let mut full = Vec::with_capacity(2 + oid_payload.len());
                        full.push(0x06);
                        full.push(oid_payload.len() as u8);
                        full.extend_from_slice(oid_payload);
                        nti.mech_types.push(full);
                    }
                }
                2 if tag.class == ber::Class::Context => {
                    // mechToken [2] OCTET STRING (wrapped, primitive).
                    let mut td = Decoder::new(payload);
                    let inner = td
                        .read_octet_string()
                        .map_err(|e| FederationError::Spnego(format!("mechToken: {e}")))?;
                    nti.mech_token = Some(inner.to_vec());
                }
                3 if tag.class == ber::Class::Context => {
                    let mut td = Decoder::new(payload);
                    let inner = td.read_octet_string().map_err(|e| FederationError::Spnego(format!("mic: {e}")))?;
                    nti.mech_list_mic = Some(inner.to_vec());
                }
                _ => {}
            }
        }
        Ok(nti)
    }

    /// Encode a NegTokenInit back to bytes (used by fixtures and the
    /// server-emitted challenge when we need to renegotiate).
    pub fn encode_initial(init: &NegTokenInit) -> Vec<u8> {
        let mut mech_elements: Vec<Element> = Vec::new();
        for m in &init.mech_types {
            // m is already the encoded OID; emit it verbatim.  We
            // peel back the leading TLV and re-wrap so this is
            // proper BER.  Since `mech_types` was produced via
            // `parse_initial`, m is shaped `06 LL ...`.
            mech_elements.push(Element {
                tag: Tag::universal(6, Form::Primitive),
                bytes: m[2..].to_vec(),
            });
        }
        let mechs_seq = ber::sequence(&mech_elements);
        let mech_types_field = Element {
            tag: Tag::context(0, Form::Constructed),
            bytes: mechs_seq.encode(),
        };
        let mut fields = vec![mech_types_field];
        if let Some(tok) = &init.mech_token {
            let inner = ber::octet_string(tok);
            fields.push(Element { tag: Tag::context(2, Form::Constructed), bytes: inner.encode() });
        }
        let body = ber::sequence(&fields);
        let init_choice = Element { tag: Tag::context(0, Form::Constructed), bytes: body.encode() };
        let mut outer_bytes = Vec::new();
        let spnego_oid = Element { tag: Tag::universal(6, Form::Primitive), bytes: oid::SPNEGO[2..].to_vec() };
        outer_bytes.extend_from_slice(&spnego_oid.encode());
        outer_bytes.extend_from_slice(&init_choice.encode());
        Element { tag: Tag::application(0, Form::Constructed), bytes: outer_bytes }.encode()
    }

    /// Encode a NegTokenResp (server -> client).
    pub fn encode_response(resp: &NegTokenResp) -> Vec<u8> {
        let mut fields = Vec::new();
        if let Some(ns) = resp.neg_state {
            let inner = ber::enumerated(ns as i64);
            fields.push(Element { tag: Tag::context(0, Form::Constructed), bytes: inner.encode() });
        }
        if let Some(m) = &resp.supported_mech {
            let elem = Element { tag: Tag::universal(6, Form::Primitive), bytes: m[2..].to_vec() };
            fields.push(Element { tag: Tag::context(1, Form::Constructed), bytes: elem.encode() });
        }
        if let Some(tok) = &resp.response_token {
            let inner = ber::octet_string(tok);
            fields.push(Element { tag: Tag::context(2, Form::Constructed), bytes: inner.encode() });
        }
        let seq = ber::sequence(&fields);
        Element { tag: Tag::context(1, Form::Constructed), bytes: seq.encode() }.encode()
    }

    /// Decide what to do with a parsed NegTokenInit.  This is the
    /// state-machine entry point for the HTTP handler.
    pub fn evaluate(init: &NegTokenInit) -> AuthState {
        // Prefer Kerberos.  If both MS-KRB5 and KRB5 are offered we
        // pick whichever was listed first.
        for m in &init.mech_types {
            if m.as_slice() == oid::KRB5 || m.as_slice() == oid::MS_KRB5 {
                return match &init.mech_token {
                    Some(t) => AuthState::AwaitingExternalVerify {
                        mech_token: t.clone(),
                        chosen_mech: m.clone(),
                    },
                    None => AuthState::Rejected,
                };
            }
        }
        AuthState::UnsupportedMechs
    }
}

/// Build the `WWW-Authenticate: Negotiate <b64>` value for a fresh
/// challenge — server-initiated SPNEGO.  Returns the raw bytes the
/// caller should base64-encode and emit.
pub fn fresh_challenge() -> Vec<u8> {
    Spnego::encode_initial(&NegTokenInit {
        mech_types: vec![oid::KRB5.to_vec()],
        mech_token: None,
        mech_list_mic: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_init(tok: Vec<u8>, mech: &[u8]) -> NegTokenInit {
        NegTokenInit {
            mech_types: vec![mech.to_vec()],
            mech_token: Some(tok),
            mech_list_mic: None,
        }
    }

    #[test]
    fn encode_decode_round_trip_krb5() {
        let nti = sample_init(b"ap-req-bytes".to_vec(), oid::KRB5);
        let bytes = Spnego::encode_initial(&nti);
        let parsed = Spnego::parse_initial(&bytes).unwrap();
        assert_eq!(parsed.mech_types, nti.mech_types);
        assert_eq!(parsed.mech_token, nti.mech_token);
    }

    #[test]
    fn evaluate_with_krb5_yields_external_verify() {
        let nti = sample_init(b"tok".to_vec(), oid::KRB5);
        match Spnego::evaluate(&nti) {
            AuthState::AwaitingExternalVerify { mech_token, chosen_mech } => {
                assert_eq!(mech_token, b"tok");
                assert_eq!(chosen_mech, oid::KRB5);
            }
            other => panic!("expected AwaitingExternalVerify, got {other:?}"),
        }
    }

    #[test]
    fn evaluate_with_only_ntlm_is_unsupported() {
        let nti = NegTokenInit {
            mech_types: vec![oid::NTLM_SSP.to_vec()],
            mech_token: Some(vec![0u8; 8]),
            mech_list_mic: None,
        };
        assert_eq!(Spnego::evaluate(&nti), AuthState::UnsupportedMechs);
    }

    #[test]
    fn evaluate_with_krb5_but_no_mech_token_rejects() {
        let nti = NegTokenInit {
            mech_types: vec![oid::KRB5.to_vec()],
            mech_token: None,
            mech_list_mic: None,
        };
        assert_eq!(Spnego::evaluate(&nti), AuthState::Rejected);
    }

    #[test]
    fn parse_rejects_non_spnego_outer_oid() {
        // Wrap KRB5 OID directly inside [APPLICATION 0] — not SPNEGO.
        let mut outer = Vec::new();
        outer.extend_from_slice(oid::KRB5);
        let frame = Element { tag: Tag::application(0, Form::Constructed), bytes: outer }.encode();
        assert!(Spnego::parse_initial(&frame).is_err());
    }

    #[test]
    fn fresh_challenge_contains_spnego_oid_bytes() {
        let chal = fresh_challenge();
        // SPNEGO OID 1.3.6.1.5.5.2 → DER 2b 06 01 05 05 02.
        let needle: [u8; 6] = [0x2b, 0x06, 0x01, 0x05, 0x05, 0x02];
        assert!(chal.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn neg_state_from_i64_covers_known_values() {
        assert_eq!(NegState::from_i64(0), Some(NegState::AcceptCompleted));
        assert_eq!(NegState::from_i64(1), Some(NegState::AcceptIncomplete));
        assert_eq!(NegState::from_i64(2), Some(NegState::Reject));
        assert_eq!(NegState::from_i64(3), Some(NegState::RequestMic));
        assert_eq!(NegState::from_i64(7), None);
    }

    #[test]
    fn encode_response_round_trip_through_parser_pieces() {
        let r = NegTokenResp {
            neg_state: Some(NegState::AcceptCompleted),
            supported_mech: Some(oid::KRB5.to_vec()),
            response_token: Some(b"ap-rep".to_vec()),
            mech_list_mic: None,
        };
        let bytes = Spnego::encode_response(&r);
        // It must at least start with [1] CONSTRUCTED.
        assert_eq!(bytes[0] & 0b1110_0000, 0b1010_0000);
        // Body contains the response token bytes.
        assert!(bytes.windows(6).any(|w| w == b"ap-rep"));
    }
}
