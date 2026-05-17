// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/core/util/XMLSignatureUtil.java
//
// XMLDSig ECDSA signing for SAML 2.0 — RED placeholder. The public
// surface and full test suite land in this commit so the failures
// pin the contract; the GREEN commit replaces every body with the
// real ECDSA implementation against `p256`/`p384`/`p521`.

#![allow(clippy::result_large_err)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use super::SamlError;

pub const ALG_ECDSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";
pub const ALG_ECDSA_SHA384: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384";
pub const ALG_ECDSA_SHA512: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha512";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcdsaCurve {
    P256,
    P384,
    P521,
}

impl EcdsaCurve {
    pub fn scalar_len(self) -> usize {
        0 // RED: deliberately wrong, will be 32/48/66 in GREEN
    }
    pub fn natural_hash(self) -> HashAlg {
        HashAlg::Sha256
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlg {
    pub fn xmldsig_alg(self) -> &'static str {
        // RED: returns empty string so the URN-mapping test fails.
        ""
    }
}

pub enum EcdsaSigningKey {
    P256,
    P384,
    P521,
}

pub enum EcdsaVerifyingKey {
    P256,
    P384,
    P521,
}

pub struct EcdsaKeyPair {
    pub curve: EcdsaCurve,
    pub signing: EcdsaSigningKey,
    pub verifying: EcdsaVerifyingKey,
}

pub fn generate_keypair(curve: EcdsaCurve) -> EcdsaKeyPair {
    let (s, v) = match curve {
        EcdsaCurve::P256 => (EcdsaSigningKey::P256, EcdsaVerifyingKey::P256),
        EcdsaCurve::P384 => (EcdsaSigningKey::P384, EcdsaVerifyingKey::P384),
        EcdsaCurve::P521 => (EcdsaSigningKey::P521, EcdsaVerifyingKey::P521),
    };
    EcdsaKeyPair { curve, signing: s, verifying: v }
}

impl EcdsaSigningKey {
    pub fn to_pkcs8_pem(&self) -> Result<String, SamlError> {
        Err(SamlError::InvalidSignature("RED: to_pkcs8_pem unimplemented".into()))
    }
    pub fn from_pkcs8_pem(_pem: &str, _curve: EcdsaCurve) -> Result<Self, SamlError> {
        Err(SamlError::InvalidSignature("RED: from_pkcs8_pem unimplemented".into()))
    }
    pub fn curve(&self) -> EcdsaCurve {
        match self {
            EcdsaSigningKey::P256 => EcdsaCurve::P256,
            EcdsaSigningKey::P384 => EcdsaCurve::P384,
            EcdsaSigningKey::P521 => EcdsaCurve::P521,
        }
    }
}

impl EcdsaVerifyingKey {
    pub fn to_public_key_pem(&self) -> Result<String, SamlError> {
        Err(SamlError::InvalidSignature("RED: to_public_key_pem unimplemented".into()))
    }
    pub fn from_public_key_pem(_pem: &str, _curve: EcdsaCurve) -> Result<Self, SamlError> {
        Err(SamlError::InvalidSignature("RED: from_public_key_pem unimplemented".into()))
    }
    pub fn curve(&self) -> EcdsaCurve {
        match self {
            EcdsaVerifyingKey::P256 => EcdsaCurve::P256,
            EcdsaVerifyingKey::P384 => EcdsaCurve::P384,
            EcdsaVerifyingKey::P521 => EcdsaCurve::P521,
        }
    }
}

pub fn sign_xml_canonical(
    _c14n_bytes: &[u8],
    _key: &EcdsaSigningKey,
    _hash: HashAlg,
) -> Result<String, SamlError> {
    Err(SamlError::InvalidSignature("RED: sign_xml_canonical unimplemented".into()))
}

pub fn verify_xml_canonical(
    _c14n_bytes: &[u8],
    _signature_b64: &str,
    _key: &EcdsaVerifyingKey,
    _hash: HashAlg,
) -> Result<(), SamlError> {
    Err(SamlError::InvalidSignature("RED: verify_xml_canonical unimplemented".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── 6 roundtrip tests: 3 curves × {Assertion, Metadata} ───

    #[test]
    fn p256_roundtrip_assertion() {
        let kp = generate_keypair(EcdsaCurve::P256);
        let body = b"<saml:Assertion>p256 assertion body</saml:Assertion>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha256).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha256).unwrap();
    }

    #[test]
    fn p256_roundtrip_metadata() {
        let kp = generate_keypair(EcdsaCurve::P256);
        let body = b"<md:EntityDescriptor>p256 metadata</md:EntityDescriptor>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha256).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha256).unwrap();
    }

    #[test]
    fn p384_roundtrip_assertion() {
        let kp = generate_keypair(EcdsaCurve::P384);
        let body = b"<saml:Assertion>p384 assertion body</saml:Assertion>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha384).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha384).unwrap();
    }

    #[test]
    fn p384_roundtrip_metadata() {
        let kp = generate_keypair(EcdsaCurve::P384);
        let body = b"<md:EntityDescriptor>p384 metadata</md:EntityDescriptor>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha384).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha384).unwrap();
    }

    #[test]
    fn p521_roundtrip_assertion() {
        let kp = generate_keypair(EcdsaCurve::P521);
        let body = b"<saml:Assertion>p521 assertion body</saml:Assertion>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha512).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha512).unwrap();
    }

    #[test]
    fn p521_roundtrip_metadata() {
        let kp = generate_keypair(EcdsaCurve::P521);
        let body = b"<md:EntityDescriptor>p521 metadata</md:EntityDescriptor>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha512).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha512).unwrap();
    }

    // ─── 3 KAT (round-trip determinism / serialization correctness) ───

    #[test]
    fn kat_p256_signature_length_is_64_bytes() {
        let kp = generate_keypair(EcdsaCurve::P256);
        let sig_b64 = sign_xml_canonical(b"kat-p256", &kp.signing, HashAlg::Sha256).unwrap();
        let raw = B64.decode(&sig_b64).unwrap();
        assert_eq!(raw.len(), 64);
    }

    #[test]
    fn kat_p384_signature_length_is_96_bytes() {
        let kp = generate_keypair(EcdsaCurve::P384);
        let sig_b64 = sign_xml_canonical(b"kat-p384", &kp.signing, HashAlg::Sha384).unwrap();
        let raw = B64.decode(&sig_b64).unwrap();
        assert_eq!(raw.len(), 96);
    }

    #[test]
    fn kat_p521_signature_length_is_132_bytes() {
        let kp = generate_keypair(EcdsaCurve::P521);
        let sig_b64 = sign_xml_canonical(b"kat-p521", &kp.signing, HashAlg::Sha512).unwrap();
        let raw = B64.decode(&sig_b64).unwrap();
        assert_eq!(raw.len(), 132);
    }

    // ─── 3 negative tests ───

    #[test]
    fn verify_rejects_wrong_key() {
        let signer = generate_keypair(EcdsaCurve::P256);
        let other = generate_keypair(EcdsaCurve::P256);
        let body = b"<saml:Assertion>wrong key</saml:Assertion>";
        let sig = sign_xml_canonical(body, &signer.signing, HashAlg::Sha256).unwrap();
        assert!(matches!(
            verify_xml_canonical(body, &sig, &other.verifying, HashAlg::Sha256).unwrap_err(),
            SamlError::InvalidSignature(_)
        ));
    }

    #[test]
    fn verify_rejects_wrong_hash() {
        let kp = generate_keypair(EcdsaCurve::P384);
        let body = b"<saml:Assertion>wrong hash</saml:Assertion>";
        let sig = sign_xml_canonical(body, &kp.signing, HashAlg::Sha384).unwrap();
        assert!(matches!(
            verify_xml_canonical(body, &sig, &kp.verifying, HashAlg::Sha256).unwrap_err(),
            SamlError::InvalidSignature(_)
        ));
    }

    #[test]
    fn verify_rejects_tampered_c14n_bytes() {
        let kp = generate_keypair(EcdsaCurve::P521);
        let original = b"<saml:Assertion>original c14n bytes</saml:Assertion>";
        let tampered = b"<saml:Assertion>tampered c14n bytes</saml:Assertion>";
        let sig = sign_xml_canonical(original, &kp.signing, HashAlg::Sha512).unwrap();
        assert!(matches!(
            verify_xml_canonical(tampered, &sig, &kp.verifying, HashAlg::Sha512).unwrap_err(),
            SamlError::InvalidSignature(_)
        ));
    }

    // ─── 9 cross-algo dispatch ───

    fn cross_case(curve: EcdsaCurve, signed: HashAlg, verify: HashAlg) {
        let kp = generate_keypair(curve);
        let body = b"<saml:Assertion>cross-algo</saml:Assertion>";
        let sig = sign_xml_canonical(body, &kp.signing, signed).unwrap();
        let result = verify_xml_canonical(body, &sig, &kp.verifying, verify);
        if signed == verify {
            assert!(result.is_ok(), "{:?}/{:?}/{:?}", curve, signed, verify);
        } else {
            assert!(result.is_err(), "{:?}/{:?}/{:?}", curve, signed, verify);
        }
    }

    #[test] fn cross_p256_sha256_sha256() { cross_case(EcdsaCurve::P256, HashAlg::Sha256, HashAlg::Sha256); }
    #[test] fn cross_p256_sha256_sha384() { cross_case(EcdsaCurve::P256, HashAlg::Sha256, HashAlg::Sha384); }
    #[test] fn cross_p256_sha384_sha512() { cross_case(EcdsaCurve::P256, HashAlg::Sha384, HashAlg::Sha512); }
    #[test] fn cross_p384_sha384_sha384() { cross_case(EcdsaCurve::P384, HashAlg::Sha384, HashAlg::Sha384); }
    #[test] fn cross_p384_sha256_sha384() { cross_case(EcdsaCurve::P384, HashAlg::Sha256, HashAlg::Sha384); }
    #[test] fn cross_p384_sha512_sha256() { cross_case(EcdsaCurve::P384, HashAlg::Sha512, HashAlg::Sha256); }
    #[test] fn cross_p521_sha512_sha512() { cross_case(EcdsaCurve::P521, HashAlg::Sha512, HashAlg::Sha512); }
    #[test] fn cross_p521_sha256_sha512() { cross_case(EcdsaCurve::P521, HashAlg::Sha256, HashAlg::Sha512); }
    #[test] fn cross_p521_sha384_sha256() { cross_case(EcdsaCurve::P521, HashAlg::Sha384, HashAlg::Sha256); }

    // ─── 6 PKCS#8 PEM round-trips ───

    fn pkcs8_signing_roundtrip(curve: EcdsaCurve) {
        let kp = generate_keypair(curve);
        let pem = kp.signing.to_pkcs8_pem().unwrap();
        assert!(pem.contains("-----BEGIN PRIVATE KEY-----"));
        let restored = EcdsaSigningKey::from_pkcs8_pem(&pem, curve).unwrap();
        let body = b"<x/>";
        let h = curve.natural_hash();
        let sig = sign_xml_canonical(body, &restored, h).unwrap();
        verify_xml_canonical(body, &sig, &kp.verifying, h).unwrap();
    }

    fn spki_verifying_roundtrip(curve: EcdsaCurve) {
        let kp = generate_keypair(curve);
        let pem = kp.verifying.to_public_key_pem().unwrap();
        assert!(pem.contains("-----BEGIN PUBLIC KEY-----"));
        let restored = EcdsaVerifyingKey::from_public_key_pem(&pem, curve).unwrap();
        let body = b"<x/>";
        let h = curve.natural_hash();
        let sig = sign_xml_canonical(body, &kp.signing, h).unwrap();
        verify_xml_canonical(body, &sig, &restored, h).unwrap();
    }

    #[test] fn pkcs8_p256_signing_pem_roundtrip()  { pkcs8_signing_roundtrip(EcdsaCurve::P256); }
    #[test] fn pkcs8_p384_signing_pem_roundtrip()  { pkcs8_signing_roundtrip(EcdsaCurve::P384); }
    #[test] fn pkcs8_p521_signing_pem_roundtrip()  { pkcs8_signing_roundtrip(EcdsaCurve::P521); }
    #[test] fn spki_p256_verifying_pem_roundtrip() { spki_verifying_roundtrip(EcdsaCurve::P256); }
    #[test] fn spki_p384_verifying_pem_roundtrip() { spki_verifying_roundtrip(EcdsaCurve::P384); }
    #[test] fn spki_p521_verifying_pem_roundtrip() { spki_verifying_roundtrip(EcdsaCurve::P521); }

    // ─── 3 R||S encoding tests ───

    #[test]
    fn rs_p256_is_always_64_bytes_over_many_samples() {
        for i in 0..16u8 {
            let kp = generate_keypair(EcdsaCurve::P256);
            let body = [b"sample-p256-", &[i][..]].concat();
            let sig_b64 = sign_xml_canonical(&body, &kp.signing, HashAlg::Sha256).unwrap();
            let raw = B64.decode(&sig_b64).unwrap();
            assert_eq!(raw.len(), 64);
            assert!(raw[..32].iter().any(|&b| b != 0));
            assert!(raw[32..].iter().any(|&b| b != 0));
        }
    }

    #[test]
    fn rs_p384_is_always_96_bytes_over_many_samples() {
        for i in 0..16u8 {
            let kp = generate_keypair(EcdsaCurve::P384);
            let body = [b"sample-p384-", &[i][..]].concat();
            let sig_b64 = sign_xml_canonical(&body, &kp.signing, HashAlg::Sha384).unwrap();
            let raw = B64.decode(&sig_b64).unwrap();
            assert_eq!(raw.len(), 96);
            assert!(raw[..48].iter().any(|&b| b != 0));
            assert!(raw[48..].iter().any(|&b| b != 0));
        }
    }

    #[test]
    fn rs_p521_is_always_132_bytes_over_many_samples() {
        for i in 0..16u8 {
            let kp = generate_keypair(EcdsaCurve::P521);
            let body = [b"sample-p521-", &[i][..]].concat();
            let sig_b64 = sign_xml_canonical(&body, &kp.signing, HashAlg::Sha512).unwrap();
            let raw = B64.decode(&sig_b64).unwrap();
            assert_eq!(raw.len(), 132);
            assert!(raw[..66].iter().any(|&b| b != 0));
            assert!(raw[66..].iter().any(|&b| b != 0));
        }
    }

    // ─── Bonus: descriptors and URN mapping ───

    #[test]
    fn xmldsig_alg_urns_are_well_known() {
        assert_eq!(
            HashAlg::Sha256.xmldsig_alg(),
            "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256"
        );
        assert_eq!(
            HashAlg::Sha384.xmldsig_alg(),
            "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384"
        );
        assert_eq!(
            HashAlg::Sha512.xmldsig_alg(),
            "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha512"
        );
    }

    #[test]
    fn curve_scalar_lengths_match_spec() {
        assert_eq!(EcdsaCurve::P256.scalar_len(), 32);
        assert_eq!(EcdsaCurve::P384.scalar_len(), 48);
        assert_eq!(EcdsaCurve::P521.scalar_len(), 66);
    }
}
