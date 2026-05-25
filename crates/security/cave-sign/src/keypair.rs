// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Keypair PEM encode/decode + key handle wrapper.
//!
//! Maps to:
//!   * cmd/cosign/cli/generate_key_pair.go → KeyPairGenerator
//!   * cmd/cosign/cli/importkeypair        → ImportKeyPair
//!   * cmd/cosign/cli/public_key.go        → ExportPublicKey
//!
//! cosign serialises P-256 keys to PEM with the `ENCRYPTED COSIGN PRIVATE KEY`
//! header; we use the compatible `COSIGN PRIVATE KEY` / `PUBLIC KEY` headers
//! and base64-encode raw secret/public bytes. Encryption is *not* applied —
//! private keys are expected to come from cave-vault, not from disk.

use crate::error::{Result, SignError};
use crate::models::KeyAlgorithm;
use crate::signature::Keypair;
use base64::Engine;

const PRIVATE_HEADER: &str = "COSIGN PRIVATE KEY";
const PUBLIC_HEADER: &str = "PUBLIC KEY";

#[derive(Debug, Clone)]
pub struct KeyHandle {
    pub algorithm: KeyAlgorithm,
    pub public_key_pem: String,
}

impl KeyHandle {
    pub fn from_keypair(kp: &Keypair) -> Self {
        Self {
            algorithm: kp.algorithm,
            public_key_pem: encode_public_pem(kp.algorithm, kp.public_key_bytes()),
        }
    }
}

/// Encode the public key bytes as a PEM block. The body carries
/// `algorithm-tag:<base64>` so we can recover the algorithm on decode.
pub fn encode_public_pem(algorithm: KeyAlgorithm, public_key: &[u8]) -> String {
    encode_pem(
        PUBLIC_HEADER,
        algorithm,
        &base64::engine::general_purpose::STANDARD.encode(public_key),
    )
}

/// Encode a private key in PEM form (UNENCRYPTED). Cave-vault is expected
/// to encrypt at rest; this is the line-protocol representation only.
pub fn encode_private_pem(kp: &Keypair) -> String {
    // SAFETY of secret material: callers must scrub the returned string.
    let body = base64::engine::general_purpose::STANDARD.encode(secret_bytes(kp));
    encode_pem(PRIVATE_HEADER, kp.algorithm, &body)
}

/// Decode a PEM block produced by `encode_public_pem`.
pub fn decode_public_pem(pem: &str) -> Result<(KeyAlgorithm, Vec<u8>)> {
    let (header, alg, body) = parse_pem(pem)?;
    if header != PUBLIC_HEADER {
        return Err(SignError::Pem(format!(
            "expected {} header, got {}",
            PUBLIC_HEADER, header
        )));
    }
    Ok((alg, body))
}

/// Decode a PEM block produced by `encode_private_pem`. Returns a fully
/// reconstructed `Keypair`.
pub fn decode_private_pem(pem: &str) -> Result<Keypair> {
    let (header, alg, body) = parse_pem(pem)?;
    if header != PRIVATE_HEADER {
        return Err(SignError::Pem(format!(
            "expected {} header, got {}",
            PRIVATE_HEADER, header
        )));
    }
    if body.len() != 32 {
        return Err(SignError::Pem(format!(
            "private key body must be 32 bytes (got {})",
            body.len()
        )));
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&body);
    Keypair::from_seed(alg, &seed)
}

fn encode_pem(header: &str, alg: KeyAlgorithm, b64_body: &str) -> String {
    // 64-char wrapping matches RFC 7468.
    let mut out = String::new();
    out.push_str(&format!("-----BEGIN {}-----\n", header));
    out.push_str(&format!("algorithm: {}\n", alg.as_str()));
    out.push('\n');
    for chunk in b64_body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {}-----\n", header));
    out
}

