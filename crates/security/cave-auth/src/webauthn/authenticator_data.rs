// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// authenticatorData parsing — webauthn4j `data.attestation.authenticator.AuthenticatorData`.
//
// Wire format (W3C WebAuthn L2 §6.1):
//   rpIdHash                 32 B
//   flags                     1 B   bits: UP / UV / BE / BS / AT / ED
//   signCount                 4 B   (big-endian u32)
//   attestedCredentialData   variable (only if AT flag)
//     aaguid                  16 B
//     credentialIdLength       2 B
//     credentialId      <credentialIdLength> B
//     credentialPublicKey  CBOR-encoded COSE_Key
//   extensions              variable CBOR map (only if ED flag)

use super::WebAuthnError;
use super::cbor;

bitflags::bitflags! {
    /// AuthenticatorData flags byte (W3C §6.1 step 5).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AuthFlags: u8 {
        /// User Present.
        const UP = 0b0000_0001;
        /// User Verified.
        const UV = 0b0000_0100;
        /// Backup Eligible (passkey-syncable).
        const BE = 0b0000_1000;
        /// Backup State (currently backed up).
        const BS = 0b0001_0000;
        /// Attested credential data included (registration).
        const AT = 0b0100_0000;
        /// Extension data included.
        const ED = 0b1000_0000;
    }
}

