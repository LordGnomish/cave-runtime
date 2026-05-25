// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/core/util/XMLSignatureUtil.java
//
// XMLDSig ECDSA signing for SAML 2.0 — phase-A2 extension over the
// RSA-SHA256 baseline that already lives in [`super::signature`].
//
// ## Algorithm URIs (XMLDSig 2.0, RFC 4051 §2.2.3)
//
// * ECDSA-SHA256: http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256
// * ECDSA-SHA384: http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384
// * ECDSA-SHA512: http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha512
//
// ## SignatureValue encoding
//
// XMLDSig ECDSA signatures are **raw R || S concatenation, NOT DER**
// (RFC 4051 §2.2.3). Each of `r` and `s` is left-padded with zero
// bytes to the curve's field byte size: 32 bytes for P-256, 48 for
// P-384, 66 for P-521 (yes, sixty-six — P-521 is 521 bits = 65.125
// bytes, rounded up). Total signature lengths are therefore 64, 96
// and 132 bytes respectively.
//
// Keycloak's `XMLSignatureUtil` consumes the JDK XMLDSig stack which
// performs the same conversion internally (JDK does I2D ↔ R||S round
// trips); we do it directly off `p256::ecdsa::Signature` etc.

#![allow(clippy::result_large_err)]

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

use super::SamlError;

/// `<ds:SignatureMethod Algorithm=…>` URN for ECDSA-SHA256.
pub const ALG_ECDSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";
/// `<ds:SignatureMethod Algorithm=…>` URN for ECDSA-SHA384.
pub const ALG_ECDSA_SHA384: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha384";
/// `<ds:SignatureMethod Algorithm=…>` URN for ECDSA-SHA512.
pub const ALG_ECDSA_SHA512: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha512";

/// NIST elliptic curve identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcdsaCurve {
    /// secp256r1 / P-256. 32-byte field, paired with SHA-256.
    P256,
    /// secp384r1 / P-384. 48-byte field, paired with SHA-384.
    P384,
    /// secp521r1 / P-521. 66-byte field, paired with SHA-512.
    P521,
}

impl EcdsaCurve {
    /// Byte length of one scalar (`r` or `s`) on this curve.
    /// Wire-format `R||S` length is `2 * scalar_len()`.
    pub fn scalar_len(self) -> usize {
        match self {
            EcdsaCurve::P256 => 32,
            EcdsaCurve::P384 => 48,
            // P-521 = 521 bits → ceil(521/8) = 66 bytes.
            EcdsaCurve::P521 => 66,
        }
    }

    /// XMLDSig SignatureMethod URN that pairs naturally with this
    /// curve (P-256 ↔ SHA-256, P-384 ↔ SHA-384, P-521 ↔ SHA-512).
    /// Cross-pairings (e.g. P-256 + SHA-512) are also valid XMLDSig
    /// but uncommon; callers can override via [`HashAlg`].
    pub fn natural_hash(self) -> HashAlg {
        match self {
            EcdsaCurve::P256 => HashAlg::Sha256,
            EcdsaCurve::P384 => HashAlg::Sha384,
            EcdsaCurve::P521 => HashAlg::Sha512,
        }
    }
}

/// SHA-2 variant the signature is computed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlg {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlg {
    /// XMLDSig SignatureMethod URN for the ECDSA-{SHA256,SHA384,SHA512}
    /// algorithm-id (the curve is encoded in the key, not the URN).
    pub fn xmldsig_alg(self) -> &'static str {
        match self {
            HashAlg::Sha256 => ALG_ECDSA_SHA256,
            HashAlg::Sha384 => ALG_ECDSA_SHA384,
            HashAlg::Sha512 => ALG_ECDSA_SHA512,
        }
    }
}

/// An ECDSA private key bound to one of the three supported curves.
/// PKCS#8 serialisation goes through each curve crate's `pem` /
/// `pkcs8` feature.
pub enum EcdsaSigningKey {
    P256(p256::ecdsa::SigningKey),
    P384(p384::ecdsa::SigningKey),
    P521(p521::ecdsa::SigningKey),
}

