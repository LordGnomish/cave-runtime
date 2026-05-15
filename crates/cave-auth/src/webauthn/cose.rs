// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/data/attestation/authenticator/COSEKey.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/attestation/authenticator/EC2COSEKey.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/attestation/authenticator/RSACOSEKey.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/attestation/authenticator/EdDSACOSEKey.java
//
// COSE_Key parsing — RFC 8152 §7.  WebAuthn carries the credential
// public key as a COSE_Key inside the `attestedCredentialData` of the
// `authenticatorData`.  Only the algorithms required by WebAuthn L3 are
// implemented:
//
//   alg=-7   ES256   EC2  P-256  + SHA-256
//   alg=-35  ES384   EC2  P-384  + SHA-384
//   alg=-257 RS256   RSA         + SHA-256
//   alg=-8   EdDSA   OKP  Ed25519

use std::collections::BTreeMap;

/// COSE Algorithm identifier (RFC 8152 §16.4 + later registrations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoseAlgorithm {
    Es256,
    Es384,
    Rs256,
    EdDsa,
}

impl CoseAlgorithm {
    pub fn from_i64(v: i64) -> Option<Self> {
        match v {
            -7 => Some(Self::Es256),
            -35 => Some(Self::Es384),
            -257 => Some(Self::Rs256),
            -8 => Some(Self::EdDsa),
            _ => None,
        }
    }

    pub fn as_i64(self) -> i64 {
        match self {
            Self::Es256 => -7,
            Self::Es384 => -35,
            Self::Rs256 => -257,
            Self::EdDsa => -8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Es256 => "ES256",
            Self::Es384 => "ES384",
            Self::Rs256 => "RS256",
            Self::EdDsa => "EdDSA",
        }
    }
}

/// Decoded COSE_Key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoseKey {
    Ec2 {
        alg: CoseAlgorithm,
        crv: i64,
        x: Vec<u8>,
        y: Vec<u8>,
    },
    Rsa {
        alg: CoseAlgorithm,
        n: Vec<u8>,
        e: Vec<u8>,
    },
    Okp {
        alg: CoseAlgorithm,
        crv: i64,
        x: Vec<u8>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CoseError {
    #[error("malformed CBOR: {0}")]
    Cbor(String),
    #[error("not a COSE_Key map")]
    NotAMap,
    #[error("missing required label {label}")]
    MissingLabel { label: i64 },
    #[error("unsupported kty {kty}")]
    UnsupportedKty { kty: i64 },
    #[error("unsupported alg {alg}")]
    UnsupportedAlg { alg: i64 },
    #[error("unsupported crv {crv} for kty {kty}")]
    UnsupportedCrv { kty: i64, crv: i64 },
    #[error("invalid coordinate length: expected {expected}, got {got}")]
    BadCoordinate { expected: usize, got: usize },
}

impl CoseKey {
    pub fn alg(&self) -> CoseAlgorithm {
        match self {
            Self::Ec2 { alg, .. } | Self::Rsa { alg, .. } | Self::Okp { alg, .. } => *alg,
        }
    }

    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CoseError> {
        let value: ciborium::Value =
            ciborium::de::from_reader(bytes).map_err(|e| CoseError::Cbor(e.to_string()))?;
        let map = match value {
            ciborium::Value::Map(m) => m,
            _ => return Err(CoseError::NotAMap),
        };
        let labels = canonicalise(map)?;
        let kty = labels
            .get(&1)
            .and_then(as_int)
            .ok_or(CoseError::MissingLabel { label: 1 })?;
        let alg_raw = labels
            .get(&3)
            .and_then(as_int)
            .ok_or(CoseError::MissingLabel { label: 3 })?;
        let alg = CoseAlgorithm::from_i64(alg_raw)
            .ok_or(CoseError::UnsupportedAlg { alg: alg_raw })?;
        match kty {
            2 => {
                let crv = labels
                    .get(&-1)
                    .and_then(as_int)
                    .ok_or(CoseError::MissingLabel { label: -1 })?;
                let coord_len = match (alg, crv) {
                    (CoseAlgorithm::Es256, 1) => 32,
                    (CoseAlgorithm::Es384, 2) => 48,
                    _ => return Err(CoseError::UnsupportedCrv { kty, crv }),
                };
                let x = labels
                    .get(&-2)
                    .and_then(as_bytes)
                    .ok_or(CoseError::MissingLabel { label: -2 })?;
                let y = labels
                    .get(&-3)
                    .and_then(as_bytes)
                    .ok_or(CoseError::MissingLabel { label: -3 })?;
                check_len(coord_len, x.len())?;
                check_len(coord_len, y.len())?;
                Ok(Self::Ec2 { alg, crv, x, y })
            }
            3 => {
                if alg != CoseAlgorithm::Rs256 {
                    return Err(CoseError::UnsupportedAlg { alg: alg_raw });
                }
                let n = labels
                    .get(&-1)
                    .and_then(as_bytes)
                    .ok_or(CoseError::MissingLabel { label: -1 })?;
                let e = labels
                    .get(&-2)
                    .and_then(as_bytes)
                    .ok_or(CoseError::MissingLabel { label: -2 })?;
                if n.len() < 256 {
                    return Err(CoseError::BadCoordinate {
                        expected: 256,
                        got: n.len(),
                    });
                }
                Ok(Self::Rsa { alg, n, e })
            }
            1 => {
                if alg != CoseAlgorithm::EdDsa {
                    return Err(CoseError::UnsupportedAlg { alg: alg_raw });
                }
                let crv = labels
                    .get(&-1)
                    .and_then(as_int)
                    .ok_or(CoseError::MissingLabel { label: -1 })?;
                if crv != 6 {
                    return Err(CoseError::UnsupportedCrv { kty, crv });
                }
                let x = labels
                    .get(&-2)
                    .and_then(as_bytes)
                    .ok_or(CoseError::MissingLabel { label: -2 })?;
                check_len(32, x.len())?;
                Ok(Self::Okp { alg, crv, x })
            }
            other => Err(CoseError::UnsupportedKty { kty: other }),
        }
    }
}

fn check_len(expected: usize, got: usize) -> Result<(), CoseError> {
    if expected == got {
        Ok(())
    } else {
        Err(CoseError::BadCoordinate { expected, got })
    }
}

fn canonicalise(
    pairs: Vec<(ciborium::Value, ciborium::Value)>,
) -> Result<BTreeMap<i64, ciborium::Value>, CoseError> {
    let mut out = BTreeMap::new();
    for (k, v) in pairs {
        let int = as_int(&k).ok_or(CoseError::NotAMap)?;
        out.insert(int, v);
    }
    Ok(out)
}

fn as_int(v: &ciborium::Value) -> Option<i64> {
    match v {
        ciborium::Value::Integer(i) => i128::from(*i).try_into().ok(),
        _ => None,
    }
}

fn as_bytes(v: &ciborium::Value) -> Option<Vec<u8>> {
    match v {
        ciborium::Value::Bytes(b) => Some(b.clone()),
        _ => None,
    }
}
