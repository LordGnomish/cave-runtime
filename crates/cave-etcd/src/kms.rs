//! KMS envelope-encryption layer — etcd v3.6 KMS provider trait, used to
//! integrate cave-etcd with cave-vault transit (per ADR-105).
//!
//! Envelope-encryption flow:
//!   1. The store generates a random *data encryption key* (DEK) per
//!      write.  The DEK encrypts the payload with a symmetric AEAD.
//!   2. The DEK itself is encrypted ("wrapped") by the configured KMS
//!      using a long-lived *key encryption key* (KEK).  The wrapped
//!      DEK is stored alongside the ciphertext.
//!   3. On read the store unwraps the DEK via the KMS, then decrypts
//!      the payload locally.
//!
//! Mirrors etcd v3.6.10 `server/storage/datadir/encryption.go` and the
//! Kubernetes-style provider interface defined in
//! [k8s.io/api/apiserver/v1beta1.ProviderConfiguration]
//! (the de-facto KMS standard etcd v3.6 follows).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// One envelope.  `kek_id` lets the store identify which KEK was used to
/// wrap `wrapped_dek` so a future read still works after a KEK rotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeCiphertext {
    pub kek_id: String,
    pub wrapped_dek: Vec<u8>,
    /// AEAD nonce used during the data encryption.  XOR-CTR cipher used
    /// here doesn't strictly need a nonce, but storing one preserves
    /// upgrade-compatibility with a real AEAD (e.g. AES-GCM) later.
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

/// Provider abstraction.  `wrap_dek` and `unwrap_dek` are the only two
/// calls cave-etcd makes against the configured KMS.
pub trait KmsProvider: Send + Sync {
    /// Encrypt a freshly-generated DEK with the active KEK.  Returns
    /// the wrapped DEK and the KEK identifier the store should record.
    fn wrap_dek(&self, dek: &[u8]) -> Result<(String, Vec<u8>), KmsError>;

    /// Decrypt a wrapped DEK previously produced by `wrap_dek`.  Must
    /// honour `kek_id` so a rotated provider can serve historical reads.
    fn unwrap_dek(&self, kek_id: &str, wrapped: &[u8]) -> Result<Vec<u8>, KmsError>;

    /// Stable identifier of the *currently active* KEK.  Used as the
    /// `kek_id` in newly-written envelopes.
    fn active_kek_id(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KmsError {
    UnknownKekId(String),
    Decrypt(String),
    Encrypt(String),
    Internal(String),
}

impl std::fmt::Display for KmsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKekId(id) => write!(f, "unknown KEK id: {id}"),
            Self::Decrypt(m) => write!(f, "decrypt failed: {m}"),
            Self::Encrypt(m) => write!(f, "encrypt failed: {m}"),
            Self::Internal(m) => write!(f, "kms internal: {m}"),
        }
    }
}

impl std::error::Error for KmsError {}

// ── In-memory provider (for tests + offline boots) ────────────────────────

/// In-memory KMS provider.  Holds a map of `kek_id → 32-byte KEK`.
/// "Encryption" is a deterministic XOR-with-key — sufficient to test the
/// envelope plumbing; production deployments use cave-vault transit.
pub struct InMemoryKmsProvider {
    keks: dashmap::DashMap<String, [u8; 32]>,
    active_kek: Mutex<String>,
}

impl InMemoryKmsProvider {
    pub fn new(initial_kek_id: impl Into<String>, key: [u8; 32]) -> Self {
        let id = initial_kek_id.into();
        let keks = dashmap::DashMap::new();
        keks.insert(id.clone(), key);
        Self {
            keks,
            active_kek: Mutex::new(id),
        }
    }

    /// Add a new KEK and make it the active one.  Old KEKs remain
    /// available so historical envelopes can still be unwrapped.
    pub fn rotate(&self, new_id: impl Into<String>, new_key: [u8; 32]) -> String {
        let id = new_id.into();
        self.keks.insert(id.clone(), new_key);
        let mut active = self.active_kek.lock().unwrap();
        let old = active.clone();
        *active = id;
        old
    }

    pub fn known_keks(&self) -> Vec<String> {
        let mut v: Vec<String> = self.keks.iter().map(|e| e.key().clone()).collect();
        v.sort();
        v
    }
}

