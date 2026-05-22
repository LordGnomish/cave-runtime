// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/main/java/org/keycloak/storage/ldap/LDAPIdentityStore.java + RFC 4511 §4.2 (BindRequest)

//! LDAP bind / unbind state machine + minimal BER LDAPv3 frame
//! encoder. Maps Keycloak's `LDAPIdentityStore.bind()` /
//! `LDAPIdentityStore.unbind()` flow onto pure-Rust BER frames
//! (RFC 4511 §4.2 BindRequest / §4.2.2 BindResponse / §4.3
//! UnbindRequest).

use std::sync::atomic::{AtomicI32, Ordering};

use super::{LdapError, ResultCode};

// ── BER tag constants ────────────────────────────────────────────────────────
//
// RFC 4511 §5.1 anchors every LDAP protocol message inside a
// SEQUENCE-of-SEQUENCE. The outer SEQUENCE is tagged `0x30`, the
// `messageID` is an INTEGER (`0x02`), and the protocol-op chosen
// is identified by an APPLICATION-tagged constructed value.

const TAG_SEQUENCE: u8 = 0x30;
const TAG_INTEGER: u8 = 0x02;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_ENUMERATED: u8 = 0x0A;

/// APPLICATION 0 — BindRequest (RFC 4511 §4.2). Constructed.
pub(crate) const TAG_BIND_REQUEST: u8 = 0x60;
/// APPLICATION 1 — BindResponse (RFC 4511 §4.2.2). Constructed.
pub(crate) const TAG_BIND_RESPONSE: u8 = 0x61;
/// APPLICATION 2 — UnbindRequest (RFC 4511 §4.3). Primitive
/// (NULL value).
pub(crate) const TAG_UNBIND_REQUEST: u8 = 0x42;

/// CONTEXT-SPECIFIC 0 — `simple` AuthenticationChoice octet
/// string (RFC 4511 §4.2 `simple [0] OCTET STRING`). Primitive.
const TAG_SIMPLE_AUTH: u8 = 0x80;
/// CONTEXT-SPECIFIC 3 — `sasl` AuthenticationChoice constructed
/// (RFC 4511 §4.2 `sasl [3] SaslCredentials`).
const TAG_SASL_AUTH: u8 = 0xA3;

/// LDAP `BindRequest.authentication` choices we support.
/// RFC 4511 §4.2 leaves the AuthenticationChoice extensible —
/// cave-auth covers the two that ship in every real federation
/// deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindAuth {
    /// `simple [0] OCTET STRING` — DN + password (cleartext over
    /// TLS).
    Simple(String),
    /// `sasl [3] SaslCredentials` — mechanism name + optional
    /// credential blob. cave-auth doesn't *drive* SASL but the
    /// frame builder will emit it; live SASL EXTERNAL test
    /// requires a real LDAP server providing client-cert auth.
    Sasl {
        /// `mechanism LDAPString`.
        mechanism: String,
        /// `credentials OCTET STRING OPTIONAL`.
        credentials: Option<Vec<u8>>,
    },
}

/// Minimal LDAP frame builder. Holds a monotonically-increasing
/// `messageID`, the way Keycloak's `LDAPIdentityStore` does.
#[derive(Debug)]
pub struct LdapConnection {
    msg_id: AtomicI32,
}