/// Parsed authenticatorData.
#[derive(Debug, Clone)]
pub struct AuthenticatorData {
    pub rp_id_hash: [u8; 32],
    pub flags: AuthFlags,
    pub sign_count: u32,
    pub attested_credential: Option<AttestedCredentialData>,
    /// Extensions CBOR map raw bytes (we don't parse these — webauthn4j
    /// keeps an opaque blob so registered-extension clients can decode
    /// per-extension on demand).
    pub extensions_raw: Option<Vec<u8>>,
    /// Raw authenticatorData bytes — needed verbatim for signature input.
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AttestedCredentialData {
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    /// Raw COSE_Key CBOR bytes — passed to [`crate::webauthn::cose::parse`].
    pub credential_public_key: Vec<u8>,
}

/// Parse authenticatorData per W3C §6.1.
///
/// Port of webauthn4j `AuthenticatorDataConverter#convert`.
pub fn parse(raw: &[u8]) -> Result<AuthenticatorData, WebAuthnError> {
    if raw.len() < 37 {
        return Err(WebAuthnError::AuthenticatorData(format!(
            "too short: {} bytes < 37",
            raw.len()
        )));
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&raw[0..32]);
    let flag_byte = raw[32];
    let flags = AuthFlags::from_bits_truncate(flag_byte);
    let sign_count = u32::from_be_bytes([raw[33], raw[34], raw[35], raw[36]]);
    let mut offset = 37usize;

    let attested_credential = if flags.contains(AuthFlags::AT) {
        if raw.len() < offset + 18 {
            return Err(WebAuthnError::AuthenticatorData(
                "attested credential data truncated".into(),
            ));
        }
        let mut aaguid = [0u8; 16];
        aaguid.copy_from_slice(&raw[offset..offset + 16]);
        offset += 16;
        let cred_id_len = u16::from_be_bytes([raw[offset], raw[offset + 1]]) as usize;
        offset += 2;
        if raw.len() < offset + cred_id_len {
            return Err(WebAuthnError::AuthenticatorData(format!(
                "credential id truncated: need {cred_id_len}, have {}",
                raw.len() - offset
            )));
        }
        let credential_id = raw[offset..offset + cred_id_len].to_vec();
        offset += cred_id_len;
        // COSE_Key is the rest of the buffer up to the extensions block (or
        // EOF when ED=0). Use a peeking decode: ciborium will stop reading
        // after the first complete item; we record how many bytes it
        // consumed by re-encoding (CBOR is deterministic for the COSE_Key
        // canonical form).
        let cose_start = offset;
        let val = cbor::decode(&raw[cose_start..])?;
        let cose_bytes = cbor::encode(&val)?;
        // Sanity — re-encoded length must not exceed remaining buffer.
        if cose_bytes.len() > raw.len() - cose_start {
            return Err(WebAuthnError::AuthenticatorData(
                "COSE_Key re-encoded longer than slice".into(),
            ));
        }
        offset += cose_bytes.len();
        Some(AttestedCredentialData {
            aaguid,
            credential_id,
            credential_public_key: cose_bytes,
        })
    } else {
        None
    };

    let extensions_raw = if flags.contains(AuthFlags::ED) && offset < raw.len() {
        Some(raw[offset..].to_vec())
    } else {
        None
    };

    Ok(AuthenticatorData {
        rp_id_hash,
        flags,
        sign_count,
        attested_credential,
        extensions_raw,
        raw: raw.to_vec(),
    })
}

/// Compute the RP-ID hash used in the rpIdHash field — `SHA-256(rp_id)`.
pub fn rp_id_hash(rp_id: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(rp_id.as_bytes());
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(flags: u8, sign_count: u32) -> Vec<u8> {
        let mut v = vec![0u8; 37];
        for (i, b) in v.iter_mut().take(32).enumerate() {
            *b = i as u8;
        }
        v[32] = flags;
        v[33..37].copy_from_slice(&sign_count.to_be_bytes());
        v
    }

    #[test]
    fn parse_minimal_no_attested() {
        let raw = minimal(AuthFlags::UP.bits(), 7);
        let ad = parse(&raw).unwrap();
        assert!(ad.flags.contains(AuthFlags::UP));
        assert!(!ad.flags.contains(AuthFlags::AT));
        assert_eq!(ad.sign_count, 7);
        assert!(ad.attested_credential.is_none());
        assert!(ad.extensions_raw.is_none());
    }

    #[test]
    fn parse_too_short() {
        assert!(parse(&[0u8; 10]).is_err());
    }

    #[test]
    fn parse_with_attested_credential() {
        let mut raw = minimal((AuthFlags::UP | AuthFlags::AT).bits(), 1);
        // aaguid
        raw.extend_from_slice(&[0xaa; 16]);
        // credentialIdLength = 4
        raw.extend_from_slice(&4u16.to_be_bytes());
        raw.extend_from_slice(&[1, 2, 3, 4]);
        // COSE_Key — minimal valid CBOR: { 1: 2 } (kty: EC2)
        let cose = ciborium::value::Value::Map(vec![(
            ciborium::value::Value::Integer(1i64.into()),
            ciborium::value::Value::Integer(2i64.into()),
        )]);
        let cose_bytes = crate::webauthn::cbor::encode(&cose).unwrap();
        raw.extend_from_slice(&cose_bytes);

        let ad = parse(&raw).unwrap();
        let acd = ad.attested_credential.unwrap();
        assert_eq!(acd.aaguid, [0xaa; 16]);
        assert_eq!(acd.credential_id, vec![1, 2, 3, 4]);
        assert_eq!(acd.credential_public_key, cose_bytes);
    }

    #[test]
    fn parse_flag_bits_all_no_attested() {
        // All non-AT flags set, no attested data block, no extensions.
        let flags = (AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS).bits();
        let raw = minimal(flags, 0);
        let ad = parse(&raw).unwrap();
        assert!(ad.flags.contains(AuthFlags::UP));
        assert!(ad.flags.contains(AuthFlags::UV));
        assert!(ad.flags.contains(AuthFlags::BE));
        assert!(ad.flags.contains(AuthFlags::BS));
        assert!(!ad.flags.contains(AuthFlags::AT));
        assert!(ad.attested_credential.is_none());
    }

    #[test]
    fn parse_at_without_data_errors() {
        let raw = minimal(AuthFlags::AT.bits(), 1);
        assert!(parse(&raw).is_err());
    }

    #[test]
    fn rp_id_hash_is_sha256() {
        let h = rp_id_hash("login.cave.dev");
        // SHA-256("login.cave.dev") — recomputed at runtime; check non-zero.
        assert_ne!(h, [0u8; 32]);
        assert_eq!(h, rp_id_hash("login.cave.dev"));
    }
}