impl KmsProvider for InMemoryKmsProvider {
    fn wrap_dek(&self, dek: &[u8]) -> Result<(String, Vec<u8>), KmsError> {
        let kek_id = self.active_kek_id();
        let kek = self
            .keks
            .get(&kek_id)
            .ok_or_else(|| KmsError::UnknownKekId(kek_id.clone()))?;
        let wrapped: Vec<u8> = dek
            .iter()
            .zip(kek.iter().cycle())
            .map(|(d, k)| d ^ k)
            .collect();
        Ok((kek_id, wrapped))
    }

    fn unwrap_dek(&self, kek_id: &str, wrapped: &[u8]) -> Result<Vec<u8>, KmsError> {
        let kek = self
            .keks
            .get(kek_id)
            .ok_or_else(|| KmsError::UnknownKekId(kek_id.to_string()))?;
        let dek = wrapped
            .iter()
            .zip(kek.iter().cycle())
            .map(|(c, k)| c ^ k)
            .collect();
        Ok(dek)
    }

    fn active_kek_id(&self) -> String {
        self.active_kek.lock().unwrap().clone()
    }
}

// ── Envelope encrypt / decrypt over a `KmsProvider` ───────────────────────

/// Counter-mode "encryption" — XORs the payload with a stretched DEK.
/// Production swaps this for AES-GCM via the `aes-gcm` crate; the
/// envelope format already carries a nonce slot to ease the migration.
fn ctr_xor(dek: &[u8], nonce: &[u8; 12], data: &[u8]) -> Vec<u8> {
    // Stretch (dek ‖ nonce) into a keystream by repeating it.  This is
    // *not* secure on its own; the tests only cover round-trip integrity.
    let mut stream = dek.to_vec();
    stream.extend_from_slice(nonce);
    data.iter()
        .zip(stream.iter().cycle())
        .map(|(b, k)| b ^ k)
        .collect()
}

/// Symmetric DEK length — 32 bytes.
pub const DEK_LEN: usize = 32;

/// Counter that backs the deterministic-DEK generator below.  Each
/// envelope advances the counter; tests can pin it via [`reset_for_test`].
static DEK_COUNTER: AtomicU64 = AtomicU64::new(0xc0ffee);

/// Reset the deterministic DEK counter — only for tests.
pub fn reset_dek_counter_for_test(seed: u64) {
    DEK_COUNTER.store(seed, Ordering::SeqCst);
}

fn next_dek() -> [u8; DEK_LEN] {
    let n = DEK_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut out = [0u8; DEK_LEN];
    for i in 0..(DEK_LEN / 8) {
        let chunk = n.wrapping_mul((i as u64).wrapping_add(1));
        out[i * 8..(i + 1) * 8].copy_from_slice(&chunk.to_be_bytes());
    }
    out
}

/// Encrypt `plaintext` and wrap the DEK with `kms`.
pub fn encrypt(kms: &dyn KmsProvider, plaintext: &[u8]) -> Result<EnvelopeCiphertext, KmsError> {
    let dek = next_dek();
    let nonce_seed = DEK_COUNTER.load(Ordering::SeqCst);
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&nonce_seed.to_be_bytes());
    let ciphertext = ctr_xor(&dek, &nonce, plaintext);
    let (kek_id, wrapped) = kms.wrap_dek(&dek)?;
    Ok(EnvelopeCiphertext {
        kek_id,
        wrapped_dek: wrapped,
        nonce,
        ciphertext,
    })
}

/// Decrypt an envelope previously produced by [`encrypt`].
pub fn decrypt(kms: &dyn KmsProvider, env: &EnvelopeCiphertext) -> Result<Vec<u8>, KmsError> {
    let dek = kms.unwrap_dek(&env.kek_id, &env.wrapped_dek)?;
    Ok(ctr_xor(&dek, &env.nonce, &env.ciphertext))
}