impl Default for LdapConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl LdapConnection {
    /// Fresh connection — `messageID` starts at 1 per RFC 4511
    /// §4.1.1.1.
    pub fn new() -> Self {
        LdapConnection {
            msg_id: AtomicI32::new(1),
        }
    }
    /// Allocate the next `messageID` for an outgoing operation.
    /// Wraps at `i32::MAX` back to 1 — matches the OpenLDAP
    /// client library behaviour.
    pub fn next_message_id(&self) -> i32 {
        let mut current = self.msg_id.load(Ordering::SeqCst);
        loop {
            let next = if current == i32::MAX { 1 } else { current + 1 };
            match self
                .msg_id
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(prev) => return prev,
                Err(actual) => current = actual,
            }
        }
    }

    /// Encode a LDAP BindRequest. Returns the BER bytes ready
    /// to write onto a TCP socket.
    ///
    /// Frame shape (RFC 4511 §4.2):
    /// ```text
    /// LDAPMessage ::= SEQUENCE {
    ///     messageID  MessageID,
    ///     protocolOp BindRequest,
    ///     ...
    /// }
    /// BindRequest ::= [APPLICATION 0] SEQUENCE {
    ///     version  INTEGER (1..127),
    ///     name     LDAPDN,
    ///     authentication AuthenticationChoice
    /// }
    /// ```
    pub fn encode_bind_request(&self, dn: &str, auth: &BindAuth) -> Vec<u8> {
        let msg_id = self.next_message_id();
        let mut inner = Vec::new();
        // version INTEGER (= 3)
        push_integer(&mut inner, 3);
        // name LDAPDN
        push_octet_string(&mut inner, dn.as_bytes());
        // authentication
        match auth {
            BindAuth::Simple(password) => {
                push_tlv(&mut inner, TAG_SIMPLE_AUTH, password.as_bytes());
            }
            BindAuth::Sasl {
                mechanism,
                credentials,
            } => {
                let mut sasl_payload = Vec::new();
                push_octet_string(&mut sasl_payload, mechanism.as_bytes());
                if let Some(cred) = credentials {
                    push_octet_string(&mut sasl_payload, cred);
                }
                push_tlv(&mut inner, TAG_SASL_AUTH, &sasl_payload);
            }
        }
        let bind_req = wrap_tlv(TAG_BIND_REQUEST, &inner);
        let mut outer = Vec::new();
        push_integer(&mut outer, msg_id as i64);
        outer.extend_from_slice(&bind_req);
        wrap_tlv(TAG_SEQUENCE, &outer)
    }

    /// Encode the LDAP UnbindRequest (RFC 4511 §4.3). The
    /// protocol-op is a primitive NULL — no inner content.
    pub fn encode_unbind_request(&self) -> Vec<u8> {
        let msg_id = self.next_message_id();
        let mut outer = Vec::new();
        push_integer(&mut outer, msg_id as i64);
        // UnbindRequest ::= [APPLICATION 2] NULL — primitive, len 0
        outer.push(TAG_UNBIND_REQUEST);
        outer.push(0x00);
        wrap_tlv(TAG_SEQUENCE, &outer)
    }
}

/// Parsed BindResponse — what the bind state machine consumes
/// to decide if the LDAP server accepted us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindResponse {
    /// `messageID` echoed by the server.
    pub message_id: i32,
    /// Result code from the operation (RFC 4511 §4.1.9).
    pub result_code: ResultCode,
    /// Matched DN — empty when result_code is not
    /// `noSuchObject`/`invalidDNSyntax`.
    pub matched_dn: String,
    /// Server-supplied human-readable message — surfaced to the
    /// caller as part of [`LdapError::BindFailed`].
    pub diagnostic_message: String,
}

impl BindResponse {
    /// Parse a `BindResponse` BER frame received from the
    /// server. Accepts the full `LDAPMessage` (outer SEQUENCE)
    /// — the caller doesn't need to know the wrapper.
    pub fn parse(bytes: &[u8]) -> Result<Self, LdapError> {
        let (outer_body, rest) = read_tlv(bytes, TAG_SEQUENCE)?;
        if !rest.is_empty() {
            return Err(LdapError::Protocol(
                "trailing bytes after LDAPMessage".into(),
            ));
        }
        let (msg_id, after_id) = read_integer(outer_body)?;
        let (resp_body, after_resp) = read_tlv(after_id, TAG_BIND_RESPONSE)?;
        if !after_resp.is_empty() {
            // BindResponse can carry controls (we don't parse them) — but
            // for cave-auth's minimal port we accept and ignore them.
        }
        let (rc, after_rc) = read_enumerated(resp_body)?;
        let (matched_dn, after_dn) = read_octet_string_utf8(after_rc)?;
        let (diag, _after_diag) = read_octet_string_utf8(after_dn)?;
        Ok(BindResponse {
            message_id: msg_id as i32,
            result_code: ResultCode::from_raw(rc),
            matched_dn,
            diagnostic_message: diag,
        })
    }
}

