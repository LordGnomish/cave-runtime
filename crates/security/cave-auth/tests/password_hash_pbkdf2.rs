// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD line-port of Keycloak's PBKDF2 password hash provider.
// Upstream (Apache-2.0):
//   server-spi-private/.../credential/hash/Pbkdf2PasswordHashProvider.java
//
// Derivation is checked against the published PBKDF2 test vectors:
//   - RFC 6070 (PBKDF2-HMAC-SHA1)
//   - the canonical PBKDF2-HMAC-SHA256 vectors (RFC 7914 worked examples)
// and the encode/verify round-trip mirrors `encodedCredential` + `verify`.

use cave_auth::password_hash::{Pbkdf2Alg, Pbkdf2PasswordHashProvider};

fn provider(alg: Pbkdf2Alg) -> Pbkdf2PasswordHashProvider {
    // Keycloak DEFAULT_DERIVED_KEY_SIZE = 512 bits.
    Pbkdf2PasswordHashProvider::new(alg, 27_500, 512)
}

#[test]
fn pbkdf2_hmac_sha1_rfc6070_vectors() {
    let p = provider(Pbkdf2Alg::HmacSha1);
    assert_eq!(
        hex::encode(p.derive(b"password", b"salt", 1, 20)),
        "0c60c80f961f0e71f3a9b524af6012062fe037a6"
    );
    assert_eq!(
        hex::encode(p.derive(b"password", b"salt", 2, 20)),
        "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"
    );
}

#[test]
fn pbkdf2_hmac_sha256_vectors() {
    let p = provider(Pbkdf2Alg::HmacSha256);
    assert_eq!(
        hex::encode(p.derive(b"password", b"salt", 1, 32)),
        "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
    );
    assert_eq!(
        hex::encode(p.derive(b"password", b"salt", 2, 32)),
        "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
    );
}

#[test]
fn encode_then_verify_round_trips() {
    let p = provider(Pbkdf2Alg::HmacSha256);
    let salt = [0xA5u8; 16];
    let encoded = p.encode("hunter2", 1000, &salt);

    // 512-bit derived key -> 64 raw bytes -> standard base64.
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded)
        .expect("encoded credential is standard base64");
    assert_eq!(decoded.len(), 64, "default derived key size is 512 bits");

    // Verify reproduces the same hash from salt + iterations.
    assert!(p.verify("hunter2", &salt, 1000, &encoded));
    // Wrong password fails.
    assert!(!p.verify("hunter3", &salt, 1000, &encoded));
    // Wrong iteration count fails (different stretch).
    assert!(!p.verify("hunter2", &salt, 999, &encoded));
}

#[test]
fn verify_honours_stored_key_size_not_provider_default() {
    // A credential stored with a 256-bit key must verify even though the provider
    // default is 512 bits — exactly Keycloak's `keySize(credential)` behaviour.
    let p = provider(Pbkdf2Alg::HmacSha256);
    let salt = [0x11u8; 16];
    let short = provider(Pbkdf2Alg::HmacSha256).encode_with_size("pw", 500, &salt, 256);
    assert_eq!(
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &short).unwrap().len(),
        32
    );
    assert!(p.verify("pw", &salt, 500, &short));
}