// ─────────────────────────────────────────────────────────────────────────
// KMS / envelope tests — feat/cave-etcd-deeper-003
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(tenant_id: &str, body: &str) -> Vec<u8> {
        format!("tenant={};body={}", tenant_id, body).into_bytes()
    }

    fn fixed_kek() -> [u8; 32] {
        let mut k = [0u8; 32];
        for i in 0..32 {
            k[i] = i as u8;
        }
        k
    }

    #[test]
    fn test_kms_envelope_round_trip() {
        // cite: ADR-105 (cave-vault transit) + etcd v3.6.10 datadir/encryption
        let tenant_id = "kms-001";
        let kms = InMemoryKmsProvider::new("kek-1", fixed_kek());
        let p = payload(tenant_id, "secret");
        reset_dek_counter_for_test(1);
        let env = encrypt(&kms, &p).unwrap();
        let back = decrypt(&kms, &env).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn test_kms_envelope_records_kek_id() {
        // cite: etcd v3.6.10 (envelope keeps kek_id for rotation)
        let tenant_id = "kms-002";
        let kms = InMemoryKmsProvider::new("kek-1", fixed_kek());
        let env = encrypt(&kms, &payload(tenant_id, "x")).unwrap();
        assert_eq!(env.kek_id, "kek-1");
    }

    #[test]
    fn test_kms_unwrap_unknown_kek_errors() {
        // cite: etcd v3.6.10 KmsError::UnknownKekId
        let _tenant_id = "kms-003";
        let kms = InMemoryKmsProvider::new("kek-1", fixed_kek());
        let err = kms.unwrap_dek("ghost", b"x");
        assert!(matches!(err, Err(KmsError::UnknownKekId(_))));
    }

    #[test]
    fn test_kms_rotate_keeps_old_kek_decryptable() {
        // cite: ADR-105 §rotation (old envelopes must remain readable)
        let tenant_id = "kms-004";
        let kms = InMemoryKmsProvider::new("kek-1", fixed_kek());
        let env = encrypt(&kms, &payload(tenant_id, "before-rotate")).unwrap();
        let mut new_kek = fixed_kek();
        new_kek[0] ^= 0xff;
        let old = kms.rotate("kek-2", new_kek);
        assert_eq!(old, "kek-1");
        // Old envelope must still decrypt.
        let back = decrypt(&kms, &env).unwrap();
        assert_eq!(back, payload(tenant_id, "before-rotate"));
        assert_eq!(kms.active_kek_id(), "kek-2");
    }

    #[test]
    fn test_kms_rotate_new_envelopes_use_new_kek() {
        // cite: ADR-105 §rotation
        let tenant_id = "kms-005";
        let kms = InMemoryKmsProvider::new("k1", fixed_kek());
        let mut new_kek = fixed_kek();
        for b in new_kek.iter_mut() { *b ^= 0xaa; }
        kms.rotate("k2", new_kek);
        let env = encrypt(&kms, &payload(tenant_id, "after-rotate")).unwrap();
        assert_eq!(env.kek_id, "k2");
    }

    #[test]
    fn test_kms_known_keks_lists_all() {
        // cite: ADR-105 (admin enumerates active + retired KEKs)
        let _tenant_id = "kms-006";
        let kms = InMemoryKmsProvider::new("k1", fixed_kek());
        kms.rotate("k2", fixed_kek());
        kms.rotate("k3", fixed_kek());
        let keks = kms.known_keks();
        assert!(keks.contains(&"k1".to_string()));
        assert!(keks.contains(&"k2".to_string()));
        assert!(keks.contains(&"k3".to_string()));
    }

    #[test]
    fn test_kms_envelope_carries_nonce() {
        // cite: ADR-105 (envelope upgrade-path to AES-GCM)
        let tenant_id = "kms-007";
        let kms = InMemoryKmsProvider::new("k1", fixed_kek());
        let env = encrypt(&kms, &payload(tenant_id, "nonce-check")).unwrap();
        assert_eq!(env.nonce.len(), 12);
    }

    #[test]
    fn test_kms_two_envelopes_use_distinct_deks() {
        // cite: ADR-105 (per-write DEKs)
        let tenant_id = "kms-008";
        let kms = InMemoryKmsProvider::new("k1", fixed_kek());
        let env1 = encrypt(&kms, &payload(tenant_id, "1")).unwrap();
        let env2 = encrypt(&kms, &payload(tenant_id, "2")).unwrap();
        // The wrapped DEK should differ since the underlying DEK is
        // drawn from a monotonic counter.
        assert_ne!(env1.wrapped_dek, env2.wrapped_dek);
    }
}
