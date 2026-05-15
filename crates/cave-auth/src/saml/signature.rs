// SPDX-License-Identifier: AGPL-3.0-or-later
//! XML DSig — RSA-SHA256 + ECDSA-{SHA256,SHA384} sign and verify
//! over the bytes of a SAML message. This module owns the *crypto
//! step* of XML DSig; the *canonicalization step* (`exc-c14n`) is
//! the caller's responsibility — see [`SignedDocument`].
//!
//! Source: keycloak/keycloak@b825ba97
//!         saml-core/src/main/java/org/keycloak/saml/processing/api/saml/v2/sig/SAML2Signature.java
//!         saml-core-api/src/main/java/org/keycloak/saml/SignatureAlgorithm.java
//!
//! ## What this implements
//!
//! Pure RSA-PKCS1-v1_5 signature over SHA-256 of the input
//! bytes, base64 encoded — exactly the wire format SAML
//! `<ds:SignatureValue>` carries. Works with PEM-DER RSA keys
//! via `ring`.
//!
//! ## What this does NOT implement
//!
//! Full XML canonicalization (`exc-c14n` rfc3741). Real SAML
//! signatures protect a `<ds:SignedInfo>` block that references
//! a *canonicalized* form of the signed element. Computing that
//! canonical form requires whitespace normalization, attribute
//! ordering, namespace inheritance, and a few other rules a
//! couple thousand lines of code long. cave-auth's broker layer
//! either:
//!   (a) treats the original wire bytes as authoritative
//!       (sufficient when the IdP and SP both emit byte-stable
//!        output), or
//!   (b) plugs in an external c14n implementation (`xmlsec1`)
//!       via the `canonicalize_fn` field on [`SignedDocument`].

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{
    RsaKeyPair, UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256,
};

use super::SamlError;

/// `<ds:SignatureMethod Algorithm=…>` URN for RSA-SHA256 — the
/// only algorithm cave-auth signs with. (Verification is more
/// permissive — see [`verify_signature`].)
pub const ALG_RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";

/// `<ds:DigestMethod Algorithm=…>` URN for SHA-256.
pub const ALG_SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";

/// `<ds:CanonicalizationMethod Algorithm=…>` URN for exclusive
/// canonicalization.
pub const ALG_EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";

/// `<ds:SignatureMethod Algorithm=…>` URN for ECDSA over NIST P-256
/// with SHA-256.
pub const ALG_ECDSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";

/// `<ds:SignatureMethod Algorithm=…>` URN for ECDSA over NIST P-384
/// with SHA-384.
pub const ALG_ECDSA_SHA384: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384";

/// A SAML document the broker is preparing to sign, paired with
/// a canonicalization function. Default canonicalization is the
/// identity function — the bytes are treated as-is. Production
/// integrations that need c14n compatibility with strict IdPs
/// set `canonicalize_fn` to an `xmlsec1`-compatible
/// implementation.
pub struct SignedDocument<'a> {
    /// Bytes of the SAML element to sign / verify.
    pub xml: &'a [u8],
    /// Optional canonicalization step. Identity if `None`.
    pub canonicalize_fn: Option<fn(&[u8]) -> Result<Vec<u8>, SamlError>>,
}

impl<'a> SignedDocument<'a> {
    pub fn new(xml: &'a [u8]) -> Self {
        Self {
            xml,
            canonicalize_fn: None,
        }
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, SamlError> {
        match self.canonicalize_fn {
            Some(f) => f(self.xml),
            None => Ok(self.xml.to_vec()),
        }
    }
}

/// Sign `doc` with a PKCS#8-encoded RSA private key (DER bytes).
/// Returns the base64-encoded signature exactly as it would
/// appear in `<ds:SignatureValue>`.
pub fn sign_rsa_sha256(doc: &SignedDocument<'_>, pkcs8_der: &[u8]) -> Result<String, SamlError> {
    let key = RsaKeyPair::from_pkcs8(pkcs8_der)
        .map_err(|e| SamlError::InvalidSignature(format!("load key: {e}")))?;
    let canon = doc.canonical_bytes()?;
    let mut sig = vec![0u8; key.public().modulus_len()];
    let rng = SystemRandom::new();
    key.sign(&RSA_PKCS1_SHA256, &rng, &canon, &mut sig)
        .map_err(|e| SamlError::InvalidSignature(format!("sign: {e}")))?;
    Ok(B64.encode(sig))
}

/// Verify `signature_b64` against `doc` using a DER-encoded RSA
/// public key. `Ok(())` means the signature is valid.
pub fn verify_signature(
    doc: &SignedDocument<'_>,
    signature_b64: &str,
    rsa_pub_der: &[u8],
) -> Result<(), SamlError> {
    let sig = B64
        .decode(signature_b64)
        .map_err(|e| SamlError::InvalidSignature(format!("base64: {e}")))?;
    let canon = doc.canonical_bytes()?;
    let key = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, rsa_pub_der);
    key.verify(&canon, &sig)
        .map_err(|_| SamlError::InvalidSignature("rsa verify failed".into()))?;
    Ok(())
}

