// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// COSE_Key (RFC 8152 §7) — parse + signature verification.
//
// COSE_Key parameter labels (RFC 8152 §7.1, §13.1.1, §13.2):
//   kty  : 1    ( EC2=2, RSA=3, OKP=1 )
//   alg  : 3    ( ES256=-7, EdDSA=-8, RS256=-257 )
//   EC2 only:
//     crv :-1   ( P-256=1 )
//     x   :-2
//     y   :-3
//   RSA only:
//     n   :-1
//     e   :-2
//   OKP (Ed25519) only:
//     crv :-1   ( Ed25519=6 )
//     x   :-2
//
// This is a focused port of webauthn4j's `CredentialPublicKey` hierarchy —
// `EC2CredentialPublicKey`, `RSACredentialPublicKey`, `EdDSACredentialPublicKey`.

use ciborium::value::Value;

use super::cbor;
use super::{CoseAlg, WebAuthnError};

/// Parsed COSE public key — discriminant by algorithm.
#[derive(Debug, Clone)]
pub enum CoseKey {
    Es256 { x: [u8; 32], y: [u8; 32] },
    EdDsa { x: [u8; 32] },
    Rs256 { n: Vec<u8>, e: Vec<u8> },
}

impl CoseKey {
    pub fn algorithm(&self) -> CoseAlg {
        match self {
            Self::Es256 { .. } => CoseAlg::Es256,
            Self::EdDsa { .. } => CoseAlg::EdDsa,
            Self::Rs256 { .. } => CoseAlg::Rs256,
        }
    }
}

/// Parse a COSE_Key from CBOR bytes.
///
/// Port of webauthn4j `CredentialPublicKeyConverter`.
pub fn parse(raw: &[u8]) -> Result<CoseKey, WebAuthnError> {
    let v = cbor::decode(raw)?;
    let kty = cbor::map_get_int(&v, 1)
        .ok_or_else(|| WebAuthnError::Cose("missing kty(1)".into()))?;
    let kty = cbor::as_i64(kty)?;
    let alg_raw = cbor::map_get_int(&v, 3)
        .ok_or_else(|| WebAuthnError::Cose("missing alg(3)".into()))?;
    let alg = cbor::as_i64(alg_raw)?;
    let alg = CoseAlg::from_i64(alg).ok_or(WebAuthnError::UnsupportedAlgorithm(alg))?;

    match (kty, alg) {
        // EC2.
        (2, CoseAlg::Es256) => {
            let crv = cbor::map_get_int(&v, -1)
                .ok_or_else(|| WebAuthnError::Cose("EC2 missing crv".into()))?;
            if cbor::as_i64(crv)? != 1 {
                return Err(WebAuthnError::Cose("EC2 crv != P-256".into()));
            }
            let x = cbor::map_get_int(&v, -2)
                .ok_or_else(|| WebAuthnError::Cose("EC2 missing x".into()))?;
            let y = cbor::map_get_int(&v, -3)
                .ok_or_else(|| WebAuthnError::Cose("EC2 missing y".into()))?;
            let xb = to_array32(cbor::as_bytes(x)?)?;
            let yb = to_array32(cbor::as_bytes(y)?)?;
            Ok(CoseKey::Es256 { x: xb, y: yb })
        }
        // OKP / Ed25519.
        (1, CoseAlg::EdDsa) => {
            let crv = cbor::map_get_int(&v, -1)
                .ok_or_else(|| WebAuthnError::Cose("OKP missing crv".into()))?;
            if cbor::as_i64(crv)? != 6 {
                return Err(WebAuthnError::Cose("OKP crv != Ed25519".into()));
            }
            let x = cbor::map_get_int(&v, -2)
                .ok_or_else(|| WebAuthnError::Cose("OKP missing x".into()))?;
            let xb = to_array32(cbor::as_bytes(x)?)?;
            Ok(CoseKey::EdDsa { x: xb })
        }
        // RSA.
        (3, CoseAlg::Rs256) => {
            let n = cbor::map_get_int(&v, -1)
                .ok_or_else(|| WebAuthnError::Cose("RSA missing n".into()))?;
            let e = cbor::map_get_int(&v, -2)
                .ok_or_else(|| WebAuthnError::Cose("RSA missing e".into()))?;
            Ok(CoseKey::Rs256 {
                n: cbor::as_bytes(n)?.to_vec(),
                e: cbor::as_bytes(e)?.to_vec(),
            })
        }
        (k, a) => Err(WebAuthnError::Cose(format!(
            "unsupported kty/alg combination: kty={k}, alg={a:?}"
        ))),
    }
}

