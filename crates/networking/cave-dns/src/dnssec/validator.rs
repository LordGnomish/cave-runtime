// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Top-level DNSSEC validator — orchestrates trust-anchor + RRSIG +
//! NSEC/NSEC3 checks into a single Secure / Insecure / Bogus verdict.
//!
//! The cryptographic verify step is delegated to the protocol layer (which
//! wraps hickory_proto); this module composes the policy decisions.

use serde::{Deserialize, Serialize};

use super::{
    dnskey::{Dnskey, TrustAnchor},
    nsec::Nsec,
    nsec3::Nsec3,
    rrsig::Rrsig,
};

/// RFC 4035 §4.3 validation verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationOutcome {
    Secure,
    Insecure,
    Bogus,
    Indeterminate,
}

pub struct Validator {
    pub trust_anchors: Vec<TrustAnchor>,
    pub now_epoch: u64,
}

impl Validator {
    pub fn new(trust_anchors: Vec<TrustAnchor>, now_epoch: u64) -> Self {
        Self {
            trust_anchors,
            now_epoch,
        }
    }

    pub fn with_root_anchor(now_epoch: u64) -> Self {
        Self {
            trust_anchors: vec![TrustAnchor::root_iana_2017()],
            now_epoch,
        }
    }

    /// Check whether `key` matches a configured trust anchor — first link
    /// of the chain-of-trust walk.
    pub fn anchored(&self, key: &Dnskey) -> bool {
        self.trust_anchors.iter().any(|ta| ta.matches(key))
    }

    /// Validate a single RRSIG's metadata (validity window + algorithm
    /// modernity). Cryptographic verification happens in the protocol
    /// layer.
    pub fn validate_rrsig_meta(&self, sig: &Rrsig) -> ValidationOutcome {
        if !sig.algorithm.is_modern() {
            return if sig.algorithm.is_legacy() {
                ValidationOutcome::Insecure
            } else {
                ValidationOutcome::Bogus
            };
        }
        if !sig.valid_at(self.now_epoch) {
            return ValidationOutcome::Bogus;
        }
        ValidationOutcome::Secure
    }

    /// Validate that the given NSEC chain proves a denial of existence.
    pub fn validate_nsec_denial(&self, nsec: &Nsec, qname: &str, qtype: u16) -> ValidationOutcome {
        if nsec.covers(qname) || nsec.proves_no_type(qname, qtype) {
            ValidationOutcome::Secure
        } else {
            ValidationOutcome::Bogus
        }
    }

    /// Validate that the given NSEC3 chain proves a denial of existence.
    pub fn validate_nsec3_denial(
        &self,
        nsec3: &Nsec3,
        qname: &str,
        qtype: u16,
    ) -> ValidationOutcome {
        if nsec3.covers_name(qname) || nsec3.proves_no_type(qname, qtype) {
            ValidationOutcome::Secure
        } else {
            ValidationOutcome::Bogus
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::dnskey::DnskeyFlags;
    use super::super::nsec3::Nsec3Params;
    use super::super::rrsig::RrsigAlgorithm;
    use super::*;

    fn ed25519_modern_sig() -> Rrsig {
        Rrsig {
            type_covered: 1,
            algorithm: RrsigAlgorithm::Ed25519,
            labels: 2,
            original_ttl: 3600,
            sig_inception: 100,
            sig_expiration: 1000,
            key_tag: 99,
            signer_name: "example.com.".into(),
            signature: vec![0u8; 64],
        }
    }

    fn legacy_sig() -> Rrsig {
        Rrsig {
            algorithm: RrsigAlgorithm::RsaSha1,
            ..ed25519_modern_sig()
        }
    }

    fn unknown_alg_sig() -> Rrsig {
        Rrsig {
            algorithm: RrsigAlgorithm::Unknown,
            ..ed25519_modern_sig()
        }
    }

    #[test]
    fn anchored_matches_via_trust_anchor() {
        let key = Dnskey {
            owner: ".".into(),
            flags: DnskeyFlags {
                zone_key: true,
                secure_entry_point: true,
                ..Default::default()
            },
            protocol: 3,
            algorithm: 8,
            public_key: vec![0xDE; 64],
        };
        let mut ta = TrustAnchor::root_iana_2017();
        ta.key_tag = key.key_tag();
        ta.algorithm = 8;
        let v = Validator::new(vec![ta], 500);
        assert!(v.anchored(&key));
    }

    #[test]
    fn rrsig_meta_secure_when_modern_and_in_window() {
        let v = Validator::with_root_anchor(500);
        assert_eq!(v.validate_rrsig_meta(&ed25519_modern_sig()), ValidationOutcome::Secure);
    }

    #[test]
    fn rrsig_meta_bogus_when_expired() {
        let v = Validator::with_root_anchor(5000);
        assert_eq!(v.validate_rrsig_meta(&ed25519_modern_sig()), ValidationOutcome::Bogus);
    }

    #[test]
    fn rrsig_meta_insecure_for_legacy_alg() {
        let v = Validator::with_root_anchor(500);
        assert_eq!(v.validate_rrsig_meta(&legacy_sig()), ValidationOutcome::Insecure);
    }

    #[test]
    fn rrsig_meta_bogus_for_unknown_alg() {
        let v = Validator::with_root_anchor(500);
        assert_eq!(v.validate_rrsig_meta(&unknown_alg_sig()), ValidationOutcome::Bogus);
    }

    #[test]
    fn nsec_denial_secure_when_covers_or_no_type() {
        let v = Validator::with_root_anchor(500);
        let n = Nsec::new("a.example.com.", "c.example.com.", vec![1]);
        assert_eq!(v.validate_nsec_denial(&n, "b.example.com.", 28), ValidationOutcome::Secure);
        let n2 = Nsec::new("svc.example.com.", "tmp.example.com.", vec![1]);
        assert_eq!(v.validate_nsec_denial(&n2, "svc.example.com.", 28), ValidationOutcome::Secure);
    }

    #[test]
    fn nsec_denial_bogus_when_no_proof() {
        let v = Validator::with_root_anchor(500);
        let n = Nsec::new("a.example.com.", "c.example.com.", vec![1]);
        assert_eq!(v.validate_nsec_denial(&n, "z.example.com.", 28), ValidationOutcome::Bogus);
    }

    #[test]
    fn nsec3_denial_secure_when_covers_name() {
        let v = Validator::with_root_anchor(500);
        let params = Nsec3Params::sha1_default();
        let h = super::super::nsec3::hash_name("svc.example.com.", &params);
        let n3 = Nsec3::new(params, h.clone(), vec![0xFF; 20], vec![1]);
        assert_eq!(v.validate_nsec3_denial(&n3, "svc.example.com.", 28), ValidationOutcome::Secure);
    }
}