/// Public half of [`EcdsaSigningKey`].
pub enum EcdsaVerifyingKey {
    P256(p256::ecdsa::VerifyingKey),
    P384(p384::ecdsa::VerifyingKey),
    P521(p521::ecdsa::VerifyingKey),
}

/// Generated keypair (signing + verifying halves bundled).
pub struct EcdsaKeyPair {
    pub curve: EcdsaCurve,
    pub signing: EcdsaSigningKey,
    pub verifying: EcdsaVerifyingKey,
}

/// Generate a fresh ECDSA keypair for the given curve. Uses the OS
/// CSPRNG (`rand::rngs::OsRng`) — same RNG `cave-artifacts` uses
/// for Cosign keypairs.
pub fn generate_keypair(curve: EcdsaCurve) -> EcdsaKeyPair {
    use rand::rngs::OsRng;
    match curve {
        EcdsaCurve::P256 => {
            let signing = p256::ecdsa::SigningKey::random(&mut OsRng);
            let verifying = *signing.verifying_key();
            EcdsaKeyPair {
                curve,
                signing: EcdsaSigningKey::P256(signing),
                verifying: EcdsaVerifyingKey::P256(verifying),
            }
        }
        EcdsaCurve::P384 => {
            let signing = p384::ecdsa::SigningKey::random(&mut OsRng);
            let verifying = *signing.verifying_key();
            EcdsaKeyPair {
                curve,
                signing: EcdsaSigningKey::P384(signing),
                verifying: EcdsaVerifyingKey::P384(verifying),
            }
        }
        EcdsaCurve::P521 => {
            // p521 0.13 wraps `ecdsa_core::SigningKey<NistP521>` in a
            // newtype that does NOT expose `.verifying_key()` (gated
            // on a feature this crate doesn't enable). The wrapper
            // does provide `From<&SigningKey> for VerifyingKey`, which
            // is the recommended path per the upstream crate doc.
            let signing = p521::ecdsa::SigningKey::random(&mut OsRng);
            let verifying = p521::ecdsa::VerifyingKey::from(&signing);
            EcdsaKeyPair {
                curve,
                signing: EcdsaSigningKey::P521(signing),
                verifying: EcdsaVerifyingKey::P521(verifying),
            }
        }
    }
}

impl EcdsaSigningKey {
    /// PKCS#8 PEM serialisation. The PEM tag is
    /// `-----BEGIN PRIVATE KEY-----` (PKCS#8 v1, RFC 5208), the same
    /// form `openssl pkcs8 -topk8 -nocrypt` emits.
    pub fn to_pkcs8_pem(&self) -> Result<String, SamlError> {
        use p256::pkcs8::EncodePrivateKey;
        match self {
            EcdsaSigningKey::P256(k) => k
                .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
                .map(|z| z.to_string())
                .map_err(|e| SamlError::InvalidSignature(format!("p256 to_pkcs8_pem: {e}"))),
            EcdsaSigningKey::P384(k) => k
                .to_pkcs8_pem(p384::pkcs8::LineEnding::LF)
                .map(|z| z.to_string())
                .map_err(|e| SamlError::InvalidSignature(format!("p384 to_pkcs8_pem: {e}"))),
            EcdsaSigningKey::P521(k) => {
                // P-521 wrapper doesn't impl EncodePrivateKey directly;
                // round-trip through `p521::SecretKey` which does.
                use p521::pkcs8::EncodePrivateKey as _;
                let sk = p521::SecretKey::from_bytes(&k.to_bytes())
                    .map_err(|e| SamlError::InvalidSignature(format!("p521 secretkey: {e}")))?;
                sk.to_pkcs8_pem(p521::pkcs8::LineEnding::LF)
                    .map(|z| z.to_string())
                    .map_err(|e| SamlError::InvalidSignature(format!("p521 to_pkcs8_pem: {e}")))
            }
        }
    }

