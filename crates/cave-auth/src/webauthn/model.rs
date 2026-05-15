// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/data/AuthenticatorData.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/AttestedCredentialData.java
//   webauthn4j-core/src/main/java/com/webauthn4j/converter/AuthenticatorDataConverter.java
//
// AuthenticatorData — W3C WebAuthn §6.1.  Fixed prefix (37 bytes) +
// optional attestedCredentialData + optional extensions.  Both
// registration and authentication ceremonies carry an
// AuthenticatorData payload; only the AT/ED flags differ.

use crate::webauthn::cose::{CoseError, CoseKey};

/// One byte of flag bits (W3C §6.1):
///   bit 0 (LSB) UP    user-present
///   bit 1       RFU1
///   bit 2       UV    user-verified
///   bit 3       BE    backup eligibility
///   bit 4       BS    backup state
///   bit 5       RFU2
///   bit 6       AT    attested-credential-data
///   bit 7 (MSB) ED    extension-data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuthenticatorDataFlags {
    pub user_present: bool,
    pub user_verified: bool,
    pub backup_eligibility: bool,
    pub backup_state: bool,
    pub attested_credential_data: bool,
    pub extension_data: bool,
}

impl AuthenticatorDataFlags {
    pub fn from_byte(b: u8) -> Self {
        Self {
            user_present: b & 0b0000_0001 != 0,
            user_verified: b & 0b0000_0100 != 0,
            backup_eligibility: b & 0b0000_1000 != 0,
            backup_state: b & 0b0001_0000 != 0,
            attested_credential_data: b & 0b0100_0000 != 0,
            extension_data: b & 0b1000_0000 != 0,
        }
    }

    pub fn to_byte(self) -> u8 {
        let mut b = 0u8;
        if self.user_present {
            b |= 0b0000_0001;
        }
        if self.user_verified {
            b |= 0b0000_0100;
        }
        if self.backup_eligibility {
            b |= 0b0000_1000;
        }
        if self.backup_state {
            b |= 0b0001_0000;
        }
        if self.attested_credential_data {
            b |= 0b0100_0000;
        }
        if self.extension_data {
            b |= 0b1000_0000;
        }
        b
    }
}

/// W3C §6.5.2 attestedCredentialData.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestedCredentialData {
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    pub public_key: CoseKey,
}

impl AttestedCredentialData {
    /// Render AAGUID as standard 8-4-4-4-12 lowercase UUID.
    pub fn aaguid_to_string(aaguid: &[u8; 16]) -> String {
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            aaguid[0], aaguid[1], aaguid[2], aaguid[3], aaguid[4], aaguid[5], aaguid[6], aaguid[7],
            aaguid[8], aaguid[9], aaguid[10], aaguid[11], aaguid[12], aaguid[13], aaguid[14], aaguid[15]
        )
    }
}

/// W3C §6.1 authenticatorData.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatorData {
    pub rp_id_hash: [u8; 32],
    pub flags: AuthenticatorDataFlags,
    pub sign_count: u32,
    pub attested_credential_data: Option<AttestedCredentialData>,
    pub extension_data_cbor: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("truncated authData: needed {needed} bytes, have {have}")]
    Truncated { needed: usize, have: usize },
    #[error("invalid credentialId length: {0}")]
    BadCredentialIdLen(usize),
    #[error("COSE_Key parse failure: {0}")]
    Cose(#[from] CoseError),
    #[error("trailing bytes after authData ({0})")]
    TrailingBytes(usize),
}