/// Encode a CoseKey back to CBOR bytes (used by tests + credential storage).
pub fn encode(key: &CoseKey) -> Result<Vec<u8>, WebAuthnError> {
    let v = match key {
        CoseKey::Es256 { x, y } => Value::Map(vec![
            (Value::Integer(1i64.into()), Value::Integer(2i64.into())), // kty=EC2
            (Value::Integer(3i64.into()), Value::Integer((-7i64).into())), // alg=ES256
            (Value::Integer((-1i64).into()), Value::Integer(1i64.into())), // crv=P-256
            (Value::Integer((-2i64).into()), Value::Bytes(x.to_vec())),
            (Value::Integer((-3i64).into()), Value::Bytes(y.to_vec())),
        ]),
        CoseKey::EdDsa { x } => Value::Map(vec![
            (Value::Integer(1i64.into()), Value::Integer(1i64.into())), // kty=OKP
            (Value::Integer(3i64.into()), Value::Integer((-8i64).into())), // alg=EdDSA
            (Value::Integer((-1i64).into()), Value::Integer(6i64.into())), // crv=Ed25519
            (Value::Integer((-2i64).into()), Value::Bytes(x.to_vec())),
        ]),
        CoseKey::Rs256 { n, e } => Value::Map(vec![
            (Value::Integer(1i64.into()), Value::Integer(3i64.into())), // kty=RSA
            (Value::Integer(3i64.into()), Value::Integer((-257i64).into())), // alg=RS256
            (Value::Integer((-1i64).into()), Value::Bytes(n.clone())),
            (Value::Integer((-2i64).into()), Value::Bytes(e.clone())),
        ]),
    };
    cbor::encode(&v)
}

/// Verify a signature over `data` using the COSE_Key.
///
/// `data` is the raw bytes to hash; concrete hash function is determined by
/// `key.algorithm()`. Returns Ok(()) on success, Err with reason on failure.
pub fn verify(key: &CoseKey, data: &[u8], signature: &[u8]) -> Result<(), WebAuthnError> {
    match key {
        CoseKey::Es256 { x, y } => verify_es256(x, y, data, signature),
        CoseKey::EdDsa { x } => verify_eddsa(x, data, signature),
        CoseKey::Rs256 { n, e } => verify_rs256(n, e, data, signature),
    }
}

