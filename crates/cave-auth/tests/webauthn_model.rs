// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/test/java/com/webauthn4j/data/AuthenticatorDataTest.java
//   webauthn4j-core/src/test/java/com/webauthn4j/data/AttestedCredentialDataTest.java
//
// RED — AuthenticatorData byte layout (W3C WebAuthn §6.1):
//
//   rpIdHash      32 bytes
//   flags         1 byte   UP | UV | BE | BS | AT | ED
//   signCount     4 bytes  big-endian
//   [attestedCredentialData (if AT=1)]
//     aaguid           16 bytes
//     credentialIdLen  2 bytes big-endian
//     credentialId     N bytes
//     credentialPubKey COSE_Key (variable CBOR)
//   [extensions (if ED=1)]                  CBOR map

use cave_auth::webauthn::cose::CoseAlgorithm;
use cave_auth::webauthn::model::{
    AttestedCredentialData, AuthenticatorData, AuthenticatorDataFlags, ParseError,
};

fn build_min_auth_data(flags: u8, sign_count: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(37);
    v.extend_from_slice(&[0u8; 32]); // rpIdHash
    v.push(flags);
    v.extend_from_slice(&sign_count.to_be_bytes());
    v
}

#[test]
fn flag_bits_decode() {
    let f = AuthenticatorDataFlags::from_byte(0b0100_0101);
    assert!(f.user_present);
    assert!(f.user_verified);
    assert!(f.attested_credential_data);
    assert!(!f.extension_data);
    assert!(!f.backup_eligibility);
    assert!(!f.backup_state);
}

#[test]
fn parses_minimal_authdata_no_attestation() {
    let raw = build_min_auth_data(0b0000_0001, 7); // UP only
    let ad = AuthenticatorData::parse(&raw).unwrap();
    assert!(ad.flags.user_present);
    assert!(!ad.flags.attested_credential_data);
    assert_eq!(ad.sign_count, 7);
    assert!(ad.attested_credential_data.is_none());
}

#[test]
fn parses_authdata_with_attested_credential_data_es256() {
    // flags = AT|UP
    let mut raw = build_min_auth_data(0b0100_0001, 1);
    // aaguid
    raw.extend_from_slice(&[0xAB; 16]);
    // credentialId len = 8, content
    raw.extend_from_slice(&8u16.to_be_bytes());
    raw.extend_from_slice(&[0xCD; 8]);
    // COSE_Key ES256
    let cbor = encode_cose_es256();
    raw.extend_from_slice(&cbor);

    let ad = AuthenticatorData::parse(&raw).unwrap();
    let cred = ad.attested_credential_data.expect("attested data");
    assert_eq!(cred.aaguid, [0xAB; 16]);
    assert_eq!(cred.credential_id.len(), 8);
    assert_eq!(cred.public_key.alg(), CoseAlgorithm::Es256);
}

#[test]
fn rejects_truncated_auth_data() {
    let raw = vec![0u8; 5];
    let err = AuthenticatorData::parse(&raw).unwrap_err();
    assert!(matches!(err, ParseError::Truncated { .. }));
}

#[test]
fn rejects_attestation_flag_without_payload() {
    let raw = build_min_auth_data(0b0100_0000, 1);
    assert!(matches!(
        AuthenticatorData::parse(&raw),
        Err(ParseError::Truncated { .. })
    ));
}

#[test]
fn aaguid_roundtrip_lowercase_uuid_str() {
    let bytes: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];
    let s = AttestedCredentialData::aaguid_to_string(&bytes);
    assert_eq!(s, "01234567-89ab-cdef-fedc-ba9876543210");
}

fn encode_cose_es256() -> Vec<u8> {
    use ciborium::Value;
    let pairs: Vec<(Value, Value)> = vec![
        (Value::Integer(1.into()), Value::Integer(2.into())),
        (Value::Integer(3.into()), Value::Integer((-7i64).into())),
        (Value::Integer((-1i64).into()), Value::Integer(1.into())),
        (
            Value::Integer((-2i64).into()),
            Value::Bytes(vec![0x11; 32]),
        ),
        (
            Value::Integer((-3i64).into()),
            Value::Bytes(vec![0x22; 32]),
        ),
    ];
    let mut buf = Vec::new();
    ciborium::ser::into_writer(&Value::Map(pairs), &mut buf).unwrap();
    buf
}
