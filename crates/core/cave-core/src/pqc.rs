// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Consolidated post-quantum cryptography (PQC) primitive home.
//!
//! This module is the **single source of truth** for PQC algorithm identity,
//! parameter-set sizes, and the length-prefixed composite-signature container
//! used across the cave runtime. It consolidates constants and container logic
//! that were previously duplicated in `cave-certs/src/pqc.rs` and
//! `cave-auth/src/keycloak/pqc.rs`.
//!
//! # PQC-first
//!
//! cave is PQC-first: there is **no classical-only fallback**. Every algorithm
//! enumerated here is a NIST-standardized post-quantum scheme. Hybrid
//! deployments (PQC + a classical signature) are expressed by packing both
//! signatures into a [`CompositeSignature`] container; the classical half is
//! never used on its own.
//!
//! # Honest scope: engines are pluggable, this module ships NO lattice crypto
//!
//! This module performs **no real lattice or hash-based cryptography**. It
//! provides:
//!
//! - [`PqcAlgorithm`]: the algorithm taxonomy with FIPS 203/204/205 sizes,
//!   classification ([`PqcAlgorithm::is_kem`] / [`PqcAlgorithm::is_signature`]),
//!   stable string ids ([`PqcAlgorithm::name`] / [`PqcAlgorithm::oid`]), and the
//!   parameter-set table ([`PqcAlgorithm::sizes`]).
//! - The [`KemEngine`] and [`SignatureEngine`] traits, which real
//!   implementations (e.g. the `fips203` / `fips204` / `fips205` crates, or
//!   `oqs-rs`) plug into. **No engine is implemented here.**
//! - [`CompositeSignature`]: the wire container for one-or-more
//!   length-prefixed signature components, with `assemble`/`parse` and explicit
//!   truncation errors.
//!
//! References: FIPS 203 (ML-KEM), FIPS 204 (ML-DSA), FIPS 205 (SLH-DSA),
//! IETF draft-ietf-lamps-pq-composite-sigs (composite signature value =
//! length-prefixed concatenation of component signatures).

use thiserror::Error;

// ─── Errors ─────────────────────────────────────────────────────────────────

/// Errors raised while assembling or parsing a [`CompositeSignature`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PqcError {
    /// The composite blob ended before a declared field could be read.
    #[error("composite signature truncated (expected \u{2265} {expected} bytes, got {actual})")]
    Truncated { expected: usize, actual: usize },

    /// The leading version byte did not match [`COMPOSITE_VERSION`].
    #[error("composite version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u8, actual: u8 },

    /// A component declared a length larger than `u32::MAX`, which the wire
    /// format cannot encode.
    #[error("composite component too large: {len} bytes exceeds u32::MAX")]
    ComponentTooLarge { len: usize },
}

// ─── Algorithm taxonomy ───────────────────────────────────────────────────────

/// A NIST-standardized post-quantum algorithm.
///
/// KEM variants (FIPS 203, ML-KEM) and signature variants (FIPS 204 ML-DSA,
/// FIPS 205 SLH-DSA) live in one enum so callers can carry an opaque algorithm
/// id and ask it whether it is a KEM or a signature scheme.
///
/// `SlhDsaSha2_*s` are the **small** ("s") SHA2 SLH-DSA parameter sets (smaller
/// signatures, slower signing). The "f" (fast) sets are intentionally not
/// modeled yet; add them here when an engine needs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PqcAlgorithm {
    // FIPS 203 — ML-KEM (key encapsulation)
    MlKem512,
    MlKem768,
    MlKem1024,
    // FIPS 204 — ML-DSA (lattice signatures)
    MlDsa44,
    MlDsa65,
    MlDsa87,
    // FIPS 205 — SLH-DSA (stateless hash-based signatures), SHA2 "small" sets
    SlhDsaSha2_128s,
    SlhDsaSha2_192s,
    SlhDsaSha2_256s,
}

