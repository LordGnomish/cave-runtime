// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DNSKEY record + trust anchor (RFC 4034 §2 / RFC 5011).

use serde::{Deserialize, Serialize};

/// Standard DNSKEY flag bits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnskeyFlags {
    pub zone_key: bool,
    pub secure_entry_point: bool,
    pub revoke: bool,
}

impl DnskeyFlags {
    pub fn from_bits(bits: u16) -> Self {
        Self {
            zone_key: bits & 0x0100 != 0,
            secure_entry_point: bits & 0x0001 != 0,
            revoke: bits & 0x0080 != 0,
        }
    }

    pub fn to_bits(self) -> u16 {
        let mut out = 0u16;
        if self.zone_key {
            out |= 0x0100;
        }
        if self.secure_entry_point {
            out |= 0x0001;
        }
        if self.revoke {
            out |= 0x0080;
        }
        out
    }

    /// KSK = ZK + SEP, ZSK = ZK only.
    pub fn is_ksk(self) -> bool {
        self.zone_key && self.secure_entry_point && !self.revoke
    }

    pub fn is_zsk(self) -> bool {
        self.zone_key && !self.secure_entry_point && !self.revoke
    }
}

/// A DNSKEY record's parsed payload (without the wire envelope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dnskey {
    pub owner: String,
    pub flags: DnskeyFlags,
    pub protocol: u8,
    pub algorithm: u8,
    pub public_key: Vec<u8>,
}

impl Dnskey {
    pub fn key_tag(&self) -> u16 {
        // RFC 4034 Appendix B — sum-of-octets keytag algorithm.
        // Reconstruct the on-wire RDATA: flags || protocol || algorithm || pubkey.
        let mut buf = Vec::with_capacity(4 + self.public_key.len());
        buf.extend_from_slice(&self.flags.to_bits().to_be_bytes());
        buf.push(self.protocol);
        buf.push(self.algorithm);
        buf.extend_from_slice(&self.public_key);
        let mut ac: u32 = 0;
        for (i, b) in buf.iter().enumerate() {
            ac += if i & 1 == 1 { *b as u32 } else { (*b as u32) << 8 };
        }
        ac += (ac >> 16) & 0xFFFF;
        (ac & 0xFFFF) as u16
    }
}

/// A trust anchor: typically the root-zone KSK fingerprint that the
/// validator pins to begin a chain-of-trust walk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustAnchor {
    pub owner: String,
    pub key_tag: u16,
    pub algorithm: u8,
    /// DS digest type (1 = SHA-1, 2 = SHA-256, 4 = SHA-384).
    pub digest_type: u8,
    pub digest: Vec<u8>,
}

impl TrustAnchor {
    pub fn root_iana_2017() -> Self {
        // KSK-2017 placeholder digest; production validators load IANA root
        // anchors from disk via the same shape.
        Self {
            owner: ".".into(),
            key_tag: 20326,
            algorithm: 8, // RSA/SHA-256
            digest_type: 2,
            digest: vec![0xE0; 32],
        }
    }

    pub fn matches(&self, key: &Dnskey) -> bool {
        self.owner == key.owner
            && self.algorithm == key.algorithm
            && self.key_tag == key.key_tag()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ksk() -> Dnskey {
        Dnskey {
            owner: ".".into(),
            flags: DnskeyFlags {
                zone_key: true,
                secure_entry_point: true,
                revoke: false,
            },
            protocol: 3,
            algorithm: 8,
            public_key: vec![0xDE; 64],
        }
    }

    #[test]
    fn flags_roundtrip_ksk() {
        let f = DnskeyFlags {
            zone_key: true,
            secure_entry_point: true,
            ..Default::default()
        };
        assert!(f.is_ksk());
        assert!(!f.is_zsk());
        assert_eq!(DnskeyFlags::from_bits(f.to_bits()), f);
    }

    #[test]
    fn flags_roundtrip_zsk() {
        let f = DnskeyFlags {
            zone_key: true,
            ..Default::default()
        };
        assert!(f.is_zsk());
        assert!(!f.is_ksk());
    }

    #[test]
    fn flags_revoked_is_neither() {
        let f = DnskeyFlags {
            zone_key: true,
            secure_entry_point: true,
            revoke: true,
        };
        assert!(!f.is_ksk());
        assert!(!f.is_zsk());
    }

    #[test]
    fn key_tag_is_deterministic() {
        let k = ksk();
        let tag = k.key_tag();
        assert_eq!(tag, k.key_tag());
        // Sanity: distinct key bytes -> distinct tag.
        let mut k2 = k.clone();
        k2.public_key[0] = 0xAA;
        assert_ne!(tag, k2.key_tag());
    }

    #[test]
    fn trust_anchor_matches_only_same_tag_alg_owner() {
        let k = ksk();
        let mut ta = TrustAnchor::root_iana_2017();
        ta.key_tag = k.key_tag();
        ta.algorithm = 8;
        ta.owner = ".".into();
        assert!(ta.matches(&k));
        ta.algorithm = 13;
        assert!(!ta.matches(&k));
    }

    #[test]
    fn root_anchor_well_known_shape() {
        let r = TrustAnchor::root_iana_2017();
        assert_eq!(r.owner, ".");
        assert_eq!(r.algorithm, 8);
        assert_eq!(r.digest_type, 2);
        assert_eq!(r.digest.len(), 32);
    }
}
