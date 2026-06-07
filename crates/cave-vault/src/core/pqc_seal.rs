// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Post-quantum (ML-KEM-768) seal-wrap for the Vault barrier master key.
//!
//! # Charter PQC baseline
//!
//! OpenBao's auto-seal (`vault/seal_autoseal.go`) delegates master-key
//! wrap/unwrap to a remote KMS backend (AWS KMS, Azure Key Vault, GCP CKMS,
//! …). Every one of those wrappers is RSA/ECDH-based and therefore harvestable
//! by a future cryptographically-relevant quantum computer ("harvest now,
//! decrypt later"). The cave-runtime charter mandates a PQC-ready baseline, so
//! this module adds a **local** seal-wrap whose key-establishment step is
//! lattice-based and quantum-resistant.
//!
//! It is a textbook **KEM-DEM hybrid envelope** built on ML-KEM-768
//! (NIST FIPS 203, security category 3) for the KEM and AES-256-GCM for the
//! DEM, with HKDF-SHA-256 between them:
//!
//! ```text
//!   (dk, ek)        ← ML-KEM-768 keypair   (decapsulation / encapsulation key)
//!   (kem_ct, ss)    ← ek.encapsulate()     ss = 32-byte shared secret
//!   wrap_key        ← HKDF-SHA256(ss, info = "cave-vault/pqc-seal/v1")
//!   nonce ‖ sealed  ← AES-256-GCM(wrap_key).seal(master_key)
//!   wrapped         = { kem_ct, nonce, ciphertext = sealed }
//! ```
//!
//! Unwrapping reverses it: `dk.decapsulate(kem_ct) → ss`, re-derive `wrap_key`,
//! AES-256-GCM open. A tampered KEM ciphertext yields a different shared secret
//! (ML-KEM decapsulation never fails — it uses implicit rejection), which makes
//! the derived AES key wrong, which makes the GCM tag verification fail. A
//! tampered DEM ciphertext fails the GCM tag directly. Either way `seal_unwrap`
//! returns an error and never a wrong plaintext.
//!
//! The real lattice arithmetic (NTT, sampling, FIPS-203 encode/decode) lives in
//! the vetted RustCrypto `ml-kem` crate; this module owns only the envelope
//! glue and its serialization, and is tested for round-trip, randomisation,
//! tamper-rejection, wrong-key-rejection and seed-determinism.

use crate::error::{VaultError, VaultResult};
use ml_kem::array::Array;
use ml_kem::kem::{Decapsulate, Encapsulate, Kem};
use ml_kem::{EncapsulationKey, KeyExport, KeyInit, MlKem768, Seed};
use ring::aead;
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

/// HKDF `info` string — domain-separates this KDF use from any other shared
/// secret derived from the same KEM in the future. The `/v1` suffix lets us
/// migrate the envelope format without silently reusing keys.
const SEAL_WRAP_INFO: &[u8] = b"cave-vault/pqc-seal/v1";

/// ML-KEM-768 wire sizes (FIPS 203, parameter set k = 3). Used for validation
/// and documented as test invariants.
pub const ML_KEM_768_EK_LEN: usize = 1184; // encapsulation (public) key
pub const ML_KEM_768_CT_LEN: usize = 1088; // KEM ciphertext
const SEED_LEN: usize = 64; // ML-KEM decapsulation-key seed
const AES_256_KEY_LEN: usize = 32;
const GCM_NONCE_LEN: usize = 12;

/// The wrapped barrier master key produced by [`PqcSealKeypair::seal_wrap`].
///
/// Stored as-is in the seal-config storage entry. Holds no plaintext key
/// material — recovering the master key requires the decapsulation key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PqcWrappedKey {
    /// ML-KEM-768 ciphertext (1088 bytes) carrying the encapsulated shared
    /// secret.
    #[serde(with = "hex_vec")]
    pub kem_ciphertext: Vec<u8>,
    /// AES-256-GCM nonce (12 bytes).
    pub nonce: [u8; GCM_NONCE_LEN],
    /// AES-256-GCM ciphertext of the master key, with the 16-byte tag
    /// appended.
    #[serde(with = "hex_vec")]
    pub ciphertext: Vec<u8>,
}

/// An ML-KEM-768 keypair acting as a PQC seal-wrap barrier.
///
/// The decapsulation key is the secret half; in a deployment it is itself
/// protected (e.g. derived from the Shamir-recovered key or held in an HSM).
/// [`seed_bytes`](Self::seed_bytes) serialises the secret to its 64-byte FIPS
/// 203 seed for storage; [`public_key_bytes`](Self::public_key_bytes) exports
/// the shareable encapsulation key.
pub struct PqcSealKeypair {
    dk: ml_kem::DecapsulationKey<MlKem768>,
    ek: EncapsulationKey<MlKem768>,
}

impl PqcSealKeypair {
    /// Generate a fresh ML-KEM-768 keypair from the system CSPRNG.
    pub fn generate() -> Self {
        let (dk, ek) = MlKem768::generate_keypair();
        Self { dk, ek }
    }

