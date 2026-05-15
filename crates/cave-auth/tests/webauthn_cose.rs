// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/test/java/com/webauthn4j/data/attestation/authenticator/COSEKeyTest.java
//
// RED — COSE_Key parsing must accept the four WebAuthn-L3 algorithms
// and reject malformed input.  The lib does not yet contain the
// `webauthn::cose` module, so this test fails to compile until the
// GREEN commit lands.

use cave_auth::webauthn::cose::{CoseAlgorithm, CoseError, CoseKey};
use ciborium::Value;

fn encode(pairs: Vec<(Value, Value)>) -> Vec<u8> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(&Value::Map(pairs), &mut buf).unwrap();
    buf
}

fn int(i: i64) -> Value {
    Value::Integer(i.into())
}

fn bytes(b: Vec<u8>) -> Value {
    Value::Bytes(b)
}

#[test]
fn cose_alg_round_trip() {
    for a in [
        CoseAlgorithm::Es256,
        CoseAlgorithm::Es384,
        CoseAlgorithm::Rs256,
        CoseAlgorithm::EdDsa,
    ] {
        assert_eq!(CoseAlgorithm::from_i64(a.as_i64()), Some(a));
        assert!(!a.name().is_empty());
    }
}

#[test]
fn cose_alg_unknown_is_none() {
    assert!(CoseAlgorithm::from_i64(-9999).is_none());
}

#[test]
fn parses_es256_key() {
    let cbor = encode(vec![
        (int(1), int(2)),
        (int(3), int(-7)),
        (int(-1), int(1)),
        (int(-2), bytes(vec![0xAA; 32])),
        (int(-3), bytes(vec![0xBB; 32])),
    ]);
    let key = CoseKey::from_cbor(&cbor).unwrap();
    assert_eq!(key.alg(), CoseAlgorithm::Es256);
}

#[test]
fn parses_es384_key() {
    let cbor = encode(vec![
        (int(1), int(2)),
        (int(3), int(-35)),
        (int(-1), int(2)),
        (int(-2), bytes(vec![0xCC; 48])),
        (int(-3), bytes(vec![0xDD; 48])),
    ]);
    let key = CoseKey::from_cbor(&cbor).unwrap();
    assert_eq!(key.alg(), CoseAlgorithm::Es384);
}

#[test]
fn parses_rs256_key() {
    let cbor = encode(vec![
        (int(1), int(3)),
        (int(3), int(-257)),
        (int(-1), bytes(vec![0xEE; 256])),
        (int(-2), bytes(vec![0x01, 0x00, 0x01])),
    ]);
    let key = CoseKey::from_cbor(&cbor).unwrap();
    assert_eq!(key.alg(), CoseAlgorithm::Rs256);
}

#[test]
fn rejects_short_rsa_modulus() {
    let cbor = encode(vec![
        (int(1), int(3)),
        (int(3), int(-257)),
        (int(-1), bytes(vec![0xEE; 64])),
        (int(-2), bytes(vec![0x01, 0x00, 0x01])),
    ]);
    assert!(matches!(
        CoseKey::from_cbor(&cbor),
        Err(CoseError::BadCoordinate { .. })
    ));
}

#[test]
fn parses_ed25519_key() {
    let cbor = encode(vec![
        (int(1), int(1)),
        (int(3), int(-8)),
        (int(-1), int(6)),
        (int(-2), bytes(vec![0x55; 32])),
    ]);
    let key = CoseKey::from_cbor(&cbor).unwrap();
    assert_eq!(key.alg(), CoseAlgorithm::EdDsa);
}

#[test]
fn rejects_unknown_alg() {
    let cbor = encode(vec![(int(1), int(2)), (int(3), int(-99))]);
    assert!(matches!(
        CoseKey::from_cbor(&cbor),
        Err(CoseError::UnsupportedAlg { .. })
    ));
}

#[test]
fn rejects_missing_kty() {
    let cbor = encode(vec![(int(3), int(-7))]);
    assert!(matches!(
        CoseKey::from_cbor(&cbor),
        Err(CoseError::MissingLabel { label: 1 })
    ));
}

#[test]
fn rejects_wrong_curve_for_es256() {
    let cbor = encode(vec![
        (int(1), int(2)),
        (int(3), int(-7)),
        (int(-1), int(2)),
        (int(-2), bytes(vec![0; 48])),
        (int(-3), bytes(vec![0; 48])),
    ]);
    assert!(matches!(
        CoseKey::from_cbor(&cbor),
        Err(CoseError::UnsupportedCrv { .. })
    ));
}

#[test]
fn rejects_non_cbor_input() {
    assert!(CoseKey::from_cbor(b"not cbor at all").is_err());
}