fn parse_pem(pem: &str) -> Result<(String, KeyAlgorithm, Vec<u8>)> {
    let trimmed = pem.trim();
    let begin_line = trimmed
        .lines()
        .next()
        .ok_or_else(|| SignError::Pem("empty pem".into()))?;
    let header = begin_line
        .strip_prefix("-----BEGIN ")
        .and_then(|s| s.strip_suffix("-----"))
        .ok_or_else(|| SignError::Pem("missing BEGIN line".into()))?
        .to_string();
    let end_marker = format!("-----END {}-----", header);
    if !trimmed.contains(&end_marker) {
        return Err(SignError::Pem("missing END line".into()));
    }
    // Split header lines (algorithm: ...) from body.
    let inner: Vec<&str> = trimmed
        .lines()
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let mut algorithm: Option<KeyAlgorithm> = None;
    let mut body_lines: Vec<&str> = Vec::new();
    let mut after_blank = false;
    for line in inner {
        if line.starts_with("algorithm:") {
            let v = line.trim_start_matches("algorithm:").trim();
            algorithm = match v {
                "ecdsa-p256" => Some(KeyAlgorithm::EcdsaP256),
                "ed25519" => Some(KeyAlgorithm::Ed25519),
                other => return Err(SignError::Pem(format!("unknown algorithm {}", other))),
            };
            continue;
        }
        if line.is_empty() {
            after_blank = true;
            continue;
        }
        if after_blank || algorithm.is_none() {
            body_lines.push(line);
        } else {
            // Tolerate body without header-section divider (legacy PEM).
            body_lines.push(line);
        }
    }
    let alg = algorithm.ok_or_else(|| SignError::Pem("missing algorithm header".into()))?;
    let body_b64 = body_lines.join("");
    let body = base64::engine::general_purpose::STANDARD
        .decode(body_b64.as_bytes())
        .map_err(|e| SignError::Pem(format!("base64: {}", e)))?;
    Ok((header, alg, body))
}

fn secret_bytes(kp: &Keypair) -> Vec<u8> {
    crate::signature::__internal_secret_bytes(kp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_pem_roundtrip_p256() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
        let pem = encode_public_pem(kp.algorithm, kp.public_key_bytes());
        assert!(pem.contains("BEGIN PUBLIC KEY"));
        let (alg, body) = decode_public_pem(&pem).unwrap();
        assert_eq!(alg, KeyAlgorithm::EcdsaP256);
        assert_eq!(body, kp.public_key_bytes().to_vec());
    }

    #[test]
    fn public_pem_roundtrip_ed25519() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[2u8; 32]).unwrap();
        let pem = encode_public_pem(kp.algorithm, kp.public_key_bytes());
        let (alg, body) = decode_public_pem(&pem).unwrap();
        assert_eq!(alg, KeyAlgorithm::Ed25519);
        assert_eq!(body, kp.public_key_bytes().to_vec());
    }

    #[test]
    fn private_pem_roundtrip() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[4u8; 32]).unwrap();
        let pem = encode_private_pem(&kp);
        assert!(pem.contains("BEGIN COSIGN PRIVATE KEY"));
        let restored = decode_private_pem(&pem).unwrap();
        assert_eq!(restored.algorithm, kp.algorithm);
        assert_eq!(restored.public_key_bytes(), kp.public_key_bytes());
    }

    #[test]
    fn private_pem_signs_compatibly() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[6u8; 32]).unwrap();
        let pem = encode_private_pem(&kp);
        let restored = decode_private_pem(&pem).unwrap();
        let sig = restored.sign(b"hello").unwrap();
        crate::signature::verify(KeyAlgorithm::Ed25519, kp.public_key_bytes(), b"hello", &sig)
            .unwrap();
    }

    #[test]
    fn rejects_unknown_algorithm() {
        let bogus = "-----BEGIN PUBLIC KEY-----\nalgorithm: rsa-4096\n\nAAAA\n-----END PUBLIC KEY-----\n";
        let err = decode_public_pem(bogus).expect_err("must reject");
        assert!(matches!(err, SignError::Pem(_)));
    }

    #[test]
    fn rejects_wrong_header() {
        let bogus = "-----BEGIN COSIGN PRIVATE KEY-----\nalgorithm: ed25519\n\nAAAA\n-----END COSIGN PRIVATE KEY-----\n";
        let err = decode_public_pem(bogus).expect_err("must reject");
        assert!(matches!(err, SignError::Pem(_)));
    }

    #[test]
    fn rejects_missing_end() {
        let bogus = "-----BEGIN PUBLIC KEY-----\nalgorithm: ed25519\n\nAAAA\n";
        let err = decode_public_pem(bogus).expect_err("must reject");
        assert!(matches!(err, SignError::Pem(_)));
    }

    #[test]
    fn pem_lines_at_64_chars() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[11u8; 32]).unwrap();
        let pem = encode_public_pem(kp.algorithm, kp.public_key_bytes());
        for l in pem.lines() {
            if l.starts_with("-----") || l.starts_with("algorithm:") || l.is_empty() {
                continue;
            }
            assert!(l.len() <= 64, "line too long: {}", l);
        }
    }

    #[test]
    fn handle_carries_algorithm() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[12u8; 32]).unwrap();
        let h = KeyHandle::from_keypair(&kp);
        assert_eq!(h.algorithm, KeyAlgorithm::EcdsaP256);
        assert!(h.public_key_pem.contains("BEGIN PUBLIC KEY"));
    }
}
