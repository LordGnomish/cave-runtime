// SPDX-License-Identifier: AGPL-3.0-or-later
//! RED phase for XML Encryption (XML-ENC 1.1) for SAML
//! `<saml:EncryptedAssertion>`.
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/core/util/XMLEncryptionUtil.java
//!         saml-core/src/main/java/org/keycloak/saml/processing/api/util/KeyInfoTools.java
//!
//! Algorithms implemented:
//!  * AES-128-GCM data cipher (http://www.w3.org/2009/xmlenc11#aes128-gcm)
//!  * AES-256-GCM data cipher (http://www.w3.org/2009/xmlenc11#aes256-gcm)
//!  * RSA-OAEP-MGF1-SHA256 key transport
//!    (http://www.w3.org/2009/xmlenc11#rsa-oaep with MGF1-SHA256)

use cave_auth::saml::xmlenc::{
    aes128_gcm_decrypt, aes128_gcm_encrypt, aes256_gcm_decrypt, aes256_gcm_encrypt,
    encrypt_assertion, decrypt_assertion, rsa_oaep_sha256_unwrap, rsa_oaep_sha256_wrap,
    XmlEncKey, ALG_AES128_GCM, ALG_AES256_GCM, ALG_RSA_OAEP_MGF1P_SHA256,
};

#[test]
fn algorithm_urns_match_w3c() {
    assert_eq!(ALG_AES128_GCM, "http://www.w3.org/2009/xmlenc11#aes128-gcm");
    assert_eq!(ALG_AES256_GCM, "http://www.w3.org/2009/xmlenc11#aes256-gcm");
    assert_eq!(
        ALG_RSA_OAEP_MGF1P_SHA256,
        "http://www.w3.org/2009/xmlenc11#rsa-oaep"
    );
}

#[test]
fn aes128_gcm_round_trips() {
    let key = [7u8; 16];
    let plaintext = b"<saml:Assertion>secret</saml:Assertion>";
    let (ct, nonce) = aes128_gcm_encrypt(&key, plaintext).unwrap();
    let pt = aes128_gcm_decrypt(&key, &nonce, &ct).unwrap();
    assert_eq!(pt, plaintext);
}

#[test]
fn aes256_gcm_round_trips() {
    let key = [9u8; 32];
    let plaintext = b"<saml:Assertion>secret-256</saml:Assertion>";
    let (ct, nonce) = aes256_gcm_encrypt(&key, plaintext).unwrap();
    let pt = aes256_gcm_decrypt(&key, &nonce, &ct).unwrap();
    assert_eq!(pt, plaintext);
}

#[test]
fn aes128_gcm_rejects_tampered_ct() {
    let key = [7u8; 16];
    let (mut ct, nonce) = aes128_gcm_encrypt(&key, b"hello").unwrap();
    ct[0] ^= 0x01;
    assert!(aes128_gcm_decrypt(&key, &nonce, &ct).is_err());
}

#[test]
fn rsa_oaep_sha256_round_trips() {
    use rsa::{pkcs8::EncodePublicKey, RsaPrivateKey, RsaPublicKey};
    let mut rng = rand::thread_rng();
    let sk = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pk = RsaPublicKey::from(&sk);
    let pk_der = pk.to_public_key_der().unwrap().as_bytes().to_vec();
    let cek = [42u8; 32]; // a 256-bit AES key
    let wrapped = rsa_oaep_sha256_wrap(&pk_der, &cek).unwrap();
    let unwrapped = rsa_oaep_sha256_unwrap(&sk, &wrapped).unwrap();
    assert_eq!(unwrapped, cek);
}

#[test]
fn encrypted_assertion_round_trips_through_xml() {
    use rsa::{pkcs8::EncodePublicKey, RsaPrivateKey, RsaPublicKey};
    let mut rng = rand::thread_rng();
    let sk = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pk = RsaPublicKey::from(&sk);
    let pk_der = pk.to_public_key_der().unwrap().as_bytes().to_vec();
    let plaintext = b"<saml:Assertion ID=\"_a\" Version=\"2.0\">payload</saml:Assertion>";
    let encrypted_xml = encrypt_assertion(plaintext, &pk_der, XmlEncKey::Aes256Gcm).unwrap();
    // Wire format must contain the algorithm URN attribute.
    let s = std::str::from_utf8(&encrypted_xml).unwrap();
    assert!(s.contains("EncryptedAssertion"), "wraps EncryptedAssertion");
    assert!(s.contains("aes256-gcm"), "advertises AES-256-GCM");
    assert!(s.contains("rsa-oaep"), "advertises RSA-OAEP-SHA256 key transport");

    let recovered = decrypt_assertion(&encrypted_xml, &sk).unwrap();
    assert_eq!(recovered, plaintext);
}

#[test]
fn encrypted_assertion_aes128_variant_round_trips() {
    use rsa::{pkcs8::EncodePublicKey, RsaPrivateKey, RsaPublicKey};
    let mut rng = rand::thread_rng();
    let sk = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pk = RsaPublicKey::from(&sk);
    let pk_der = pk.to_public_key_der().unwrap().as_bytes().to_vec();
    let plaintext = b"<saml:Assertion>plain</saml:Assertion>";
    let encrypted_xml = encrypt_assertion(plaintext, &pk_der, XmlEncKey::Aes128Gcm).unwrap();
    let s = std::str::from_utf8(&encrypted_xml).unwrap();
    assert!(s.contains("aes128-gcm"));
    let recovered = decrypt_assertion(&encrypted_xml, &sk).unwrap();
    assert_eq!(recovered, plaintext);
}