/// Parameter-set sizes for a [`PqcAlgorithm`], all in bytes.
///
/// KEM algorithms populate `public_key_len`, `secret_key_len`,
/// `ciphertext_len`, and `shared_secret_len`; their `signature_len` is `None`.
/// Signature algorithms populate `public_key_len`, `secret_key_len`, and
/// `signature_len`; their `ciphertext_len` and `shared_secret_len` are `None`.
///
/// Sizes are taken directly from the FIPS standards. Any value cave does not
/// have an authoritative source for is left as `None` and documented at the
/// call site rather than guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PqcSizes {
    /// Public/encapsulation key length (always known for the modeled sets).
    pub public_key_len: usize,
    /// Secret/decapsulation key length (always known for the modeled sets).
    pub secret_key_len: usize,
    /// KEM ciphertext length; `None` for signature algorithms.
    pub ciphertext_len: Option<usize>,
    /// KEM shared-secret length; `None` for signature algorithms.
    pub shared_secret_len: Option<usize>,
    /// Signature length; `None` for KEM algorithms.
    pub signature_len: Option<usize>,
}

impl PqcAlgorithm {
    /// All algorithms modeled by this module, in declaration order.
    pub const ALL: [PqcAlgorithm; 9] = [
        PqcAlgorithm::MlKem512,
        PqcAlgorithm::MlKem768,
        PqcAlgorithm::MlKem1024,
        PqcAlgorithm::MlDsa44,
        PqcAlgorithm::MlDsa65,
        PqcAlgorithm::MlDsa87,
        PqcAlgorithm::SlhDsaSha2_128s,
        PqcAlgorithm::SlhDsaSha2_192s,
        PqcAlgorithm::SlhDsaSha2_256s,
    ];

    /// `true` if this is a key-encapsulation mechanism (FIPS 203 ML-KEM).
    pub fn is_kem(self) -> bool {
        matches!(
            self,
            PqcAlgorithm::MlKem512 | PqcAlgorithm::MlKem768 | PqcAlgorithm::MlKem1024
        )
    }

    /// `true` if this is a digital-signature algorithm (FIPS 204 / FIPS 205).
    pub fn is_signature(self) -> bool {
        !self.is_kem()
    }

