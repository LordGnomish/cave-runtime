// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/kerberos/.../ + RFC 2743 §3.1 (GSSAPI InitialContextToken)

//! GSSAPI initial-context-token wrapper. RFC 2743 §3.1 defines:
//! ```text
//! InitialContextToken ::=
//!   [APPLICATION 0] IMPLICIT SEQUENCE {
//!     thisMech       MechType,
//!     innerContextToken ANY DEFINED BY thisMech
//!   }
//! ```
//! The outer tag is `0x60` (APPLICATION 0 constructed). Inside
//! is an OID (the mechanism) and the mechanism-specific blob
//! that follows. For SPNEGO the OID is `1.3.6.1.5.5.2`; for
//! plain Kerberos v5 it's `1.2.840.113554.1.2.2`.

use super::KerberosError;

/// SPNEGO mechanism OID per RFC 4178 §3.
pub const OID_SPNEGO: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x02];
/// Kerberos v5 mechanism OID per RFC 4121.
pub const OID_KRB5: &[u8] = &[
    0x2a, 0x86, 0x48, 0x86, 0xf7, 0x12, 0x01, 0x02, 0x02,
];
/// Microsoft "legacy" Kerberos v5 OID (sometimes seen from
/// older AD servers).
pub const OID_MS_KRB5: &[u8] = &[
    0x2a, 0x86, 0x48, 0x82, 0xf7, 0x12, 0x01, 0x02, 0x02,
];

/// Parsed initial-context-token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialContextToken<'a> {
    /// Mechanism OID — encoded form (no leading `0x06`/length).
    pub mech_oid: &'a [u8],
    /// Mechanism-specific payload (e.g. a SPNEGO NegTokenInit).
    pub inner: &'a [u8],
}

impl<'a> InitialContextToken<'a> {
    /// Parse the RFC 2743 §3.1 outer wrapping. Returns
    /// (`mech_oid`, `inner`) sliced into the original buffer.
    pub fn parse(input: &'a [u8]) -> Result<Self, KerberosError> {
        if input.is_empty() {
            return Err(KerberosError::Asn1("empty GSS token".into()));
        }
        if input[0] != 0x60 {
            return Err(KerberosError::Asn1(format!(
                "expected APPLICATION 0 tag 0x60, got 0x{:02x}",
                input[0]
            )));
        }
        let (body, _rest) = read_tlv(input, 0x60)?;
        // body := OID (tag 0x06) ++ mech-specific bytes
        if body.is_empty() || body[0] != 0x06 {
            return Err(KerberosError::Asn1(
                "missing thisMech OID in GSS token".into(),
            ));
        }
        let (oid, after_oid) = read_tlv(body, 0x06)?;
        Ok(InitialContextToken {
            mech_oid: oid,
            inner: after_oid,
        })
    }

    /// Convenience — true if this is a SPNEGO-wrapped token.
    pub fn is_spnego(&self) -> bool {
        self.mech_oid == OID_SPNEGO
    }

    /// Convenience — true if this is a plain Kerberos v5 token.
    pub fn is_krb5(&self) -> bool {
        self.mech_oid == OID_KRB5 || self.mech_oid == OID_MS_KRB5
    }
}