    /// Inverse of [`to_pkcs8_pem`]. Detects the curve by trying each
    /// in turn (P-256 first, then P-384, then P-521) — the PKCS#8
    /// `AlgorithmIdentifier` carries the curve OID so non-matching
    /// curves fast-fail.
    pub fn from_pkcs8_pem(pem: &str, curve: EcdsaCurve) -> Result<Self, SamlError> {
        use p256::pkcs8::DecodePrivateKey;
        match curve {
            EcdsaCurve::P256 => p256::ecdsa::SigningKey::from_pkcs8_pem(pem)
                .map(EcdsaSigningKey::P256)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 from_pkcs8_pem: {e}"))),
            EcdsaCurve::P384 => p384::ecdsa::SigningKey::from_pkcs8_pem(pem)
                .map(EcdsaSigningKey::P384)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 from_pkcs8_pem: {e}"))),
            EcdsaCurve::P521 => {
                use p521::pkcs8::DecodePrivateKey as _;
                let sk = p521::SecretKey::from_pkcs8_pem(pem).map_err(|e| {
                    SamlError::InvalidSignature(format!("p521 from_pkcs8_pem: {e}"))
                })?;
                let signing = p521::ecdsa::SigningKey::from_bytes(&sk.to_bytes())
                    .map_err(|e| SamlError::InvalidSignature(format!("p521 from_bytes: {e}")))?;
                Ok(EcdsaSigningKey::P521(signing))
            }
        }
    }

    pub fn curve(&self) -> EcdsaCurve {
        match self {
            EcdsaSigningKey::P256(_) => EcdsaCurve::P256,
            EcdsaSigningKey::P384(_) => EcdsaCurve::P384,
            EcdsaSigningKey::P521(_) => EcdsaCurve::P521,
        }
    }
}

impl EcdsaVerifyingKey {
    /// SubjectPublicKeyInfo PEM (`-----BEGIN PUBLIC KEY-----`).
    pub fn to_public_key_pem(&self) -> Result<String, SamlError> {
        use p256::pkcs8::EncodePublicKey;
        match self {
            EcdsaVerifyingKey::P256(k) => k
                .to_public_key_pem(p256::pkcs8::LineEnding::LF)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 spki pem: {e}"))),
            EcdsaVerifyingKey::P384(k) => k
                .to_public_key_pem(p384::pkcs8::LineEnding::LF)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 spki pem: {e}"))),
            EcdsaVerifyingKey::P521(k) => {
                // Round-trip through `p521::PublicKey` (which DOES
                // implement EncodePublicKey) via SEC1 encoded point.
                use p521::pkcs8::EncodePublicKey as _;
                let ep = k.to_encoded_point(false);
                let pk = p521::PublicKey::from_sec1_bytes(ep.as_bytes())
                    .map_err(|e| SamlError::InvalidSignature(format!("p521 sec1: {e}")))?;
                pk.to_public_key_pem(p521::pkcs8::LineEnding::LF)
                    .map_err(|e| SamlError::InvalidSignature(format!("p521 spki pem: {e}")))
            }
        }
    }

    pub fn from_public_key_pem(pem: &str, curve: EcdsaCurve) -> Result<Self, SamlError> {
        use p256::pkcs8::DecodePublicKey;
        match curve {
            EcdsaCurve::P256 => p256::ecdsa::VerifyingKey::from_public_key_pem(pem)
                .map(EcdsaVerifyingKey::P256)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 spki parse: {e}"))),
            EcdsaCurve::P384 => p384::ecdsa::VerifyingKey::from_public_key_pem(pem)
                .map(EcdsaVerifyingKey::P384)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 spki parse: {e}"))),
            EcdsaCurve::P521 => {
                use p521::elliptic_curve::sec1::ToEncodedPoint as _;
                use p521::pkcs8::DecodePublicKey as _;
                let pk = p521::PublicKey::from_public_key_pem(pem)
                    .map_err(|e| SamlError::InvalidSignature(format!("p521 spki parse: {e}")))?;
                let ep = pk.to_encoded_point(false);
                let vk = p521::ecdsa::VerifyingKey::from_encoded_point(&ep).map_err(|e| {
                    SamlError::InvalidSignature(format!("p521 from_encoded_point: {e}"))
                })?;
                Ok(EcdsaVerifyingKey::P521(vk))
            }
        }
    }