impl AuthenticatorData {
    pub fn parse(buf: &[u8]) -> Result<Self, ParseError> {
        if buf.len() < 37 {
            return Err(ParseError::Truncated {
                needed: 37,
                have: buf.len(),
            });
        }
        let mut rp_id_hash = [0u8; 32];
        rp_id_hash.copy_from_slice(&buf[0..32]);
        let flags = AuthenticatorDataFlags::from_byte(buf[32]);
        let sign_count = u32::from_be_bytes([buf[33], buf[34], buf[35], buf[36]]);
        let mut cursor = 37usize;
        let attested = if flags.attested_credential_data {
            if buf.len() < cursor + 18 {
                return Err(ParseError::Truncated {
                    needed: cursor + 18,
                    have: buf.len(),
                });
            }
            let mut aaguid = [0u8; 16];
            aaguid.copy_from_slice(&buf[cursor..cursor + 16]);
            cursor += 16;
            let cred_id_len = u16::from_be_bytes([buf[cursor], buf[cursor + 1]]) as usize;
            cursor += 2;
            // W3C §6.5.2: credentialId length must be ≤ 1023 bytes.
            if cred_id_len == 0 || cred_id_len > 1023 {
                return Err(ParseError::BadCredentialIdLen(cred_id_len));
            }
            if buf.len() < cursor + cred_id_len {
                return Err(ParseError::Truncated {
                    needed: cursor + cred_id_len,
                    have: buf.len(),
                });
            }
            let credential_id = buf[cursor..cursor + cred_id_len].to_vec();
            cursor += cred_id_len;
            // The COSE_Key length is not encoded; ciborium reads until the
            // map terminator, after which we recover the byte count via
            // the writer round-trip.
            let (public_key, consumed) = consume_cose_key(&buf[cursor..])?;
            cursor += consumed;
            Some(AttestedCredentialData {
                aaguid,
                credential_id,
                public_key,
            })
        } else {
            None
        };
        let extension_data_cbor = if flags.extension_data {
            if cursor >= buf.len() {
                return Err(ParseError::Truncated {
                    needed: cursor + 1,
                    have: buf.len(),
                });
            }
            Some(buf[cursor..].to_vec())
        } else {
            None
        };
        // If ED=0 we expect to have consumed everything.
        if !flags.extension_data && cursor < buf.len() {
            return Err(ParseError::TrailingBytes(buf.len() - cursor));
        }
        Ok(Self {
            rp_id_hash,
            flags,
            sign_count,
            attested_credential_data: attested,
            extension_data_cbor,
        })
    }
}

/// Parse exactly one CBOR item from `buf` and return its decoded
/// COSE_Key plus the number of bytes consumed.  ciborium does not
/// expose a "how many bytes did you read" API on the reader, so we
/// re-encode the value and trust the canonical form to match.  This is
/// safe because COSE_Key is required to be CTAP2-canonical (RFC 8949
/// §4.2.1) and ciborium emits canonical CBOR.
fn consume_cose_key(buf: &[u8]) -> Result<(CoseKey, usize), ParseError> {
    let mut reader = std::io::Cursor::new(buf);
    let value: ciborium::Value = ciborium::de::from_reader(&mut reader)
        .map_err(|e| ParseError::Cose(CoseError::Cbor(e.to_string())))?;
    let consumed = reader.position() as usize;
    let mut reenc = Vec::new();
    ciborium::ser::into_writer(&value, &mut reenc)
        .map_err(|e| ParseError::Cose(CoseError::Cbor(e.to_string())))?;
    let key = CoseKey::from_cbor(&reenc)?;
    Ok((key, consumed))
}

/// Transport hints carried with a credential (W3C §5.8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    Usb,
    Nfc,
    Ble,
    Internal,
    Hybrid,
    SmartCard,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usb => "usb",
            Self::Nfc => "nfc",
            Self::Ble => "ble",
            Self::Internal => "internal",
            Self::Hybrid => "hybrid",
            Self::SmartCard => "smart-card",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "usb" => Some(Self::Usb),
            "nfc" => Some(Self::Nfc),
            "ble" => Some(Self::Ble),
            "internal" => Some(Self::Internal),
            "hybrid" => Some(Self::Hybrid),
            "smart-card" => Some(Self::SmartCard),
            _ => None,
        }
    }
}

/// W3C §5.1 "credential record" — what the RP persists per credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub credential_id: Vec<u8>,
    pub public_key: CoseKey,
    pub sign_counter: u32,
    pub transports: Vec<Transport>,
    pub aaguid: [u8; 16],
    pub attestation_format: String,
    /// Resident-key / passkey only — user handle the authenticator
    /// returned during registration.
    pub user_handle: Option<Vec<u8>>,
    pub backup_eligible: bool,
    pub backup_state: bool,
    pub uv_initialized: bool,
}