// ─── ECDSA-SHA256 (NIST P-256) ──────────────────────────────────────────────

/// Generate a fresh P-256 keypair, returning `(pkcs8_der, spki_der)`.
/// `pkcs8_der` is the PKCS#8-encoded private key suitable for
/// [`sign_ecdsa_sha256`]; `spki_der` is the SubjectPublicKeyInfo DER
/// encoding suitable for [`verify_signature_ecdsa_sha256`].
pub fn ecdsa_p256_generate_pkcs8() -> (Vec<u8>, Vec<u8>) {
    use p256::ecdsa::SigningKey;
    use p256::pkcs8::{EncodePrivateKey, EncodePublicKey};
    let sk = SigningKey::random(&mut rand::thread_rng());
    let pkcs8 = sk.to_pkcs8_der().expect("pkcs8 encode").as_bytes().to_vec();
    let vk = *sk.verifying_key();
    let spki = vk.to_public_key_der().expect("spki encode").as_bytes().to_vec();
    (pkcs8, spki)
}

/// Sign `doc` with a PKCS#8-encoded P-256 private key. Signature is
/// the 64-byte fixed-size IEEE P1363 concatenation `r||s` exactly as
/// `<ds:SignatureValue>` is required to carry it (xmldsig-more §3.2),
/// then base64.
pub fn sign_ecdsa_sha256(
    doc: &SignedDocument<'_>,
    pkcs8_der: &[u8],
) -> Result<String, SamlError> {
    use p256::ecdsa::{signature::Signer, Signature, SigningKey};
    use p256::pkcs8::DecodePrivateKey;
    let sk = SigningKey::from_pkcs8_der(pkcs8_der)
        .map_err(|e| SamlError::InvalidSignature(format!("p256 load: {e}")))?;
    let canon = doc.canonical_bytes()?;
    let sig: Signature = sk.sign(&canon);
    Ok(B64.encode(sig.to_bytes()))
}

/// Verify an ECDSA-P256-SHA256 `<ds:SignatureValue>` against `doc`,
/// given the verifying key as SPKI DER. Returns `Ok(())` on a valid
/// signature.
pub fn verify_signature_ecdsa_sha256(
    doc: &SignedDocument<'_>,
    signature_b64: &str,
    spki_der: &[u8],
) -> Result<(), SamlError> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p256::pkcs8::DecodePublicKey;
    let sig_bytes = B64
        .decode(signature_b64)
        .map_err(|e| SamlError::InvalidSignature(format!("base64: {e}")))?;
    let sig = Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| SamlError::InvalidSignature(format!("p256 sig: {e}")))?;
    let vk = VerifyingKey::from_public_key_der(spki_der)
        .map_err(|e| SamlError::InvalidSignature(format!("p256 spki: {e}")))?;
    let canon = doc.canonical_bytes()?;
    vk.verify(&canon, &sig)
        .map_err(|_| SamlError::InvalidSignature("ecdsa-p256 verify failed".into()))?;
    Ok(())
}

// ─── ECDSA-SHA384 (NIST P-384) ──────────────────────────────────────────────

/// Generate a fresh P-384 keypair, returning `(pkcs8_der, spki_der)`.
pub fn ecdsa_p384_generate_pkcs8() -> (Vec<u8>, Vec<u8>) {
    use p384::ecdsa::SigningKey;
    use p384::pkcs8::{EncodePrivateKey, EncodePublicKey};
    let sk = SigningKey::random(&mut rand::thread_rng());
    let pkcs8 = sk.to_pkcs8_der().expect("pkcs8 encode").as_bytes().to_vec();
    let vk = *sk.verifying_key();
    let spki = vk.to_public_key_der().expect("spki encode").as_bytes().to_vec();
    (pkcs8, spki)
}