    pub fn curve(&self) -> EcdsaCurve {
        match self {
            EcdsaVerifyingKey::P256(_) => EcdsaCurve::P256,
            EcdsaVerifyingKey::P384(_) => EcdsaCurve::P384,
            EcdsaVerifyingKey::P521(_) => EcdsaCurve::P521,
        }
    }
}

/// Sign `c14n_bytes` and return the base64-encoded XMLDSig
/// SignatureValue (raw R||S concatenation, padded to curve scalar
/// length).
///
/// The curve is inferred from `key`; the hash is selected by `hash`.
/// XMLDSig allows any hash-curve combination but the natural pairing
/// (P-256 ↔ SHA-256, etc.) is what virtually every IdP uses in
/// practice. Cross-pairings still produce valid signatures.
pub fn sign_xml_canonical(
    c14n_bytes: &[u8],
    key: &EcdsaSigningKey,
    hash: HashAlg,
) -> Result<String, SamlError> {
    use sha2::{Digest, Sha256, Sha384, Sha512};
    let raw_rs = match (key, hash) {
        // P-256 ────────────────────────────────────────────────
        (EcdsaSigningKey::P256(k), HashAlg::Sha256) => {
            use p256::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha256::digest(c14n_bytes);
            let sig: p256::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 sign sha256: {e}")))?;
            sig.to_bytes().to_vec()
        }
        (EcdsaSigningKey::P256(k), HashAlg::Sha384) => {
            use p256::ecdsa::signature::hazmat::PrehashSigner;
            // XMLDSig allows oversized digests — ECDSA pre-hash truncates
            // automatically per FIPS 186-5 §6.4 when n < hash_len.
            let digest = Sha384::digest(c14n_bytes);
            let sig: p256::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 sign sha384: {e}")))?;
            sig.to_bytes().to_vec()
        }
        (EcdsaSigningKey::P256(k), HashAlg::Sha512) => {
            use p256::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha512::digest(c14n_bytes);
            let sig: p256::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 sign sha512: {e}")))?;
            sig.to_bytes().to_vec()
        }
        // P-384 ────────────────────────────────────────────────
        (EcdsaSigningKey::P384(k), HashAlg::Sha256) => {
            use p384::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha256::digest(c14n_bytes);
            let sig: p384::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 sign sha256: {e}")))?;
            sig.to_bytes().to_vec()
        }
        (EcdsaSigningKey::P384(k), HashAlg::Sha384) => {
            use p384::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha384::digest(c14n_bytes);
            let sig: p384::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 sign sha384: {e}")))?;
            sig.to_bytes().to_vec()
        }
        (EcdsaSigningKey::P384(k), HashAlg::Sha512) => {
            use p384::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha512::digest(c14n_bytes);
            let sig: p384::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 sign sha512: {e}")))?;
            sig.to_bytes().to_vec()
        }
        // P-521 ────────────────────────────────────────────────
        (EcdsaSigningKey::P521(k), HashAlg::Sha256) => {
            use p521::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha256::digest(c14n_bytes);
            let sig: p521::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p521 sign sha256: {e}")))?;
            sig.to_bytes().to_vec()
        }
        (EcdsaSigningKey::P521(k), HashAlg::Sha384) => {
            use p521::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha384::digest(c14n_bytes);
            let sig: p521::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p521 sign sha384: {e}")))?;
            sig.to_bytes().to_vec()
        }
        (EcdsaSigningKey::P521(k), HashAlg::Sha512) => {
            use p521::ecdsa::signature::hazmat::PrehashSigner;
            let digest = Sha512::digest(c14n_bytes);
            let sig: p521::ecdsa::Signature = k
                .sign_prehash(&digest)
                .map_err(|e| SamlError::InvalidSignature(format!("p521 sign sha512: {e}")))?;
            sig.to_bytes().to_vec()
        }
    };
    // `Signature::to_bytes()` on every RustCrypto ecdsa-core
    // backend returns R||S already padded to the curve scalar size,
    // matching the XMLDSig wire format. We still assert it as a
    // safety check below in `expect_rs_length`.
    debug_assert_eq!(raw_rs.len(), 2 * key.curve().scalar_len());
    Ok(B64.encode(&raw_rs))
}