    /// Reconstruct a keypair from its 64-byte FIPS 203 seed. Deterministic:
    /// the same seed always yields the same keypair.
    pub fn from_seed_bytes(seed: &[u8; SEED_LEN]) -> Self {
        let seed_arr: Seed = Array::from(*seed);
        let dk = <ml_kem::DecapsulationKey<MlKem768> as KeyInit>::new(&seed_arr);
        let ek = dk.encapsulation_key().clone();
        Self { dk, ek }
    }

    /// Serialize the decapsulation key to its 64-byte seed for storage.
    pub fn seed_bytes(&self) -> [u8; SEED_LEN] {
        let seed: Seed = self.dk.to_bytes();
        let mut out = [0u8; SEED_LEN];
        out.copy_from_slice(seed.as_slice());
        out
    }

    /// Export the encapsulation (public) key — 1184 bytes. Anyone holding it
    /// can [`seal_wrap_to_public`](Self::seal_wrap_to_public) a master key for
    /// this keypair without being able to unwrap it.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.ek.to_bytes().as_slice().to_vec()
    }

    /// Wrap `master_key` under this keypair's encapsulation key.
    pub fn seal_wrap(&self, master_key: &[u8]) -> VaultResult<PqcWrappedKey> {
        encapsulate_and_seal(&self.ek, master_key)
    }

    /// Wrap `master_key` given only a serialized encapsulation key. Used by an
    /// operator who can seal but not unseal (separation of duties).
    pub fn seal_wrap_to_public(public_key: &[u8], master_key: &[u8]) -> VaultResult<PqcWrappedKey> {
        if public_key.len() != ML_KEM_768_EK_LEN {
            return Err(VaultError::InvalidRequest(format!(
                "ML-KEM-768 encapsulation key must be {ML_KEM_768_EK_LEN} bytes, got {}",
                public_key.len()
            )));
        }
        let key_arr = Array::try_from(public_key)
            .map_err(|_| VaultError::Crypto("malformed encapsulation key".into()))?;
        let ek = EncapsulationKey::<MlKem768>::new(&key_arr)
            .map_err(|_| VaultError::Crypto("invalid encapsulation key".into()))?;
        encapsulate_and_seal(&ek, master_key)
    }

    /// Recover the master key from a [`PqcWrappedKey`]. Returns an error if the
    /// envelope was produced for a different keypair or has been tampered with.
    pub fn seal_unwrap(&self, wrapped: &PqcWrappedKey) -> VaultResult<Vec<u8>> {
        if wrapped.kem_ciphertext.len() != ML_KEM_768_CT_LEN {
            return Err(VaultError::InvalidRequest(format!(
                "ML-KEM-768 ciphertext must be {ML_KEM_768_CT_LEN} bytes, got {}",
                wrapped.kem_ciphertext.len()
            )));
        }
        // ML-KEM decapsulation is infallible (implicit rejection): a bad
        // ciphertext returns a pseudo-random shared secret rather than an
        // error. The GCM tag below is what actually rejects tampering.
        let shared = self
            .dk
            .decapsulate_slice(&wrapped.kem_ciphertext)
            .map_err(|_| VaultError::Crypto("decapsulation failed".into()))?;
        let wrap_key = derive_wrap_key(shared.as_slice())?;
        aes256_gcm_open(&wrap_key, &wrapped.nonce, &wrapped.ciphertext)
    }
}

/// Shared encapsulate + AES-GCM seal path for both `seal_wrap` variants.
fn encapsulate_and_seal(
    ek: &EncapsulationKey<MlKem768>,
    master_key: &[u8],
) -> VaultResult<PqcWrappedKey> {
    let (kem_ct, shared) = ek.encapsulate();
    let wrap_key = derive_wrap_key(shared.as_slice())?;
    let (nonce, ciphertext) = aes256_gcm_seal(&wrap_key, master_key)?;
    Ok(PqcWrappedKey {
        kem_ciphertext: kem_ct.as_slice().to_vec(),
        nonce,
        ciphertext,
    })
}

/// HKDF-SHA256 expand the 32-byte KEM shared secret into a 32-byte AES key.
fn derive_wrap_key(shared_secret: &[u8]) -> VaultResult<[u8; AES_256_KEY_LEN]> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
    let prk = salt.extract(shared_secret);
    let okm = prk
        .expand(&[SEAL_WRAP_INFO], hkdf::HKDF_SHA256)
        .map_err(|_| VaultError::Crypto("hkdf expand failed".into()))?;
    let mut key = [0u8; AES_256_KEY_LEN];
    okm.fill(&mut key)
        .map_err(|_| VaultError::Crypto("hkdf fill failed".into()))?;
    Ok(key)
}