// ── Low-level BER primitives ─────────────────────────────────────────────────
//
// Just enough to encode the LDAPMessage / BindRequest / read
// LDAPMessage / BindResponse path. Not a general BER library.

/// Encode an INTEGER (RFC 4511 §5.1, X.690 §8.3). Stripping the
/// leading 0x00 if present, two's-complement for negatives.
pub(crate) fn push_integer(out: &mut Vec<u8>, value: i64) {
    let mut bytes = value.to_be_bytes().to_vec();
    while bytes.len() > 1 {
        let b0 = bytes[0];
        let b1 = bytes[1];
        if (b0 == 0x00 && b1 & 0x80 == 0) || (b0 == 0xFF && b1 & 0x80 != 0) {
            bytes.remove(0);
        } else {
            break;
        }
    }
    push_tlv(out, TAG_INTEGER, &bytes);
}

pub(crate) fn push_octet_string(out: &mut Vec<u8>, value: &[u8]) {
    push_tlv(out, TAG_OCTET_STRING, value);
}

pub(crate) fn push_tlv(out: &mut Vec<u8>, tag: u8, value: &[u8]) {
    out.push(tag);
    push_length(out, value.len());
    out.extend_from_slice(value);
}

pub(crate) fn wrap_tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() + 4);
    out.push(tag);
    push_length(&mut out, value.len());
    out.extend_from_slice(value);
    out
}

pub(crate) fn push_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
        return;
    }
    let mut bytes = Vec::new();
    let mut n = len;
    while n > 0 {
        bytes.push((n & 0xFF) as u8);
        n >>= 8;
    }
    bytes.reverse();
    out.push(0x80 | bytes.len() as u8);
    out.extend_from_slice(&bytes);
}

pub(crate) fn read_tlv(bytes: &[u8], expected_tag: u8) -> Result<(&[u8], &[u8]), LdapError> {
    if bytes.is_empty() {
        return Err(LdapError::Protocol("unexpected EOF reading tag".into()));
    }
    if bytes[0] != expected_tag {
        return Err(LdapError::Protocol(format!(
            "expected tag {expected_tag:#04x}, got {:#04x}",
            bytes[0]
        )));
    }
    let (len, after_len) = read_length(&bytes[1..])?;
    if after_len.len() < len {
        return Err(LdapError::Protocol("LDAP TLV truncated".into()));
    }
    Ok((&after_len[..len], &after_len[len..]))
}

pub(crate) fn read_length(bytes: &[u8]) -> Result<(usize, &[u8]), LdapError> {
    if bytes.is_empty() {
        return Err(LdapError::Protocol("EOF reading length".into()));
    }
    let first = bytes[0];
    if first & 0x80 == 0 {
        return Ok((first as usize, &bytes[1..]));
    }
    let n = (first & 0x7F) as usize;
    if n == 0 {
        return Err(LdapError::Protocol("indefinite length forbidden".into()));
    }
    if bytes.len() < 1 + n {
        return Err(LdapError::Protocol("length field truncated".into()));
    }
    let mut len: usize = 0;
    for i in 0..n {
        len = (len << 8) | bytes[1 + i] as usize;
    }
    Ok((len, &bytes[1 + n..]))
}

pub(crate) fn read_integer(bytes: &[u8]) -> Result<(i64, &[u8]), LdapError> {
    let (body, rest) = read_tlv(bytes, TAG_INTEGER)?;
    if body.is_empty() {
        return Err(LdapError::Protocol("INTEGER body empty".into()));
    }
    let neg = body[0] & 0x80 != 0;
    let mut val: i64 = if neg { -1 } else { 0 };
    for b in body {
        val = (val << 8) | (*b as i64 & 0xFF);
    }
    Ok((val, rest))
}