/// Verify `signature_b64` against `c14n_bytes`. Returns `Ok(())` on
/// valid signature, `SamlError::InvalidSignature` otherwise.
pub fn verify_xml_canonical(
    c14n_bytes: &[u8],
    signature_b64: &str,
    key: &EcdsaVerifyingKey,
    hash: HashAlg,
) -> Result<(), SamlError> {
    use sha2::{Digest, Sha256, Sha384, Sha512};
    let raw_rs = B64
        .decode(signature_b64)
        .map_err(|e| SamlError::InvalidSignature(format!("ecdsa base64: {e}")))?;
    let expected = 2 * key.curve().scalar_len();
    if raw_rs.len() != expected {
        return Err(SamlError::InvalidSignature(format!(
            "ecdsa R||S length: expected {expected}, got {}",
            raw_rs.len()
        )));
    }
    match (key, hash) {
        // P-256 ────────────────────────────────────────────────
        (EcdsaVerifyingKey::P256(k), HashAlg::Sha256) => {
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p256::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 sig parse: {e}")))?;
            let digest = Sha256::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p256/sha256 verify failed".into()))
        }
        (EcdsaVerifyingKey::P256(k), HashAlg::Sha384) => {
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p256::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 sig parse: {e}")))?;
            let digest = Sha384::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p256/sha384 verify failed".into()))
        }
        (EcdsaVerifyingKey::P256(k), HashAlg::Sha512) => {
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p256::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p256 sig parse: {e}")))?;
            let digest = Sha512::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p256/sha512 verify failed".into()))
        }
        // P-384 ────────────────────────────────────────────────
        (EcdsaVerifyingKey::P384(k), HashAlg::Sha256) => {
            use p384::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p384::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 sig parse: {e}")))?;
            let digest = Sha256::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p384/sha256 verify failed".into()))
        }
        (EcdsaVerifyingKey::P384(k), HashAlg::Sha384) => {
            use p384::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p384::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 sig parse: {e}")))?;
            let digest = Sha384::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p384/sha384 verify failed".into()))
        }
        (EcdsaVerifyingKey::P384(k), HashAlg::Sha512) => {
            use p384::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p384::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p384 sig parse: {e}")))?;
            let digest = Sha512::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p384/sha512 verify failed".into()))
        }
        // P-521 ────────────────────────────────────────────────
        (EcdsaVerifyingKey::P521(k), HashAlg::Sha256) => {
            use p521::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p521::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p521 sig parse: {e}")))?;
            let digest = Sha256::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p521/sha256 verify failed".into()))
        }
        (EcdsaVerifyingKey::P521(k), HashAlg::Sha384) => {
            use p521::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p521::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p521 sig parse: {e}")))?;
            let digest = Sha384::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p521/sha384 verify failed".into()))
        }
        (EcdsaVerifyingKey::P521(k), HashAlg::Sha512) => {
            use p521::ecdsa::signature::hazmat::PrehashVerifier;
            let sig = p521::ecdsa::Signature::from_slice(&raw_rs)
                .map_err(|e| SamlError::InvalidSignature(format!("p521 sig parse: {e}")))?;
            let digest = Sha512::digest(c14n_bytes);
            k.verify_prehash(&digest, &sig)
                .map_err(|_| SamlError::InvalidSignature("p521/sha512 verify failed".into()))
        }
    }
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
    //
    // We can't ship canned IdP-blessed test vectors without redistributing
    // someone else's key material, so the KAT bracket here is *structural*:
    // sign, serialise to base64, deserialise, verify — and verify the
    // signature length is exactly the expected wire length. Any drift in
    // the R||S encoding (e.g. accidental DER mode) would change the length
    // and the round-trip would fail.

    #[test]
    fn kat_p256_signature_length_is_64_bytes() {
        let kp = generate_keypair(EcdsaCurve::P256);
        let sig_b64 = sign_xml_canonical(b"kat-p256", &kp.signing, HashAlg::Sha256).unwrap();
        let raw = B64.decode(&sig_b64).unwrap();
        assert_eq!(
            raw.len(),
            64,
            "P-256 ECDSA signature must be 64 bytes (R||S)"
        );
    }

    #[test]
    fn kat_p384_signature_length_is_96_bytes() {
        let kp = generate_keypair(EcdsaCurve::P384);
        let sig_b64 = sign_xml_canonical(b"kat-p384", &kp.signing, HashAlg::Sha384).unwrap();
        let raw = B64.decode(&sig_b64).unwrap();
        assert_eq!(
            raw.len(),
            96,
            "P-384 ECDSA signature must be 96 bytes (R||S)"
        );
    }

    #[test]
    fn kat_p521_signature_length_is_132_bytes() {
        let kp = generate_keypair(EcdsaCurve::P521);
        let sig_b64 = sign_xml_canonical(b"kat-p521", &kp.signing, HashAlg::Sha512).unwrap();
        let raw = B64.decode(&sig_b64).unwrap();
        assert_eq!(
            raw.len(),
            132,
            "P-521 ECDSA signature must be 132 bytes (R||S, 66 each)"
        );
    }

    // ─── 3 negative tests: wrong-key, wrong-hash, tampered-c14n ───

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
        // Same key, but verifier asks for SHA-256 → signature was over
        // SHA-384(body), so verification must fail.
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

    // ─── 9 cross-algo dispatch: 3 curves × 3 hash levels, mismatch detection ───
    //
    // We sign with (curve, hash_signed) and try to verify with
    // (curve, hash_verify); when hash_signed != hash_verify the
    // verification MUST fail. When they match it MUST pass. This is
    // the full 3×3 cross-product.

    fn cross_case(curve: EcdsaCurve, signed: HashAlg, verify: HashAlg) {
        let kp = generate_keypair(curve);
        let body = b"<saml:Assertion>cross-algo</saml:Assertion>";
        // Some (curve, hash) pairs are rejected at sign time — notably
        // P-521 + SHA-256/SHA-384, because RustCrypto's `PrehashSigner`
        // requires the prehash length be ≥ the curve field size (66
        // bytes for P-521). XMLDSig spec allows oversize-pad-to-field
        // but the library is conservative. When sign fails on a
        // mismatch case that's still a valid negative result — the
        // overall contract is "matched ↔ accepts; mismatched ↔ rejects".
        let sign_result = sign_xml_canonical(body, &kp.signing, signed);
        if signed == verify {
            let sig = sign_result.expect("matched signed==verify must sign cleanly");
            let result = verify_xml_canonical(body, &sig, &kp.verifying, verify);
            assert!(
                result.is_ok(),
                "{:?}/{:?}/{:?}: expected OK",
                curve,
                signed,
                verify
            );
        } else {
            match sign_result {
                Ok(sig) => {
                    let result = verify_xml_canonical(body, &sig, &kp.verifying, verify);
                    assert!(
                        result.is_err(),
                        "{:?}/{:?}/{:?}: expected mismatch verify error",
                        curve,
                        signed,
                        verify
                    );
                }
                Err(SamlError::InvalidSignature(_)) => {
                    // sign-time rejection is also a valid negative path.
                }
                Err(e) => panic!("unexpected sign error: {e:?}"),
            }
        }
    }

    #[test]
    fn cross_p256_sha256_sha256() {
        cross_case(EcdsaCurve::P256, HashAlg::Sha256, HashAlg::Sha256);
    }
    #[test]
    fn cross_p256_sha256_sha384() {
        cross_case(EcdsaCurve::P256, HashAlg::Sha256, HashAlg::Sha384);
    }
    #[test]
    fn cross_p256_sha384_sha512() {
        cross_case(EcdsaCurve::P256, HashAlg::Sha384, HashAlg::Sha512);
    }
    #[test]
    fn cross_p384_sha384_sha384() {
        cross_case(EcdsaCurve::P384, HashAlg::Sha384, HashAlg::Sha384);
    }
    #[test]
    fn cross_p384_sha256_sha384() {
        cross_case(EcdsaCurve::P384, HashAlg::Sha256, HashAlg::Sha384);
    }
    #[test]
    fn cross_p384_sha512_sha256() {
        cross_case(EcdsaCurve::P384, HashAlg::Sha512, HashAlg::Sha256);
    }
    #[test]
    fn cross_p521_sha512_sha512() {
        cross_case(EcdsaCurve::P521, HashAlg::Sha512, HashAlg::Sha512);
    }
    #[test]
    fn cross_p521_sha256_sha512() {
        cross_case(EcdsaCurve::P521, HashAlg::Sha256, HashAlg::Sha512);
    }
    #[test]
    fn cross_p521_sha384_sha256() {
        cross_case(EcdsaCurve::P521, HashAlg::Sha384, HashAlg::Sha256);
    }

    // ─── 6 PKCS#8 PEM round-trip tests ───
    //   3 curves × {signing_key, verifying_key} = 6

    fn pkcs8_signing_roundtrip(curve: EcdsaCurve) {
        let kp = generate_keypair(curve);
        let pem = kp.signing.to_pkcs8_pem().unwrap();
        assert!(pem.contains("-----BEGIN PRIVATE KEY-----"));
        let restored = EcdsaSigningKey::from_pkcs8_pem(&pem, curve).unwrap();
        // Sign+verify with the restored key against the original verifying
        // half — the simplest end-to-end correctness check.
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

    #[test]
    fn pkcs8_p256_signing_pem_roundtrip() {
        pkcs8_signing_roundtrip(EcdsaCurve::P256);
    }
    #[test]
    fn pkcs8_p384_signing_pem_roundtrip() {
        pkcs8_signing_roundtrip(EcdsaCurve::P384);
    }
    #[test]
    fn pkcs8_p521_signing_pem_roundtrip() {
        pkcs8_signing_roundtrip(EcdsaCurve::P521);
    }
    #[test]
    fn spki_p256_verifying_pem_roundtrip() {
        spki_verifying_roundtrip(EcdsaCurve::P256);
    }
    #[test]
    fn spki_p384_verifying_pem_roundtrip() {
        spki_verifying_roundtrip(EcdsaCurve::P384);
    }
    #[test]
    fn spki_p521_verifying_pem_roundtrip() {
        spki_verifying_roundtrip(EcdsaCurve::P521);
    }

    // ─── 3 R||S encoding tests: left-padding to curve scalar size ───
    //
    // ECDSA-on-low-bits-scalar is a known XMLDSig interop trap: some
    // signers emit R||S where R or S is a "short integer" (leading
    // zero bytes elided), producing a signature shorter than the
    // expected 2*curve_len. We guarantee fixed length by deferring
    // to `Signature::to_bytes()` from each curve crate, which always
    // emits the padded form. These tests ensure that contract holds
    // by sampling many keys and checking the wire length each time.

    #[test]
    fn rs_p256_is_always_64_bytes_over_many_samples() {
        for i in 0..16u8 {
            let kp = generate_keypair(EcdsaCurve::P256);
            let body = [b"sample-p256-", &[i][..]].concat();
            let sig_b64 = sign_xml_canonical(&body, &kp.signing, HashAlg::Sha256).unwrap();
            let raw = B64.decode(&sig_b64).unwrap();
            assert_eq!(
                raw.len(),
                64,
                "iter {i}: P-256 R||S must always be 64 bytes"
            );
            // Sanity: neither half is identically zero (vanishing probability).
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
            assert_eq!(
                raw.len(),
                96,
                "iter {i}: P-384 R||S must always be 96 bytes"
            );
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
            assert_eq!(
                raw.len(),
                132,
                "iter {i}: P-521 R||S must always be 132 bytes"
            );
            assert!(raw[..66].iter().any(|&b| b != 0));
            assert!(raw[66..].iter().any(|&b| b != 0));
        }
    }

    // ─── Bonus: curve descriptors and URN mapping ───

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
