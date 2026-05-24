// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RRSIG record + algorithm enum + validity check (RFC 4034 §3).

use serde::{Deserialize, Serialize};

/// RRSIG algorithm registry (IANA Domain Name System Security (DNSSEC)
/// Algorithm Numbers) — the cave-dns validator only commits to the active
/// modern algorithms; legacy MD5 / SHA-1 entries decode but reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RrsigAlgorithm {
    RsaSha1 = 5,
    RsaSha256 = 8,
    RsaSha512 = 10,
    EcdsaP256Sha256 = 13,
    EcdsaP384Sha384 = 14,
    Ed25519 = 15,
    Ed448 = 16,
    Unknown = 0,
}

impl RrsigAlgorithm {
    pub fn from_u8(v: u8) -> Self {
        match v {
            5 => Self::RsaSha1,
            8 => Self::RsaSha256,
            10 => Self::RsaSha512,
            13 => Self::EcdsaP256Sha256,
            14 => Self::EcdsaP384Sha384,
            15 => Self::Ed25519,
            16 => Self::Ed448,
            _ => Self::Unknown,
        }
    }

    pub fn is_legacy(self) -> bool {
        matches!(self, Self::RsaSha1)
    }

    pub fn is_modern(self) -> bool {
        matches!(
            self,
            Self::RsaSha256
                | Self::RsaSha512
                | Self::EcdsaP256Sha256
                | Self::EcdsaP384Sha384
                | Self::Ed25519
                | Self::Ed448
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rrsig {
    pub type_covered: u16,
    pub algorithm: RrsigAlgorithm,
    pub labels: u8,
    pub original_ttl: u32,
    /// Unix epoch seconds.
    pub sig_expiration: u64,
    /// Unix epoch seconds.
    pub sig_inception: u64,
    pub key_tag: u16,
    pub signer_name: String,
    pub signature: Vec<u8>,
}

impl Rrsig {
    /// Validity gate: now in [inception, expiration].
    pub fn valid_at(&self, now_epoch: u64) -> bool {
        now_epoch >= self.sig_inception && now_epoch <= self.sig_expiration
    }

    /// Seconds until expiration (0 if already expired).
    pub fn time_to_expiry(&self, now_epoch: u64) -> u64 {
        self.sig_expiration.saturating_sub(now_epoch)
    }

    /// Standard freshness flag: signatures with <= refresh_window seconds
    /// of life left should be re-signed.
    pub fn needs_refresh(&self, now_epoch: u64, refresh_window: u64) -> bool {
        self.time_to_expiry(now_epoch) <= refresh_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed25519_sig(inception: u64, expiration: u64) -> Rrsig {
        Rrsig {
            type_covered: 1, // A
            algorithm: RrsigAlgorithm::Ed25519,
            labels: 2,
            original_ttl: 3600,
            sig_expiration: expiration,
            sig_inception: inception,
            key_tag: 12345,
            signer_name: "example.com.".into(),
            signature: vec![0u8; 64],
        }
    }

    #[test]
    fn algorithm_codes_roundtrip() {
        assert_eq!(RrsigAlgorithm::from_u8(13), RrsigAlgorithm::EcdsaP256Sha256);
        assert_eq!(RrsigAlgorithm::from_u8(15), RrsigAlgorithm::Ed25519);
        assert_eq!(RrsigAlgorithm::from_u8(99), RrsigAlgorithm::Unknown);
    }

    #[test]
    fn rsasha1_is_legacy_not_modern() {
        assert!(RrsigAlgorithm::RsaSha1.is_legacy());
        assert!(!RrsigAlgorithm::RsaSha1.is_modern());
    }

    #[test]
    fn ed25519_is_modern_not_legacy() {
        assert!(RrsigAlgorithm::Ed25519.is_modern());
        assert!(!RrsigAlgorithm::Ed25519.is_legacy());
    }

    #[test]
    fn valid_at_window_inclusive() {
        let r = ed25519_sig(1000, 2000);
        assert!(r.valid_at(1000));
        assert!(r.valid_at(1500));
        assert!(r.valid_at(2000));
        assert!(!r.valid_at(999));
        assert!(!r.valid_at(2001));
    }

    #[test]
    fn time_to_expiry_saturates_after_expiration() {
        let r = ed25519_sig(1000, 2000);
        assert_eq!(r.time_to_expiry(1500), 500);
        assert_eq!(r.time_to_expiry(2500), 0);
    }

    #[test]
    fn needs_refresh_below_window() {
        let r = ed25519_sig(1000, 2000);
        assert!(!r.needs_refresh(1000, 500));
        assert!(r.needs_refresh(1700, 500));
        assert!(r.needs_refresh(2000, 1));
    }
}