    /// Stable, human-readable id matching the NIST / IETF naming convention.
    ///
    /// Round-trips with [`PqcAlgorithm::from_name`].
    pub fn name(self) -> &'static str {
        match self {
            PqcAlgorithm::MlKem512 => "ML-KEM-512",
            PqcAlgorithm::MlKem768 => "ML-KEM-768",
            PqcAlgorithm::MlKem1024 => "ML-KEM-1024",
            PqcAlgorithm::MlDsa44 => "ML-DSA-44",
            PqcAlgorithm::MlDsa65 => "ML-DSA-65",
            PqcAlgorithm::MlDsa87 => "ML-DSA-87",
            PqcAlgorithm::SlhDsaSha2_128s => "SLH-DSA-SHA2-128s",
            PqcAlgorithm::SlhDsaSha2_192s => "SLH-DSA-SHA2-192s",
            PqcAlgorithm::SlhDsaSha2_256s => "SLH-DSA-SHA2-256s",
        }
    }

    /// Parse a [`PqcAlgorithm::name`] back into the enum. Case-sensitive.
    pub fn from_name(name: &str) -> Option<PqcAlgorithm> {
        PqcAlgorithm::ALL.into_iter().find(|a| a.name() == name)
    }

    /// NIST CSOR object identifier (dotted string) for this algorithm.
    ///
    /// Cite: NIST Computer Security Objects Register arc `2.16.840.1.101.3.4`
    /// (`.4.*` = ML-KEM, `.3.*` = ML-DSA, SLH-DSA SHA2 under `.3.20+`). These
    /// are the standardized OIDs; engines that need DER encoding should encode
    /// from these dotted forms.
    pub fn oid(self) -> &'static str {
        match self {
            // ML-KEM (FIPS 203): 2.16.840.1.101.3.4.4.{1,2,3}
            PqcAlgorithm::MlKem512 => "2.16.840.1.101.3.4.4.1",
            PqcAlgorithm::MlKem768 => "2.16.840.1.101.3.4.4.2",
            PqcAlgorithm::MlKem1024 => "2.16.840.1.101.3.4.4.3",
            // ML-DSA (FIPS 204): 2.16.840.1.101.3.4.3.{17,18,19}
            PqcAlgorithm::MlDsa44 => "2.16.840.1.101.3.4.3.17",
            PqcAlgorithm::MlDsa65 => "2.16.840.1.101.3.4.3.18",
            PqcAlgorithm::MlDsa87 => "2.16.840.1.101.3.4.3.19",
            // SLH-DSA SHA2 small (FIPS 205): 2.16.840.1.101.3.4.3.{20,22,24}
            PqcAlgorithm::SlhDsaSha2_128s => "2.16.840.1.101.3.4.3.20",
            PqcAlgorithm::SlhDsaSha2_192s => "2.16.840.1.101.3.4.3.22",
            PqcAlgorithm::SlhDsaSha2_256s => "2.16.840.1.101.3.4.3.24",
        }
    }

    /// Parameter-set sizes (bytes) per the relevant FIPS standard.
    ///
    /// Cite:
    /// - FIPS 203 (ML-KEM): 512 ⇒ pk=800 sk=1632 ct=768 ss=32;
    ///   768 ⇒ pk=1184 sk=2400 ct=1088 ss=32;
    ///   1024 ⇒ pk=1568 sk=3168 ct=1568 ss=32.
    /// - FIPS 204 (ML-DSA): 44 ⇒ pk=1312 sk=2560 sig=2420;
    ///   65 ⇒ pk=1952 sk=4032 sig=3309;
    ///   87 ⇒ pk=2592 sk=4896 sig=4627.
    /// - FIPS 205 (SLH-DSA-SHA2 small): 128s ⇒ pk=32 sk=64 sig=7856;
    ///   192s ⇒ pk=48 sk=96 sig=16224;
    ///   256s ⇒ pk=64 sk=128 sig=29792.
    pub fn sizes(self) -> PqcSizes {
        const SS: usize = 32; // ML-KEM shared-secret length is 32 bytes for all sets.
        match self {
            PqcAlgorithm::MlKem512 => PqcSizes {
                public_key_len: 800,
                secret_key_len: 1632,
                ciphertext_len: Some(768),
                shared_secret_len: Some(SS),
                signature_len: None,
            },
            PqcAlgorithm::MlKem768 => PqcSizes {
                public_key_len: 1184,
                secret_key_len: 2400,
                ciphertext_len: Some(1088),
                shared_secret_len: Some(SS),
                signature_len: None,
            },
            PqcAlgorithm::MlKem1024 => PqcSizes {
                public_key_len: 1568,
                secret_key_len: 3168,
                ciphertext_len: Some(1568),
                shared_secret_len: Some(SS),
                signature_len: None,
            },
            PqcAlgorithm::MlDsa44 => PqcSizes {
                public_key_len: 1312,
                secret_key_len: 2560,
                ciphertext_len: None,
                shared_secret_len: None,
                signature_len: Some(2420),
            },
            PqcAlgorithm::MlDsa65 => PqcSizes {
                public_key_len: 1952,
                secret_key_len: 4032,
                ciphertext_len: None,
                shared_secret_len: None,
                signature_len: Some(3309),
            },
            PqcAlgorithm::MlDsa87 => PqcSizes {
                public_key_len: 2592,
                secret_key_len: 4896,
                ciphertext_len: None,
                shared_secret_len: None,
                signature_len: Some(4627),
            },
            PqcAlgorithm::SlhDsaSha2_128s => PqcSizes {
                public_key_len: 32,
                secret_key_len: 64,
                ciphertext_len: None,
                shared_secret_len: None,
                signature_len: Some(7856),
            },
            PqcAlgorithm::SlhDsaSha2_192s => PqcSizes {
                public_key_len: 48,
                secret_key_len: 96,
                ciphertext_len: None,
                shared_secret_len: None,
                signature_len: Some(16224),
            },
            PqcAlgorithm::SlhDsaSha2_256s => PqcSizes {
                public_key_len: 64,
                secret_key_len: 128,
                ciphertext_len: None,
                shared_secret_len: None,
                signature_len: Some(29792),
            },
        }
    }
}

// ─── Pluggable engine traits ──────────────────────────────────────────────────

/// A key-encapsulation engine for a single [`PqcAlgorithm`] KEM (FIPS 203).
///
/// **Pluggable, not implemented here.** Real backends (e.g. the `fips203`
/// crate or `oqs-rs`) implement this trait; cave-core ships none. The byte
/// slices flowing through these methods are expected to match the lengths in
/// [`PqcAlgorithm::sizes`]; engines are responsible for enforcing that.
pub trait KemEngine {
    /// Engine-specific opaque error.
    type Error;

