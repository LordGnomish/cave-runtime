// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/kerberos/.../ + RFC 4178 §4 (SPNEGO Token Definitions)

//! SPNEGO `NegTokenInit` / `NegTokenResp` parsers (RFC 4178 §4).
//!
//! NegTokenInit (sent by initiator, wrapped in
//! `InitialContextToken`):
//! ```text
//! NegotiationToken ::= CHOICE {
//!   negTokenInit  [0] NegTokenInit,
//!   negTokenResp  [1] NegTokenResp
//! }
//! NegTokenInit ::= SEQUENCE {
//!   mechTypes  [0] MechTypeList,
//!   reqFlags   [1] ContextFlags OPTIONAL,
//!   mechToken  [2] OCTET STRING OPTIONAL,
//!   mechListMIC[3] OCTET STRING OPTIONAL
//! }
//! NegTokenResp ::= SEQUENCE {
//!   negState        [0] ENUMERATED OPTIONAL,
//!   supportedMech   [1] OBJECT IDENTIFIER OPTIONAL,
//!   responseToken   [2] OCTET STRING OPTIONAL,
//!   mechListMIC     [3] OCTET STRING OPTIONAL
//! }
//! ```

use super::gssapi::{read_length, read_tlv, push_length};
use super::KerberosError;

const TAG_OID: u8 = 0x06;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_ENUMERATED: u8 = 0x0A;

const TAG_CTX_0: u8 = 0xa0;
const TAG_CTX_1: u8 = 0xa1;
const TAG_CTX_2: u8 = 0xa2;
#[allow(dead_code)]
const TAG_CTX_3: u8 = 0xa3;

/// RFC 4178 §4.2.2 `negState` ENUMERATED.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegState {
    AcceptCompleted = 0,
    AcceptIncomplete = 1,
    Reject = 2,
    RequestMic = 3,
}

impl NegState {
    pub fn from_raw(n: u32) -> Option<Self> {
        Some(match n {
            0 => NegState::AcceptCompleted,
            1 => NegState::AcceptIncomplete,
            2 => NegState::Reject,
            3 => NegState::RequestMic,
            _ => return None,
        })
    }
}

/// RFC 4178 §4.2.1 NegTokenInit. The OIDs in `mech_types` are
/// raw encoded form (without the `0x06`/length prefix) so they
/// can be byte-compared against [`super::gssapi::OID_SPNEGO`]
/// etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTokenInit<'a> {
    pub mech_types: Vec<&'a [u8]>,
    pub mech_token: Option<&'a [u8]>,
}

impl<'a> NegTokenInit<'a> {
    /// Parse a `NegTokenInit`. Accepts either the raw
    /// `CHOICE [0]` wrapper (`0xa0 …`) or the inner SEQUENCE.
    pub fn parse(input: &'a [u8]) -> Result<Self, KerberosError> {
        let body = if input.first() == Some(&TAG_CTX_0) {
            let (b, _) = read_tlv(input, TAG_CTX_0)?;
            b
        } else {
            input
        };
        let (seq, _) = read_tlv(body, TAG_SEQUENCE)?;
        let mut cur = seq;
        let mut mech_types: Vec<&[u8]> = Vec::new();
        let mut mech_token: Option<&[u8]> = None;
        while !cur.is_empty() {
            let tag = cur[0];
            let (inner, rest) = read_tlv(cur, tag)?;
            match tag {
                TAG_CTX_0 => {
                    let (list_seq, _) = read_tlv(inner, TAG_SEQUENCE)?;
                    let mut lcur = list_seq;
                    while !lcur.is_empty() {
                        let (oid, lrest) = read_tlv(lcur, TAG_OID)?;
                        mech_types.push(oid);
                        lcur = lrest;
                    }
                }
                TAG_CTX_2 => {
                    let (octet, _) = read_tlv(inner, TAG_OCTET_STRING)?;
                    mech_token = Some(octet);
                }
                // reqFlags / mechListMIC ignored — Keycloak's
                // SPNEGOAuthenticator does the same; flags carry
                // hints, MIC is a post-handshake integrity tag.
                _ => {}
            }
            cur = rest;
        }
        if mech_types.is_empty() {
            return Err(KerberosError::Spnego(
                "NegTokenInit missing mechTypes".into(),
            ));
        }
        Ok(NegTokenInit {
            mech_types,
            mech_token,
        })
    }
}