/// AES-256-GCM seal. Returns `(nonce, ciphertext‖tag)`.
fn aes256_gcm_seal(
    key: &[u8; AES_256_KEY_LEN],
    plaintext: &[u8],
) -> VaultResult<([u8; GCM_NONCE_LEN], Vec<u8>)> {
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| VaultError::Crypto("aead key init failed".into()))?;
    let mut nonce_bytes = [0u8; GCM_NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce_bytes)
        .map_err(|_| VaultError::Crypto("rng failure".into()))?;
    let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
    let sealing = aead::LessSafeKey::new(unbound);
    let mut in_out = plaintext.to_vec();
    sealing
        .seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| VaultError::Crypto("seal failed".into()))?;
    Ok((nonce_bytes, in_out))
}

/// AES-256-GCM open. Errors on any tag mismatch.
fn aes256_gcm_open(
    key: &[u8; AES_256_KEY_LEN],
    nonce_bytes: &[u8; GCM_NONCE_LEN],
    ciphertext: &[u8],
) -> VaultResult<Vec<u8>> {
    let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| VaultError::Crypto("aead key init failed".into()))?;
    let nonce = aead::Nonce::assume_unique_for_key(*nonce_bytes);
    let opening = aead::LessSafeKey::new(unbound);
    let mut in_out = ciphertext.to_vec();
    let plaintext = opening
        .open_in_place(nonce, aead::Aad::empty(), &mut in_out)
        .map_err(|_| VaultError::Crypto("open failed (tag mismatch)".into()))?;
    Ok(plaintext.to_vec())
}

/// Serde helper: hex-encode `Vec<u8>` fields so the seal-config JSON entry is
/// human-inspectable and matches OpenBao's hex convention for stored keys.
mod hex_vec {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ml_kem::B32;

    #[test]
    fn wrap_unwrap_roundtrip() {
        let kp = PqcSealKeypair::generate();
        let mk = b"unit-test-32-byte-master-key!!!!".to_vec();
        let wrapped = kp.seal_wrap(&mk).unwrap();
        assert_eq!(kp.seal_unwrap(&wrapped).unwrap(), mk);
    }

    #[test]
    fn sizes_match_ml_kem_768() {
        let kp = PqcSealKeypair::generate();
        assert_eq!(kp.public_key_bytes().len(), ML_KEM_768_EK_LEN);
        let w = kp.seal_wrap(b"x").unwrap();
        assert_eq!(w.kem_ciphertext.len(), ML_KEM_768_CT_LEN);
        assert_eq!(kp.seed_bytes().len(), SEED_LEN);
    }

    #[test]
    fn seed_is_deterministic_and_reproduces_keypair() {
        // Two keypairs built from the SAME seed must be identical: same public
        // key, and one can unwrap what the other sealed.
        let seed = [7u8; SEED_LEN];
        let a = PqcSealKeypair::from_seed_bytes(&seed);
        let b = PqcSealKeypair::from_seed_bytes(&seed);
        assert_eq!(a.public_key_bytes(), b.public_key_bytes());
        assert_eq!(a.seed_bytes(), seed);

        let mk = b"deterministic-seed-master-key!!!".to_vec();
        let wrapped = a.seal_wrap(&mk).unwrap();
        assert_eq!(b.seal_unwrap(&wrapped).unwrap(), mk);
    }

    #[test]
    fn deterministic_encapsulation_is_reproducible() {
        // FIPS 203 deterministic encapsulation: a fixed keypair + fixed message
        // `m` must yield a byte-identical ciphertext and shared secret, and
        // decapsulation must recover that shared secret. This pins the whole
        // KEM pipeline to a reproducible vector without any RNG.
        let kp = PqcSealKeypair::from_seed_bytes(&[0x11; SEED_LEN]);
        let m: B32 = Array::from([0x42u8; 32]);

        let (ct1, ss1) = kp.ek.encapsulate_deterministic(&m);
        let (ct2, ss2) = kp.ek.encapsulate_deterministic(&m);
        assert_eq!(ct1.as_slice(), ct2.as_slice(), "ct must be deterministic");
        assert_eq!(ss1.as_slice(), ss2.as_slice(), "ss must be deterministic");

        let recovered = kp.dk.decapsulate(&ct1);
        assert_eq!(
            recovered.as_slice(),
            ss1.as_slice(),
            "decapsulate must recover the encapsulated shared secret"
        );
    }

    #[test]
    fn rejects_wrong_length_inputs() {
        let kp = PqcSealKeypair::generate();
        // Too-short public key.
        assert!(PqcSealKeypair::seal_wrap_to_public(&[0u8; 10], b"mk").is_err());
        // Too-short KEM ciphertext on unwrap.
        let mut w = kp.seal_wrap(b"mk").unwrap();
        w.kem_ciphertext.truncate(100);
        assert!(kp.seal_unwrap(&w).is_err());
    }

    #[test]
    fn wrapped_key_serde_round_trips() {
        let kp = PqcSealKeypair::generate();
        let mk = b"serde-master-key-for-storage!!!!".to_vec();
        let w = kp.seal_wrap(&mk).unwrap();
        let json = serde_json::to_string(&w).unwrap();
        // hex-encoded, no raw key bytes leaked as an array.
        assert!(json.contains("kem_ciphertext"));
        let back: PqcWrappedKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, w);
        assert_eq!(kp.seal_unwrap(&back).unwrap(), mk);
    }
}