pub(crate) fn read_enumerated(bytes: &[u8]) -> Result<(u32, &[u8]), LdapError> {
    let (body, rest) = read_tlv(bytes, TAG_ENUMERATED)?;
    if body.is_empty() {
        return Err(LdapError::Protocol("ENUMERATED body empty".into()));
    }
    let mut val: u32 = 0;
    for b in body {
        val = (val << 8) | (*b as u32 & 0xFF);
    }
    Ok((val, rest))
}

pub(crate) fn read_octet_string_utf8(bytes: &[u8]) -> Result<(String, &[u8]), LdapError> {
    let (body, rest) = read_tlv(bytes, TAG_OCTET_STRING)?;
    let s = std::str::from_utf8(body)
        .map_err(|e| LdapError::Protocol(format!("non-utf8 LDAPString: {e}")))?
        .to_owned();
    Ok((s, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_id_starts_at_one_and_increments() {
        let c = LdapConnection::new();
        assert_eq!(c.next_message_id(), 1);
        assert_eq!(c.next_message_id(), 2);
        assert_eq!(c.next_message_id(), 3);
    }

    #[test]
    fn bind_request_has_outer_sequence_and_message_id() {
        let c = LdapConnection::new();
        let frame = c.encode_bind_request(
            "cn=admin,dc=example,dc=com",
            &BindAuth::Simple("hunter2".into()),
        );
        assert_eq!(frame[0], TAG_SEQUENCE);
        // RFC 4511: outer is SEQUENCE { messageID, protocolOp }.
        // The first content byte after the outer length must be
        // INTEGER (the messageID).
        let (body, _) = read_tlv(&frame, TAG_SEQUENCE).unwrap();
        let (msg_id, _) = read_integer(body).unwrap();
        assert_eq!(msg_id, 1);
    }

    #[test]
    fn bind_request_uses_application0_tag_for_protocol_op() {
        let c = LdapConnection::new();
        let frame = c.encode_bind_request(
            "uid=jdoe,ou=people,dc=example,dc=com",
            &BindAuth::Simple("pw".into()),
        );
        let (body, _) = read_tlv(&frame, TAG_SEQUENCE).unwrap();
        // skip messageID
        let (_msg_id, after_id) = read_integer(body).unwrap();
        // next tag must be APPLICATION 0 (= 0x60) — BindRequest
        assert_eq!(after_id[0], TAG_BIND_REQUEST);
    }

    #[test]
    fn bind_request_version_is_three() {
        let c = LdapConnection::new();
        let frame = c.encode_bind_request("", &BindAuth::Simple("p".into()));
        let (body, _) = read_tlv(&frame, TAG_SEQUENCE).unwrap();
        let (_msg_id, after_id) = read_integer(body).unwrap();
        let (req_body, _) = read_tlv(after_id, TAG_BIND_REQUEST).unwrap();
        let (version, _) = read_integer(req_body).unwrap();
        assert_eq!(version, 3, "LDAPv3 mandates version=3");
    }

    #[test]
    fn bind_request_carries_dn() {
        let c = LdapConnection::new();
        let dn = "cn=keycloak,dc=example,dc=com";
        let frame = c.encode_bind_request(dn, &BindAuth::Simple("x".into()));
        // Cheap byte search — the DN must appear verbatim somewhere
        // in the encoded frame.
        assert!(
            frame.windows(dn.len()).any(|w| w == dn.as_bytes()),
            "DN must be present in the encoded frame"
        );
    }

    #[test]
    fn simple_bind_auth_uses_context0_tag() {
        let c = LdapConnection::new();
        let frame = c.encode_bind_request("dn=x", &BindAuth::Simple("pw".into()));
        // The simple bind credential carries CONTEXT-SPECIFIC 0
        // tag = 0x80 — search for it in the frame.
        assert!(
            frame.iter().any(|b| *b == 0x80),
            "simple bind must use CONTEXT-SPECIFIC 0 tag"
        );
    }

    #[test]
    fn sasl_bind_uses_context3_tag() {
        let c = LdapConnection::new();
        let frame = c.encode_bind_request(
            "",
            &BindAuth::Sasl {
                mechanism: "EXTERNAL".into(),
                credentials: None,
            },
        );
        assert!(
            frame.iter().any(|b| *b == 0xA3),
            "sasl bind must use CONTEXT-SPECIFIC 3 tag"
        );
        // The mechanism name must appear in the frame
        assert!(frame.windows(8).any(|w| w == b"EXTERNAL"));
    }

    #[test]
    fn unbind_request_is_single_byte_application2() {
        let c = LdapConnection::new();
        let frame = c.encode_unbind_request();
        let (body, _) = read_tlv(&frame, TAG_SEQUENCE).unwrap();
        let (_msg_id, after_id) = read_integer(body).unwrap();
        // UnbindRequest is APPLICATION 2 primitive (NULL),
        // tag = 0x42, length = 0
        assert_eq!(after_id, &[TAG_UNBIND_REQUEST, 0x00]);
    }

    #[test]
    fn bind_response_parses_success() {
        // Hand-crafted success BindResponse (resultCode = 0,
        // matchedDN = "", diagnosticMessage = "").
        // 30 0c — outer SEQUENCE
        //   02 01 01 — messageID 1
        //   61 07 — BindResponse [APPLICATION 1]
        //     0a 01 00 — ENUMERATED 0 (success)
        //     04 00 — matchedDN ""
        //     04 00 — diagnostic ""
        let bytes = [
            0x30, 0x0c, 0x02, 0x01, 0x01, 0x61, 0x07, 0x0a, 0x01, 0x00, 0x04, 0x00, 0x04, 0x00,
        ];
        let resp = BindResponse::parse(&bytes).unwrap();
        assert_eq!(resp.message_id, 1);
        assert!(resp.result_code.is_success());
        assert_eq!(resp.matched_dn, "");
        assert_eq!(resp.diagnostic_message, "");
    }

    #[test]
    fn bind_response_parses_invalid_credentials() {
        let bytes = [
            0x30, 0x10, 0x02, 0x01, 0x02, 0x61, 0x0b, 0x0a, 0x01, 0x31, 0x04, 0x00, 0x04, 0x04,
            b'b', b'a', b'd', b'!',
        ];
        let resp = BindResponse::parse(&bytes).unwrap();
        assert_eq!(resp.result_code, ResultCode::InvalidCredentials);
        assert_eq!(resp.diagnostic_message, "bad!");
    }

    #[test]
    fn ber_length_short_form() {
        let mut out = Vec::new();
        push_length(&mut out, 5);
        assert_eq!(out, vec![5]);
    }

    #[test]
    fn ber_length_long_form_128() {
        let mut out = Vec::new();
        push_length(&mut out, 128);
        assert_eq!(out, vec![0x81, 0x80]);
    }

    #[test]
    fn ber_length_long_form_300() {
        let mut out = Vec::new();
        push_length(&mut out, 300);
        // 0x82 = long form, 2 length bytes
        // 0x01 0x2c = 300
        assert_eq!(out, vec![0x82, 0x01, 0x2c]);
    }

    #[test]
    fn integer_roundtrips_through_ber() {
        for v in [0_i64, 1, 127, 128, 255, 256, 65535, 65536, -1, -128, -129] {
            let mut out = Vec::new();
            push_integer(&mut out, v);
            let (parsed, _) = read_integer(&out).unwrap();
            assert_eq!(parsed, v);
        }
    }

    #[test]
    fn protocol_error_on_unexpected_tag() {
        let bytes = [0x99, 0x00];
        assert!(matches!(
            BindResponse::parse(&bytes),
            Err(LdapError::Protocol(_))
        ));
    }
}
