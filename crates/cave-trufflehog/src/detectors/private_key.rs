// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PEM-encoded private keys — port of `pkg/detectors/privatekey/`. Catches
//! RSA / EC / Ed25519 / OpenSSH private-key headers.

use crate::detector::Detector;
use crate::models::{DetectionResult, DetectorType};
use regex::Regex;
use std::sync::OnceLock;

pub struct PrivateKey;

static RE: OnceLock<Regex> = OnceLock::new();

fn re() -> &'static Regex {
    // Match the entire PEM block — anchored on BEGIN/END markers, body is
    // any base64 + newline characters in between.
    RE.get_or_init(|| {
        Regex::new(
            r"(?s)-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY( BLOCK)?-----.+?-----END (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY( BLOCK)?-----",
        )
        .unwrap()
    })
}

impl Detector for PrivateKey {
    fn detector_type(&self) -> DetectorType {
        DetectorType::PrivateKey
    }
    fn description(&self) -> &'static str {
        "PEM-encoded private key (RSA / EC / DSA / OpenSSH / PGP)"
    }
    fn keywords(&self) -> &'static [&'static str] {
        &["PRIVATE KEY"]
    }
    fn from_data(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        re()
            .find_iter(s)
            .map(|m| DetectionResult::new(DetectorType::PrivateKey, m.as_str()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rsa_private_key() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOQIBAAJBALR1L5kZ\n-----END RSA PRIVATE KEY-----";
        let r = PrivateKey.from_data(pem.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_openssh_private_key() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXkt\n-----END OPENSSH PRIVATE KEY-----";
        let r = PrivateKey.from_data(pem.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn detects_pgp_private_key_block() {
        let pem = "-----BEGIN PGP PRIVATE KEY BLOCK-----\nbody\n-----END PGP PRIVATE KEY BLOCK-----";
        let r = PrivateKey.from_data(pem.as_bytes());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn rejects_unmatched_header() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOQ\nNO_END";
        assert!(PrivateKey.from_data(pem.as_bytes()).is_empty());
    }
}
