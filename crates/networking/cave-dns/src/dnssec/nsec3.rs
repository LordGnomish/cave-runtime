// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NSEC3 hashed denial-of-existence (RFC 5155).
//!
//! cave-dns ships a deterministic placeholder hash (FNV-1a 32-bit padded
//! to 20 bytes — the SHA-1 output size NSEC3 uses on the wire). Production
//! validators replace `hash_name` with the ring-backed SHA-1 implementation
//! shared by the cave-pki crate.

use serde::{Deserialize, Serialize};

/// NSEC3 parameters from an NSEC3PARAM / NSEC3 record's rdata header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nsec3Params {
    pub hash_algorithm: u8,
    pub flags: u8,
    pub iterations: u16,
    pub salt: Vec<u8>,
}

impl Nsec3Params {
    pub fn sha1_default() -> Self {
        Self {
            hash_algorithm: 1, // SHA-1
            flags: 0,
            iterations: 0,
            salt: vec![],
        }
    }

    pub fn opt_out(&self) -> bool {
        self.flags & 0x01 != 0
    }
}

/// A parsed NSEC3 record: a hashed-owner-name link to the next hashed name
/// in canonical hash order, plus the type-bitmap at the original owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nsec3 {
    pub params: Nsec3Params,
    pub hashed_owner: Vec<u8>,
    pub next_hashed_owner: Vec<u8>,
    pub type_bitmap: Vec<u16>,
}

impl Nsec3 {
    pub fn new(
        params: Nsec3Params,
        hashed_owner: Vec<u8>,
        next_hashed_owner: Vec<u8>,
        types: Vec<u16>,
    ) -> Self {
        let mut t = types;
        t.sort_unstable();
        t.dedup();
        Self {
            params,
            hashed_owner,
            next_hashed_owner,
            type_bitmap: t,
        }
    }

    /// Does this NSEC3 record cover the hashed form of `name`?
    pub fn covers_hash(&self, hashed: &[u8]) -> bool {
        let h = hashed.to_vec();
        bytes_lt(&self.hashed_owner, &h) && bytes_lt(&h, &self.next_hashed_owner)
    }

    /// Convenience: hash + cover check.
    pub fn covers_name(&self, name: &str) -> bool {
        let h = hash_name(name, &self.params);
        self.covers_hash(&h)
    }

    pub fn proves_no_type(&self, name: &str, qtype: u16) -> bool {
        let h = hash_name(name, &self.params);
        h == self.hashed_owner && !self.type_bitmap.contains(&qtype)
    }
}

/// Placeholder hash — FNV-1a 32-bit, padded with zero bytes to 20 bytes
/// (SHA-1 output size). Deterministic + sufficient for unit-level cover
/// tests; production replaces with SHA-1 + salt + iteration loop per RFC.
pub fn hash_name(name: &str, params: &Nsec3Params) -> Vec<u8> {
    let mut bytes = name.to_ascii_lowercase().into_bytes();
    bytes.extend_from_slice(&params.salt);
    let mut hash: u32 = 0x811C9DC5;
    for _ in 0..=params.iterations {
        for b in &bytes {
            hash ^= *b as u32;
            hash = hash.wrapping_mul(0x01000193);
        }
    }
    let mut out = vec![0u8; 20];
    out[..4].copy_from_slice(&hash.to_be_bytes());
    out
}

fn bytes_lt(a: &[u8], b: &[u8]) -> bool {
    a < b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsec3_params_default_sha1() {
        let p = Nsec3Params::sha1_default();
        assert_eq!(p.hash_algorithm, 1);
        assert_eq!(p.iterations, 0);
        assert!(p.salt.is_empty());
        assert!(!p.opt_out());
    }

    #[test]
    fn nsec3_opt_out_flag_bit_zero() {
        let p = Nsec3Params {
            flags: 0x01,
            ..Nsec3Params::sha1_default()
        };
        assert!(p.opt_out());
    }

    #[test]
    fn hash_name_is_deterministic_and_salt_sensitive() {
        let p1 = Nsec3Params::sha1_default();
        let mut p2 = p1.clone();
        p2.salt = vec![0xAB, 0xCD];
        let h1 = hash_name("svc.example.com.", &p1);
        let h2 = hash_name("svc.example.com.", &p2);
        assert_eq!(hash_name("svc.example.com.", &p1), h1);
        assert_ne!(h1, h2);
        assert_eq!(h1.len(), 20);
    }

    #[test]
    fn nsec3_covers_hash_lexicographically_between() {
        let params = Nsec3Params::sha1_default();
        let n = Nsec3::new(params, vec![0x10, 0x00], vec![0x30, 0x00], vec![1]);
        assert!(n.covers_hash(&[0x20, 0x00]));
        assert!(!n.covers_hash(&[0x40, 0x00]));
        assert!(!n.covers_hash(&[0x10, 0x00]));
    }

    #[test]
    fn nsec3_proves_no_type_when_hash_matches_and_bit_absent() {
        let params = Nsec3Params::sha1_default();
        let h_a = hash_name("svc.example.com.", &params);
        let n = Nsec3::new(params, h_a.clone(), vec![0xFF; 20], vec![1]);
        assert!(n.proves_no_type("svc.example.com.", 28));
        assert!(!n.proves_no_type("svc.example.com.", 1));
        assert!(!n.proves_no_type("other.example.com.", 28));
    }
}