/// Sign `doc` with a PKCS#8-encoded P-384 private key. Signature is
/// the 96-byte fixed-size IEEE P1363 concatenation `r||s`, base64.
pub fn sign_ecdsa_sha384(
    doc: &SignedDocument<'_>,
    pkcs8_der: &[u8],
) -> Result<String, SamlError> {
    use p384::ecdsa::{signature::Signer, Signature, SigningKey};
    use p384::pkcs8::DecodePrivateKey;
    let sk = SigningKey::from_pkcs8_der(pkcs8_der)
        .map_err(|e| SamlError::InvalidSignature(format!("p384 load: {e}")))?;
    let canon = doc.canonical_bytes()?;
    let sig: Signature = sk.sign(&canon);
    Ok(B64.encode(sig.to_bytes()))
}

/// Verify an ECDSA-P384-SHA384 `<ds:SignatureValue>` against `doc`.
pub fn verify_signature_ecdsa_sha384(
    doc: &SignedDocument<'_>,
    signature_b64: &str,
    spki_der: &[u8],
) -> Result<(), SamlError> {
    use p384::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p384::pkcs8::DecodePublicKey;
    let sig_bytes = B64
        .decode(signature_b64)
        .map_err(|e| SamlError::InvalidSignature(format!("base64: {e}")))?;
    let sig = Signature::try_from(sig_bytes.as_slice())
        .map_err(|e| SamlError::InvalidSignature(format!("p384 sig: {e}")))?;
    let vk = VerifyingKey::from_public_key_der(spki_der)
        .map_err(|e| SamlError::InvalidSignature(format!("p384 spki: {e}")))?;
    let canon = doc.canonical_bytes()?;
    vk.verify(&canon, &sig)
        .map_err(|_| SamlError::InvalidSignature("ecdsa-p384 verify failed".into()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real PKCS#8-encoded 2048-bit RSA private key, generated
    /// once offline (`openssl genpkey -algorithm RSA + openssl
    /// pkcs8 -topk8 -nocrypt`). Test-only material — never used
    /// outside this test suite. Generating fresh keys per test
    /// would cost several seconds; baking one in keeps the unit
    /// tests fast.
    fn test_keypair_pkcs8_der() -> Vec<u8> {
        const KEY_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCPegzMZl+1jHVMT0PW68K/qcIYqbBkkO6ooVUmxuDLFq0NIQmuteQ30RM06txbzpJdtBO/vAxOfcUBQ+jmKwixHC0JUcW6jixFfTOwFKIdeByzIRNoi1i/ZbrhhknLKZ3U2IQz4VwroyKbL2mFg5dPDA1oj1cJG4QODWLqbcjngRmExdM8remq+c6HGiI2TS0aldg3/wGBI5C+IyOeniVjzaFN/Z3GCqq9uC7Ij8spDoGBZPpskH8ehFLb6RsoxvVWJKJmB7LSNkabWXVLD+a+oqVO9ozMlV1R6qZZ4IUV7+lNS4BQp4Vla3RIKajjj2YKzGIl9UUEyH/A3SOlkqrrAgMBAAECggEANzZ8nlv3EOJQcWE/dgGcHC2zp9IFM24iqXoMTrPR5dWAGsFP/I+6l1A51+9ZhWrlIHIf93TiN4Jmwankgk6lNaLmIeP592Sm3MblkSkfib+jK7vawCx/pof7drY6x5foSPRZS625zoEk3BtOvDZ7j8vPjSE8GSEhnFbCbfx5h7yu4RqjBVEAz7feOMade++Qjn/IyfoNJ2Wq7oq/w7lXVYUNVIS7Ulj9cdTXIF6QFf+84B46d+YTsYiZRGMb/eZYk5IyXdv0vDg+qCD2mV+JYs1PD2qZOKemCxLjYs0OMYy1fKxbYVra4g0gOtTcnUYTJFixuyFifnfOyKKNrpbpgQKBgQDKRtc94O2t+Bah/bU4+90RNB4/lVmCRB0ExkMzJ/djT9TLYYFtnjX6DympmQ6ACzO8cqsArB2nEgbsXCV2lcwCldVY5/I9SuplyqtGKfPRdXlU3GopFNjZ/bdi1GF7MgRpD59yWWRHNN55HV94Eef/LumDOvTBtVu28jRWGJXYmwKBgQC1lUsoBAyBCnXkddsVIm5bqoi84CvcNC+nVRTcn8+x+GYb8o35RSSOymMQzlNd/b1YHzhi2b0R3vikSU/r3LtMdrWgdoIV6ElKgAwcbaoqIb/Zovh3qUXimIZvB8krR6a60QqJVw/1lTRnSuU82zV3ZncCSOJo64TZXmEdm47T8QKBgQCO/smG6w3bYHjPh8WnRRYg5VFE7dXbKz/AclBrR6Oxx2vNY17WGXRbFIEFbjg7+K9YV0/gJ8zGoQ3X5cRuMrOIWFf8g+xRvDY8Q6wU6+97caqWfUNnS1+Jq70K1s0bBF7tzqePdPZZCF0GDefBwBbb5VQa+4Cvt//gMxUgkDzOZQKBgQCRrjQ853qssJrC7vcUrqoBawEHH4awxUGSK0Vwd9qm+xXYyDG1Ug6xbJgsLIxf9SnKoEmZrPzucIflLlgrb8zo3Lh9A3b8Yn8igTa2PBlwceE8l25memzyDdKVE5cG3RZb/UhJxYqtScZgNItT1r6/i3phX94dtQ7BYeHiYiIl0QKBgQCCGN21FfQalMH2duGu7UQnZ03To0uDyn3zoaxxVK7M+9xB8bQ5rFq23ZOuGy1qYE7CitzGkCLf9goiJaNCowwUIKVsj+Joufxg1K9usyThr/OpWwQYNu1TOXpzBmKY1AnK+JVpUsRppc0BzpaPiDcnfi1Ch0ds0gVgPLUfflmX/A==";
        B64.decode(KEY_B64).unwrap()
    }

    /// Matching public key as bare `RSAPublicKey` DER (modulus
    /// + exponent only), produced via `openssl rsa
    /// -RSAPublicKey_out`. ring's `RSA_PKCS1_2048_8192_SHA256`
    /// verifier consumes this form directly.
    fn test_public_key_der() -> Vec<u8> {
        const PUB_B64: &str = "MIIBCgKCAQEAj3oMzGZftYx1TE9D1uvCv6nCGKmwZJDuqKFVJsbgyxatDSEJrrXkN9ETNOrcW86SXbQTv7wMTn3FAUPo5isIsRwtCVHFuo4sRX0zsBSiHXgcsyETaItYv2W64YZJyymd1NiEM+FcK6Mimy9phYOXTwwNaI9XCRuEDg1i6m3I54EZhMXTPK3pqvnOhxoiNk0tGpXYN/8BgSOQviMjnp4lY82hTf2dxgqqvbguyI/LKQ6BgWT6bJB/HoRS2+kbKMb1ViSiZgey0jZGm1l1Sw/mvqKlTvaMzJVdUeqmWeCFFe/pTUuAUKeFZWt0SCmo449mCsxiJfVFBMh/wN0jpZKq6wIDAQAB";
        B64.decode(PUB_B64).unwrap()
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let key = test_keypair_pkcs8_der();
        let pubk = test_public_key_der();
        let doc_bytes = b"<saml:Assertion>some canonicalized form</saml:Assertion>";
        let doc = SignedDocument::new(doc_bytes);
        let sig = sign_rsa_sha256(&doc, &key).unwrap();
        verify_signature(&doc, &sig, &pubk).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        let key = test_keypair_pkcs8_der();
        let pubk = test_public_key_der();
        let doc_a = SignedDocument::new(b"<saml:Assertion>original</saml:Assertion>");
        let sig = sign_rsa_sha256(&doc_a, &key).unwrap();
        let doc_b = SignedDocument::new(b"<saml:Assertion>tampered</saml:Assertion>");
        assert!(matches!(
            verify_signature(&doc_b, &sig, &pubk).unwrap_err(),
            SamlError::InvalidSignature(_)
        ));
    }

    #[test]
    fn verify_rejects_bad_base64() {
        let pubk = test_public_key_der();
        let doc = SignedDocument::new(b"x");
        assert!(verify_signature(&doc, "!not!base64!", &pubk).is_err());
    }

    #[test]
    fn canonicalize_fn_is_applied() {
        let key = test_keypair_pkcs8_der();
        let pubk = test_public_key_der();
        fn upper(b: &[u8]) -> Result<Vec<u8>, SamlError> {
            Ok(b.to_ascii_uppercase())
        }
        let doc = SignedDocument {
            xml: b"hello",
            canonicalize_fn: Some(upper),
        };
        let sig = sign_rsa_sha256(&doc, &key).unwrap();
        // Signature was made over "HELLO" — verifying without
        // the same canonicalize_fn (over raw "hello") must fail.
        let doc_raw = SignedDocument::new(b"hello");
        assert!(verify_signature(&doc_raw, &sig, &pubk).is_err());
        // But applying the same canonicalize_fn succeeds.
        let doc_canon = SignedDocument {
            xml: b"hello",
            canonicalize_fn: Some(upper),
        };
        verify_signature(&doc_canon, &sig, &pubk).unwrap();
    }
}