    /// Which KEM this engine implements. Implementations should return a
    /// [`PqcAlgorithm`] where [`PqcAlgorithm::is_kem`] is `true`.
    fn algorithm(&self) -> PqcAlgorithm;

    /// Generate a `(public_key, secret_key)` pair.
    fn generate_keypair(&self) -> Result<(Vec<u8>, Vec<u8>), Self::Error>;

    /// Encapsulate to `public_key`, returning `(ciphertext, shared_secret)`.
    fn encapsulate(&self, public_key: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Self::Error>;

    /// Decapsulate `ciphertext` with `secret_key`, returning the shared secret.
    fn decapsulate(&self, secret_key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, Self::Error>;
}

/// A digital-signature engine for a single [`PqcAlgorithm`] signature scheme
/// (FIPS 204 ML-DSA or FIPS 205 SLH-DSA).
///
/// **Pluggable, not implemented here.** Real backends (e.g. `fips204` /
/// `fips205` / `oqs-rs`) implement this trait; cave-core ships none.
pub trait SignatureEngine {
    /// Engine-specific opaque error.
    type Error;

    /// Which signature scheme this engine implements. Implementations should
    /// return a [`PqcAlgorithm`] where [`PqcAlgorithm::is_signature`] is `true`.
    fn algorithm(&self) -> PqcAlgorithm;

    /// Generate a `(public_key, secret_key)` pair.
    fn generate_keypair(&self) -> Result<(Vec<u8>, Vec<u8>), Self::Error>;

    /// Sign `message` with `secret_key`.
    fn sign(&self, secret_key: &[u8], message: &[u8]) -> Result<Vec<u8>, Self::Error>;

    /// Verify `signature` over `message` against `public_key`.
    fn verify(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<bool, Self::Error>;
}

// ─── Composite-signature container ────────────────────────────────────────────

/// Wire-format version byte for [`CompositeSignature`]. Layout, repeated per
/// component: `[0x01][u32 BE len][len bytes]...`.
pub const COMPOSITE_VERSION: u8 = 0x01;

/// A length-prefixed container holding one or more signature components.
///
/// This is the consolidated home of the composite container that previously
/// lived in `cave-certs/src/pqc.rs` (hybrid PQC + classical dual-sign). It is
/// algorithm-agnostic: it carries opaque component byte strings in order and
/// does not interpret them. A hybrid deployment typically packs
/// `[pqc_signature, classical_signature]`; a multi-PQC deployment can pack more.
///
/// Wire format (`assemble` / `parse`):
///
/// ```text
/// byte 0      : version (0x01)
/// for each component:
///   bytes     : u32 big-endian component length L
///   bytes     : L raw component bytes
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompositeSignature {
    /// Ordered signature components. Order is preserved across assemble/parse.
    pub components: Vec<Vec<u8>>,
}

impl CompositeSignature {
    /// Build a composite from owned components.
    pub fn new(components: Vec<Vec<u8>>) -> Self {
        Self { components }
    }

    /// Serialize to the length-prefixed wire format.
    ///
    /// Returns [`PqcError::ComponentTooLarge`] if any component exceeds the
    /// `u32` length field.
    pub fn assemble(&self) -> Result<Vec<u8>, PqcError> {
        let mut total = 1usize;
        for c in &self.components {
            if c.len() > u32::MAX as usize {
                return Err(PqcError::ComponentTooLarge { len: c.len() });
            }
            total += 4 + c.len();
        }
        let mut out = Vec::with_capacity(total);
        out.push(COMPOSITE_VERSION);
        for c in &self.components {
            out.extend_from_slice(&(c.len() as u32).to_be_bytes());
            out.extend_from_slice(c);
        }
        Ok(out)
    }

    /// Parse the length-prefixed wire format back into components.
    ///
    /// Errors:
    /// - [`PqcError::Truncated`] if the blob is empty or ends mid-field.
    /// - [`PqcError::VersionMismatch`] if the leading byte is not
    ///   [`COMPOSITE_VERSION`].
    pub fn parse(blob: &[u8]) -> Result<Self, PqcError> {
        if blob.is_empty() {
            return Err(PqcError::Truncated {
                expected: 1,
                actual: 0,
            });
        }
        if blob[0] != COMPOSITE_VERSION {
            return Err(PqcError::VersionMismatch {
                expected: COMPOSITE_VERSION,
                actual: blob[0],
            });
        }

        let mut components = Vec::new();
        let mut pos = 1usize;
        while pos < blob.len() {
            // Need 4 bytes for the length prefix.
            if pos + 4 > blob.len() {
                return Err(PqcError::Truncated {
                    expected: pos + 4,
                    actual: blob.len(),
                });
            }
            let len = u32::from_be_bytes(blob[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let end = pos + len;
            if end > blob.len() {
                return Err(PqcError::Truncated {
                    expected: end,
                    actual: blob.len(),
                });
            }
            components.push(blob[pos..end].to_vec());
            pos = end;
        }

        Ok(Self { components })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kem_classification() {
        for a in [
            PqcAlgorithm::MlKem512,
            PqcAlgorithm::MlKem768,
            PqcAlgorithm::MlKem1024,
        ] {
            assert!(a.is_kem(), "{} should be a KEM", a.name());
            assert!(!a.is_signature(), "{} should not be a signature", a.name());
        }
    }

    #[test]
    fn signature_classification() {
        for a in [
            PqcAlgorithm::MlDsa44,
            PqcAlgorithm::MlDsa65,
            PqcAlgorithm::MlDsa87,
            PqcAlgorithm::SlhDsaSha2_128s,
            PqcAlgorithm::SlhDsaSha2_192s,
            PqcAlgorithm::SlhDsaSha2_256s,
        ] {
            assert!(a.is_signature(), "{} should be a signature", a.name());
            assert!(!a.is_kem(), "{} should not be a KEM", a.name());
        }
    }

    #[test]
    fn ml_kem_768_sizes() {
        let s = PqcAlgorithm::MlKem768.sizes();
        assert_eq!(s.public_key_len, 1184);
        assert_eq!(s.secret_key_len, 2400);
        assert_eq!(s.ciphertext_len, Some(1088));
        assert_eq!(s.shared_secret_len, Some(32));
        assert_eq!(s.signature_len, None);
    }

    #[test]
    fn ml_dsa_65_sizes() {
        let s = PqcAlgorithm::MlDsa65.sizes();
        assert_eq!(s.public_key_len, 1952);
        assert_eq!(s.secret_key_len, 4032);
        assert_eq!(s.signature_len, Some(3309));
        assert_eq!(s.ciphertext_len, None);
        assert_eq!(s.shared_secret_len, None);
    }

    #[test]
    fn slh_dsa_128s_sizes() {
        let s = PqcAlgorithm::SlhDsaSha2_128s.sizes();
        assert_eq!(s.public_key_len, 32);
        assert_eq!(s.secret_key_len, 64);
        assert_eq!(s.signature_len, Some(7856));
        assert_eq!(s.ciphertext_len, None);
        assert_eq!(s.shared_secret_len, None);
    }

    #[test]
    fn name_roundtrip_all() {
        for a in PqcAlgorithm::ALL {
            let n = a.name();
            assert_eq!(
                PqcAlgorithm::from_name(n),
                Some(a),
                "name roundtrip failed for {n}"
            );
        }
        // Spot-check exact canonical spellings.
        assert_eq!(PqcAlgorithm::MlKem768.name(), "ML-KEM-768");
        assert_eq!(PqcAlgorithm::MlDsa65.name(), "ML-DSA-65");
        assert_eq!(PqcAlgorithm::SlhDsaSha2_128s.name(), "SLH-DSA-SHA2-128s");
    }

    #[test]
    fn name_roundtrip_unknown() {
        assert_eq!(PqcAlgorithm::from_name("RSA-2048"), None);
        assert_eq!(PqcAlgorithm::from_name("ml-kem-768"), None); // case sensitive
        assert_eq!(PqcAlgorithm::from_name(""), None);
    }

    #[test]
    fn oid_distinct_per_algorithm() {
        let mut seen = std::collections::HashSet::new();
        for a in PqcAlgorithm::ALL {
            let oid = a.oid();
            assert!(oid.starts_with("2.16.840.1.101.3.4."), "{oid}");
            assert!(seen.insert(oid), "duplicate OID {oid}");
        }
        assert_eq!(seen.len(), PqcAlgorithm::ALL.len());
    }

    #[test]
    fn kem_sizes_none_for_signature_algos() {
        // Every signature algorithm has no KEM fields.
        for a in PqcAlgorithm::ALL.into_iter().filter(|a| a.is_signature()) {
            let s = a.sizes();
            assert_eq!(s.ciphertext_len, None, "{}", a.name());
            assert_eq!(s.shared_secret_len, None, "{}", a.name());
            assert!(s.signature_len.is_some(), "{}", a.name());
        }
    }

    #[test]
    fn sig_sizes_none_for_kem_algos() {
        // Every KEM algorithm has no signature field but has KEM fields.
        for a in PqcAlgorithm::ALL.into_iter().filter(|a| a.is_kem()) {
            let s = a.sizes();
            assert_eq!(s.signature_len, None, "{}", a.name());
            assert!(s.ciphertext_len.is_some(), "{}", a.name());
            assert_eq!(s.shared_secret_len, Some(32), "{}", a.name());
        }
    }

    #[test]
    fn composite_version_constant_is_one() {
        assert_eq!(COMPOSITE_VERSION, 0x01);
    }

    #[test]
    fn composite_assemble_parse_roundtrip() {
        // Typical hybrid layout: [pqc_sig, classical_sig].
        let pqc = vec![0xABu8; 3309];
        let classical = vec![0xCDu8; 64];
        let original = CompositeSignature::new(vec![pqc.clone(), classical.clone()]);

        let blob = original.assemble().unwrap();
        // Layout sanity: version byte + per-component (4-byte len + payload).
        assert_eq!(blob[0], COMPOSITE_VERSION);
        assert_eq!(blob.len(), 1 + (4 + 3309) + (4 + 64));

        let parsed = CompositeSignature::parse(&blob).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(parsed.components[0], pqc);
        assert_eq!(parsed.components[1], classical);
    }

    #[test]
    fn composite_empty_components_roundtrip() {
        let original = CompositeSignature::default();
        let blob = original.assemble().unwrap();
        assert_eq!(blob, vec![COMPOSITE_VERSION]);
        let parsed = CompositeSignature::parse(&blob).unwrap();
        assert_eq!(parsed.components.len(), 0);
        assert_eq!(parsed, original);
    }

    #[test]
    fn composite_three_components_roundtrip() {
        let original = CompositeSignature::new(vec![
            b"first".to_vec(),
            Vec::new(), // zero-length component must survive
            b"third-component-bytes".to_vec(),
        ]);
        let blob = original.assemble().unwrap();
        let parsed = CompositeSignature::parse(&blob).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(parsed.components[1].len(), 0);
    }

    #[test]
    fn composite_rejects_empty_blob() {
        let err = CompositeSignature::parse(&[]).unwrap_err();
        assert_eq!(
            err,
            PqcError::Truncated {
                expected: 1,
                actual: 0
            }
        );
    }

    #[test]
    fn composite_rejects_bad_version() {
        let blob = [0x02u8, 0, 0, 0, 0];
        let err = CompositeSignature::parse(&blob).unwrap_err();
        assert_eq!(
            err,
            PqcError::VersionMismatch {
                expected: 0x01,
                actual: 0x02
            }
        );
    }

    #[test]
    fn composite_rejects_truncated_length_prefix() {
        // Version byte present, but the 4-byte length prefix is incomplete.
        let blob = [COMPOSITE_VERSION, 0x00, 0x00]; // only 2 of 4 length bytes
        let err = CompositeSignature::parse(&blob).unwrap_err();
        assert_eq!(
            err,
            PqcError::Truncated {
                expected: 5,
                actual: 3
            }
        );
    }

    #[test]
    fn composite_rejects_truncated_payload() {
        // Declares a 10-byte component but only supplies 3 payload bytes.
        let mut blob = vec![COMPOSITE_VERSION];
        blob.extend_from_slice(&10u32.to_be_bytes());
        blob.extend_from_slice(&[1, 2, 3]);
        let err = CompositeSignature::parse(&blob).unwrap_err();
        assert_eq!(
            err,
            PqcError::Truncated {
                expected: 5 + 10,
                actual: 5 + 3
            }
        );
    }
}