fn to_array32(b: &[u8]) -> Result<[u8; 32], WebAuthnError> {
    if b.len() != 32 {
        return Err(WebAuthnError::Cose(format!(
            "expected 32-byte coord, got {}",
            b.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(b);
    Ok(out)
}

fn verify_es256(x: &[u8; 32], y: &[u8; 32], data: &[u8], sig: &[u8]) -> Result<(), WebAuthnError> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p256::elliptic_curve::sec1::EncodedPoint;
    use p256::NistP256;

    let point = EncodedPoint::<NistP256>::from_affine_coordinates(x.into(), y.into(), false);
    let vk = VerifyingKey::from_encoded_point(&point)
        .map_err(|e| WebAuthnError::Signature(format!("ES256 vk: {e}")))?;
    let sig = Signature::from_der(sig)
        .map_err(|e| WebAuthnError::Signature(format!("ES256 sig DER: {e}")))?;
    vk.verify(data, &sig)
        .map_err(|e| WebAuthnError::Signature(format!("ES256 verify: {e}")))
}

fn verify_eddsa(x: &[u8; 32], data: &[u8], sig: &[u8]) -> Result<(), WebAuthnError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let vk = VerifyingKey::from_bytes(x)
        .map_err(|e| WebAuthnError::Signature(format!("EdDSA vk: {e}")))?;
    let sig_arr: [u8; 64] = sig
        .try_into()
        .map_err(|_| WebAuthnError::Signature("EdDSA sig wrong length".into()))?;
    let sig = Signature::from_bytes(&sig_arr);
    vk.verify(data, &sig)
        .map_err(|e| WebAuthnError::Signature(format!("EdDSA verify: {e}")))
}

fn verify_rs256(n: &[u8], e: &[u8], data: &[u8], sig: &[u8]) -> Result<(), WebAuthnError> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::signature::Verifier;
    use rsa::BigUint;
    use rsa::RsaPublicKey;
    use sha2::Sha256;

    let n_bn = BigUint::from_bytes_be(n);
    let e_bn = BigUint::from_bytes_be(e);
    let pk = RsaPublicKey::new(n_bn, e_bn)
        .map_err(|e| WebAuthnError::Signature(format!("RS256 pk: {e}")))?;
    let vk: VerifyingKey<Sha256> = VerifyingKey::new(pk);
    let sig = Signature::try_from(sig)
        .map_err(|e| WebAuthnError::Signature(format!("RS256 sig: {e}")))?;
    vk.verify(data, &sig)
        .map_err(|e| WebAuthnError::Signature(format!("RS256 verify: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Signer as _, SigningKey};
    use rand::rngs::OsRng;

    #[test]
    fn es256_roundtrip_encode_parse() {
        let sk = SigningKey::random(&mut OsRng);
        let vk = sk.verifying_key();
        let pt = vk.to_encoded_point(false);
        let x: [u8; 32] = (*pt.x().unwrap()).into();
        let y: [u8; 32] = (*pt.y().unwrap()).into();
        let original = CoseKey::Es256 { x, y };
        let bytes = encode(&original).unwrap();
        let parsed = parse(&bytes).unwrap();
        match parsed {
            CoseKey::Es256 { x: x2, y: y2 } => {
                assert_eq!(x, x2);
                assert_eq!(y, y2);
            }
            _ => panic!("expected ES256"),
        }
    }

    #[test]
    fn es256_sign_then_verify_succeeds() {
        let sk = SigningKey::random(&mut OsRng);
        let vk = sk.verifying_key();
        let pt = vk.to_encoded_point(false);
        let x: [u8; 32] = (*pt.x().unwrap()).into();
        let y: [u8; 32] = (*pt.y().unwrap()).into();
        let key = CoseKey::Es256 { x, y };
        let msg = b"hello cave-auth webauthn";
        let sig: p256::ecdsa::Signature = sk.sign(msg);
        let der = sig.to_der();
        verify(&key, msg, der.as_bytes()).unwrap();
    }

    #[test]
    fn es256_verify_rejects_tampered_msg() {
        let sk = SigningKey::random(&mut OsRng);
        let vk = sk.verifying_key();
        let pt = vk.to_encoded_point(false);
        let x: [u8; 32] = (*pt.x().unwrap()).into();
        let y: [u8; 32] = (*pt.y().unwrap()).into();
        let key = CoseKey::Es256 { x, y };
        let sig: p256::ecdsa::Signature = sk.sign(b"original");
        let der = sig.to_der();
        assert!(verify(&key, b"tampered", der.as_bytes()).is_err());
    }

    #[test]
    fn eddsa_sign_then_verify_succeeds() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut csprng = OsRng;
        let sk = SigningKey::generate(&mut csprng);
        let vk = sk.verifying_key();
        let key = CoseKey::EdDsa { x: vk.to_bytes() };
        let msg = b"hello eddsa webauthn";
        let sig = sk.sign(msg);
        verify(&key, msg, &sig.to_bytes()).unwrap();
    }

    #[test]
    fn eddsa_verify_rejects_tampered() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut csprng = OsRng;
        let sk = SigningKey::generate(&mut csprng);
        let vk = sk.verifying_key();
        let key = CoseKey::EdDsa { x: vk.to_bytes() };
        let sig = sk.sign(b"a");
        assert!(verify(&key, b"b", &sig.to_bytes()).is_err());
    }

    #[test]
    fn rs256_sign_then_verify_succeeds() {
        use rsa::pkcs1v15::SigningKey;
        use rsa::signature::SignatureEncoding as _;
        use rsa::signature::Signer as _;
        use rsa::RsaPrivateKey;
        use sha2::Sha256;
        let mut rng = OsRng;
        // 2048 RSA keygen is slow — use 1024 only for tests. The verify path
        // doesn't care about key size, but the BigUint round-trip does.
        let sk_inner = RsaPrivateKey::new(&mut rng, 1024).expect("rsa keygen");
        let pk = sk_inner.to_public_key();
        let sk: SigningKey<Sha256> = SigningKey::new(sk_inner);
        let msg = b"hello rs256 webauthn";
        let sig = sk.sign(msg);
        use rsa::traits::PublicKeyParts;
        let key = CoseKey::Rs256 {
            n: pk.n().to_bytes_be(),
            e: pk.e().to_bytes_be(),
        };
        verify(&key, msg, &sig.to_bytes()).unwrap();
    }

    #[test]
    fn parse_unsupported_alg_returns_error() {
        let v = Value::Map(vec![
            (Value::Integer(1i64.into()), Value::Integer(2i64.into())),
            (Value::Integer(3i64.into()), Value::Integer((-99i64).into())),
        ]);
        let bytes = cbor::encode(&v).unwrap();
        assert!(matches!(
            parse(&bytes),
            Err(WebAuthnError::UnsupportedAlgorithm(-99))
        ));
    }

    #[test]
    fn parse_missing_kty_returns_error() {
        let v = Value::Map(vec![]);
        let bytes = cbor::encode(&v).unwrap();
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn algorithm_accessor_matches_variant() {
        let k = CoseKey::Es256 {
            x: [0; 32],
            y: [0; 32],
        };
        assert_eq!(k.algorithm(), CoseAlg::Es256);
        let k = CoseKey::EdDsa { x: [0; 32] };
        assert_eq!(k.algorithm(), CoseAlg::EdDsa);
        let k = CoseKey::Rs256 {
            n: vec![1],
            e: vec![1],
        };
        assert_eq!(k.algorithm(), CoseAlg::Rs256);
    }
}