/// RFC 4178 §4.2.2 NegTokenResp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegTokenResp<'a> {
    pub neg_state: Option<NegState>,
    pub supported_mech: Option<&'a [u8]>,
    pub response_token: Option<&'a [u8]>,
}

impl<'a> NegTokenResp<'a> {
    /// Parse a `NegTokenResp`. Accepts either the raw
    /// `CHOICE [1]` wrapper or the inner SEQUENCE.
    pub fn parse(input: &'a [u8]) -> Result<Self, KerberosError> {
        let body = if input.first() == Some(&TAG_CTX_1) {
            let (b, _) = read_tlv(input, TAG_CTX_1)?;
            b
        } else {
            input
        };
        let (seq, _) = read_tlv(body, TAG_SEQUENCE)?;
        let mut cur = seq;
        let mut neg_state: Option<NegState> = None;
        let mut supported_mech: Option<&[u8]> = None;
        let mut response_token: Option<&[u8]> = None;
        while !cur.is_empty() {
            let tag = cur[0];
            let (inner, rest) = read_tlv(cur, tag)?;
            match tag {
                TAG_CTX_0 => {
                    let (e, _) = read_tlv(inner, TAG_ENUMERATED)?;
                    if e.is_empty() {
                        return Err(KerberosError::Spnego(
                            "empty negState".into(),
                        ));
                    }
                    let v = e.iter().fold(0u32, |acc, b| (acc << 8) | *b as u32);
                    neg_state = NegState::from_raw(v);
                }
                TAG_CTX_1 => {
                    let (oid, _) = read_tlv(inner, TAG_OID)?;
                    supported_mech = Some(oid);
                }
                TAG_CTX_2 => {
                    let (octet, _) = read_tlv(inner, TAG_OCTET_STRING)?;
                    response_token = Some(octet);
                }
                _ => {}
            }
            cur = rest;
        }
        Ok(NegTokenResp {
            neg_state,
            supported_mech,
            response_token,
        })
    }
}

/// Build a NegTokenInit DER. Helper for the server-side
/// challenge — Keycloak's `SPNEGOAuthenticator.continueAuthChallenge`
/// emits the same wire-format.
pub fn build_neg_token_init(mech_types: &[&[u8]], mech_token: Option<&[u8]>) -> Vec<u8> {
    let mut list = Vec::new();
    for oid in mech_types {
        list.push(TAG_OID);
        push_length(&mut list, oid.len());
        list.extend_from_slice(oid);
    }
    let mut mech_list_seq = Vec::new();
    mech_list_seq.push(TAG_SEQUENCE);
    push_length(&mut mech_list_seq, list.len());
    mech_list_seq.extend_from_slice(&list);

    let mut wrapped_mech_types = Vec::new();
    wrapped_mech_types.push(TAG_CTX_0);
    push_length(&mut wrapped_mech_types, mech_list_seq.len());
    wrapped_mech_types.extend_from_slice(&mech_list_seq);

    let mut inner = Vec::new();
    inner.extend_from_slice(&wrapped_mech_types);

    if let Some(tok) = mech_token {
        let mut wrapped_token = Vec::new();
        wrapped_token.push(TAG_OCTET_STRING);
        push_length(&mut wrapped_token, tok.len());
        wrapped_token.extend_from_slice(tok);

        let mut ctx2 = Vec::new();
        ctx2.push(TAG_CTX_2);
        push_length(&mut ctx2, wrapped_token.len());
        ctx2.extend_from_slice(&wrapped_token);

        inner.extend_from_slice(&ctx2);
    }

    let mut seq = Vec::new();
    seq.push(TAG_SEQUENCE);
    push_length(&mut seq, inner.len());
    seq.extend_from_slice(&inner);

    let mut out = Vec::new();
    out.push(TAG_CTX_0);
    push_length(&mut out, seq.len());
    out.extend_from_slice(&seq);
    out
}