/// Build an InitialContextToken — wrap `inner` with the OID
/// preamble. The opposite of [`InitialContextToken::parse`].
pub fn wrap_initial_context_token(mech_oid: &[u8], inner: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(mech_oid.len() + inner.len() + 4);
    // OID
    body.push(0x06);
    push_length(&mut body, mech_oid.len());
    body.extend_from_slice(mech_oid);
    // inner mech-specific
    body.extend_from_slice(inner);
    let mut out = Vec::with_capacity(body.len() + 4);
    out.push(0x60);
    push_length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

// ── Local ASN.1 helpers (same shape as the LDAP BER helpers, but
//   we keep them duplicated here so the Kerberos module is
//   independent — RFC 2743 §3.1 is a separate spec).

pub(crate) fn read_tlv(bytes: &[u8], expected_tag: u8) -> Result<(&[u8], &[u8]), KerberosError> {
    if bytes.is_empty() {
        return Err(KerberosError::Asn1("unexpected EOF".into()));
    }
    if bytes[0] != expected_tag {
        return Err(KerberosError::Asn1(format!(
            "expected tag {expected_tag:#04x}, got {:#04x}",
            bytes[0]
        )));
    }
    let (len, after_len) = read_length(&bytes[1..])?;
    if after_len.len() < len {
        return Err(KerberosError::Asn1("TLV truncated".into()));
    }
    Ok((&after_len[..len], &after_len[len..]))
}

pub(crate) fn read_length(bytes: &[u8]) -> Result<(usize, &[u8]), KerberosError> {
    if bytes.is_empty() {
        return Err(KerberosError::Asn1("EOF reading length".into()));
    }
    let first = bytes[0];
    if first & 0x80 == 0 {
        return Ok((first as usize, &bytes[1..]));
    }
    let n = (first & 0x7F) as usize;
    if n == 0 {
        return Err(KerberosError::Asn1("indefinite length forbidden".into()));
    }
    if bytes.len() < 1 + n {
        return Err(KerberosError::Asn1("length field truncated".into()));
    }
    let mut len: usize = 0;
    for i in 0..n {
        len = (len << 8) | bytes[1 + i] as usize;
    }
    Ok((len, &bytes[1 + n..]))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_initial_context_token_extracts_spnego_oid() {
        // 60 11 — outer APPLICATION 0, length 17
        //   06 06 2b 06 01 05 05 02 — OID 1.3.6.1.5.5.2 (SPNEGO)
        //   01 02 03 04 05 06 07 08 09 — inner payload (placeholder)
        let bytes = [
            0x60, 0x11, 0x06, 0x06, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x02, 0x01, 0x02, 0x03, 0x04,
            0x05, 0x06, 0x07, 0x08, 0x09,
        ];
        let parsed = InitialContextToken::parse(&bytes).unwrap();
        assert!(parsed.is_spnego());
        assert_eq!(parsed.inner.len(), 9);
    }

    #[test]
    fn parse_initial_context_token_extracts_krb5_oid() {
        let mut bytes = vec![0x60, 0x0c, 0x06, 0x09];
        bytes.extend_from_slice(OID_KRB5);
        bytes.push(0xff);
        let parsed = InitialContextToken::parse(&bytes).unwrap();
        assert!(parsed.is_krb5());
    }

    #[test]
    fn parse_initial_context_token_rejects_wrong_outer_tag() {
        let bytes = [0x30, 0x02, 0x06, 0x00];
        let err = InitialContextToken::parse(&bytes).unwrap_err();
        assert!(matches!(err, KerberosError::Asn1(_)));
    }

    #[test]
    fn parse_initial_context_token_rejects_missing_oid() {
        // 60 02 04 00 — outer, inner is OCTET STRING not OID
        let bytes = [0x60, 0x02, 0x04, 0x00];
        assert!(InitialContextToken::parse(&bytes).is_err());
    }

    #[test]
    fn wrap_initial_context_token_round_trips_through_parse() {
        let inner = [0xaa, 0xbb, 0xcc];
        let wrapped = wrap_initial_context_token(OID_SPNEGO, &inner);
        let parsed = InitialContextToken::parse(&wrapped).unwrap();
        assert!(parsed.is_spnego());
        assert_eq!(parsed.inner, inner);
    }

    #[test]
    fn long_form_length_encoded() {
        // Build a 130-byte inner payload — exercises the long-form
        // length encoding in `push_length`.
        let inner = vec![0xee; 130];
        let wrapped = wrap_initial_context_token(OID_SPNEGO, &inner);
        let parsed = InitialContextToken::parse(&wrapped).unwrap();
        assert_eq!(parsed.inner.len(), 130);
    }
}