/// Read length helper re-export for the negotiate layer.
#[allow(dead_code)]
pub(crate) fn read_length_raw(b: &[u8]) -> Result<(usize, &[u8]), KerberosError> {
    read_length(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kerberos::gssapi::OID_KRB5;
    use crate::kerberos::gssapi::OID_SPNEGO;

    #[test]
    fn neg_token_init_round_trips_through_parse() {
        let oids: &[&[u8]] = &[OID_KRB5];
        let token = [0xaa, 0xbb, 0xcc];
        let bytes = build_neg_token_init(oids, Some(&token));
        let parsed = NegTokenInit::parse(&bytes).unwrap();
        assert_eq!(parsed.mech_types, vec![OID_KRB5]);
        assert_eq!(parsed.mech_token.unwrap(), &token);
    }

    #[test]
    fn neg_token_init_without_mech_token() {
        let bytes = build_neg_token_init(&[OID_SPNEGO], None);
        let parsed = NegTokenInit::parse(&bytes).unwrap();
        assert!(parsed.mech_token.is_none());
    }

    #[test]
    fn neg_token_init_rejects_when_no_mech_types() {
        // SEQUENCE with no mechTypes — should fail.
        let bytes = vec![0xa0, 0x02, 0x30, 0x00];
        let err = NegTokenInit::parse(&bytes).unwrap_err();
        assert!(matches!(err, KerberosError::Spnego(_) | KerberosError::Asn1(_)));
    }

    #[test]
    fn neg_token_resp_parses_accept_completed() {
        // CHOICE [1] SEQUENCE { negState [0] ENUM 0 }
        // a1 05 30 03 a0 03 0a 01 00
        let bytes = [
            0xa1, 0x07, 0x30, 0x05, 0xa0, 0x03, 0x0a, 0x01, 0x00,
        ];
        let parsed = NegTokenResp::parse(&bytes).unwrap();
        assert_eq!(parsed.neg_state, Some(NegState::AcceptCompleted));
    }

    #[test]
    fn neg_token_resp_parses_accept_incomplete_with_supported_mech() {
        // CHOICE [1] SEQUENCE {
        //   negState [0] ENUMERATED 1,
        //   supportedMech [1] OID krb5
        // }
        let mut inner = vec![0xa0, 0x03, 0x0a, 0x01, 0x01];
        // supportedMech ctx[1] wrapping OID krb5
        let mut oid_tlv = vec![0x06, OID_KRB5.len() as u8];
        oid_tlv.extend_from_slice(OID_KRB5);
        inner.push(0xa1);
        inner.push(oid_tlv.len() as u8);
        inner.extend_from_slice(&oid_tlv);

        let mut seq = vec![0x30, inner.len() as u8];
        seq.extend_from_slice(&inner);
        let mut bytes = vec![0xa1, seq.len() as u8];
        bytes.extend_from_slice(&seq);

        let parsed = NegTokenResp::parse(&bytes).unwrap();
        assert_eq!(parsed.neg_state, Some(NegState::AcceptIncomplete));
        assert_eq!(parsed.supported_mech, Some(OID_KRB5));
    }

    #[test]
    fn neg_token_resp_parses_response_token() {
        // negState=1, responseToken=[0xde 0xad]
        let mut inner = vec![0xa0, 0x03, 0x0a, 0x01, 0x01];
        let tok = [0xde, 0xad];
        let mut tok_tlv = vec![0x04, tok.len() as u8];
        tok_tlv.extend_from_slice(&tok);
        inner.push(0xa2);
        inner.push(tok_tlv.len() as u8);
        inner.extend_from_slice(&tok_tlv);
        let mut seq = vec![0x30, inner.len() as u8];
        seq.extend_from_slice(&inner);
        let mut bytes = vec![0xa1, seq.len() as u8];
        bytes.extend_from_slice(&seq);
        let parsed = NegTokenResp::parse(&bytes).unwrap();
        assert_eq!(parsed.response_token, Some(&tok[..]));
    }

    #[test]
    fn neg_state_round_trips() {
        assert_eq!(NegState::from_raw(0), Some(NegState::AcceptCompleted));
        assert_eq!(NegState::from_raw(1), Some(NegState::AcceptIncomplete));
        assert_eq!(NegState::from_raw(2), Some(NegState::Reject));
        assert_eq!(NegState::from_raw(3), Some(NegState::RequestMic));
        assert_eq!(NegState::from_raw(99), None);
    }
}
