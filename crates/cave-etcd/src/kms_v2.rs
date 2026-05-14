// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KMSv2 binary-envelope layer — extends [`crate::kms`] with the wire-format,
//! AAD-binding, and DEK caching that Kubernetes' KMSv2 (KEP-3299) and etcd
//! v3.6's encryption-at-rest feature both rely on.
//!
//! The base [`crate::kms`] module provides the provider trait and a JSON-shaped
//! `EnvelopeCiphertext`.  This module adds:
//!
//!   * a compact **binary envelope** (`encode_envelope` / `decode_envelope`),
//!   * **additional authenticated data (AAD)** — bind a ciphertext to a tenant
//!     so cross-tenant replay fails,
//!   * a **DEK cache** with capacity + TTL, modelled on
//!     `kubernetes.io/staging/src/k8s.io/kms/pkg/encrypt/aes/cache.go`,
//!   * a **rotate** helper that re-encrypts an envelope under the active KEK.
//!
//! Mirrors etcd v3.6.10
//!   `server/storage/datadir/encryption.go` (envelope persistence) and
//!   `vendor/k8s.io/apiserver/pkg/storage/value/encrypt/envelope/kmsv2/envelope.go`.

use crate::kms::{KmsError, KmsProvider, DEK_LEN};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Magic prefix used by all KMSv2 envelopes ("k2").
pub const KMSV2_MAGIC: [u8; 2] = [0x6B, 0x32];
/// Wire-format version.
pub const KMSV2_VERSION: u8 = 0x01;
/// AEAD nonce length — fixed at 12 bytes to match AES-GCM.
pub const NONCE_LEN: usize = 12;
/// Authenticated-encryption tag length (matches AES-GCM 128-bit tag).
pub const TAG_LEN: usize = 16;

// ── Errors ────────────────────────────────────────────────────────────────

/// Errors specific to KMSv2 envelope handling.  Wraps base [`KmsError`] for
/// provider failures.
#[derive(Debug, PartialEq, Eq)]
pub enum KmsV2Error {
    /// The envelope was shorter than its declared layout.
    Truncated,
    /// The leading magic bytes did not match [`KMSV2_MAGIC`].
    BadMagic,
    /// We don't understand this wire-format version.
    UnsupportedVersion(u8),
    /// AEAD authentication failed — wrong key, wrong AAD, or tampered bytes.
    Authentication,
    /// Length fields would overflow on encode.
    LengthOverflow,
    /// The configured `KmsProvider` failed.
    Provider(KmsError),
    /// `kek_id` recorded in the envelope is unknown to the active provider.
    UnknownKekId(String),
}

impl std::fmt::Display for KmsV2Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "envelope truncated"),
            Self::BadMagic => write!(f, "bad KMSv2 magic"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported version {v}"),
            Self::Authentication => write!(f, "authentication failure"),
            Self::LengthOverflow => write!(f, "length overflow"),
            Self::Provider(e) => write!(f, "kms provider: {e}"),
            Self::UnknownKekId(id) => write!(f, "unknown kek id: {id}"),
        }
    }
}

impl std::error::Error for KmsV2Error {}

impl From<KmsError> for KmsV2Error {
    fn from(e: KmsError) -> Self { Self::Provider(e) }
}

// ── Envelope codec ────────────────────────────────────────────────────────

/// Binary-envelope layout (network byte order):
///
/// ```text
///   magic[2] | version[1] | kek_id_len[1] | kek_id[N]
///   nonce[12]
///   wrapped_dek_len[2 BE] | wrapped_dek[N]
///   ciphertext_len[4 BE]  | ciphertext[N]   (ct includes 16-byte AEAD tag)
/// ```
#[derive(Debug)]
pub struct EnvelopeView<'a> {
    pub kek_id: String,
    pub nonce: &'a [u8; NONCE_LEN],
    pub wrapped_dek: &'a [u8],
    pub ciphertext: &'a [u8],
}

/// Encode a binary envelope.  Returns `LengthOverflow` if any field exceeds
/// the wire-format's length-field width.
pub fn encode_envelope(
    kek_id: &str,
    nonce: &[u8; NONCE_LEN],
    wrapped_dek: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, KmsV2Error> {
    if kek_id.len() > u8::MAX as usize { return Err(KmsV2Error::LengthOverflow); }
    if wrapped_dek.len() > u16::MAX as usize { return Err(KmsV2Error::LengthOverflow); }
    if ciphertext.len() > u32::MAX as usize { return Err(KmsV2Error::LengthOverflow); }

    let mut out = Vec::with_capacity(2 + 1 + 1 + kek_id.len() + NONCE_LEN + 2 + wrapped_dek.len() + 4 + ciphertext.len());
    out.extend_from_slice(&KMSV2_MAGIC);
    out.push(KMSV2_VERSION);
    out.push(kek_id.len() as u8);
    out.extend_from_slice(kek_id.as_bytes());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&(wrapped_dek.len() as u16).to_be_bytes());
    out.extend_from_slice(wrapped_dek);
    out.extend_from_slice(&(ciphertext.len() as u32).to_be_bytes());
    out.extend_from_slice(ciphertext);
    Ok(out)
}

/// Parse a binary envelope into an [`EnvelopeView`].  Borrows from `buf`.
pub fn decode_envelope(buf: &[u8]) -> Result<EnvelopeView<'_>, KmsV2Error> {
    if buf.len() < 2 + 1 + 1 { return Err(KmsV2Error::Truncated); }
    if buf[0..2] != KMSV2_MAGIC { return Err(KmsV2Error::BadMagic); }
    if buf[2] != KMSV2_VERSION { return Err(KmsV2Error::UnsupportedVersion(buf[2])); }

    let mut p = 3usize;
    let kek_id_len = buf[p] as usize; p += 1;
    if p + kek_id_len + NONCE_LEN + 2 > buf.len() { return Err(KmsV2Error::Truncated); }
    let kek_id = std::str::from_utf8(&buf[p..p + kek_id_len])
        .map_err(|_| KmsV2Error::BadMagic)?
        .to_string();
    p += kek_id_len;

    let nonce: &[u8; NONCE_LEN] = (&buf[p..p + NONCE_LEN]).try_into().unwrap();
    p += NONCE_LEN;

    let wrapped_len = u16::from_be_bytes(buf[p..p + 2].try_into().unwrap()) as usize;
    p += 2;
    if p + wrapped_len + 4 > buf.len() { return Err(KmsV2Error::Truncated); }
    let wrapped_dek = &buf[p..p + wrapped_len];
    p += wrapped_len;

    let ct_len = u32::from_be_bytes(buf[p..p + 4].try_into().unwrap()) as usize;
    p += 4;
    if p + ct_len > buf.len() { return Err(KmsV2Error::Truncated); }
    let ciphertext = &buf[p..p + ct_len];

    Ok(EnvelopeView { kek_id, nonce, wrapped_dek, ciphertext })
}

// ── DEK cache ─────────────────────────────────────────────────────────────

/// Cache wrapped-DEK ⇒ raw-DEK so repeated reads of the same value skip
/// the provider round-trip.
#[derive(Debug)]
pub struct DekCache {
    capacity: usize,
    ttl: Duration,
    inner: Mutex<DekCacheInner>,
}

#[derive(Debug, Default)]
struct DekCacheInner {
    entries: HashMap<Vec<u8>, ([u8; DEK_LEN], Instant)>,
    hits: u64,
    misses: u64,
}

impl DekCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self { capacity, ttl, inner: Mutex::new(DekCacheInner::default()) }
    }

    pub fn get(&self, wrapped: &[u8]) -> Option<[u8; DEK_LEN]> {
        let mut e = self.inner.lock().unwrap();
        if let Some((dek, inserted)) = e.entries.get(wrapped).cloned() {
            if inserted.elapsed() <= self.ttl {
                e.hits += 1;
                return Some(dek);
            }
            e.entries.remove(wrapped);
        }
        e.misses += 1;
        None
    }

    pub fn put(&self, wrapped: Vec<u8>, dek: [u8; DEK_LEN]) {
        let mut e = self.inner.lock().unwrap();
        if e.entries.len() >= self.capacity {
            if let Some(k) = e.entries.keys().next().cloned() {
                e.entries.remove(&k);
            }
        }
        e.entries.insert(wrapped, (dek, Instant::now()));
    }

    pub fn len(&self) -> usize { self.inner.lock().unwrap().entries.len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
    pub fn hits(&self) -> u64 { self.inner.lock().unwrap().hits }
    pub fn misses(&self) -> u64 { self.inner.lock().unwrap().misses }
    pub fn clear(&self) {
        let mut e = self.inner.lock().unwrap();
        e.entries.clear();
        e.hits = 0;
        e.misses = 0;
    }
}

// ── AAD-bound seal/open ───────────────────────────────────────────────────

/// Build the AAD used for tenant binding.  The separator byte makes the
/// boundary unambiguous so `tenant="ab"+key="cd"` and `tenant="a"+key="bcd"`
/// produce different AADs.
pub fn tenant_aad(tenant_id: &str, key: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(tenant_id.len() + 1 + key.len());
    aad.extend_from_slice(tenant_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(key);
    aad
}

fn keystream_byte(dek: &[u8; DEK_LEN], nonce: &[u8; NONCE_LEN], i: usize) -> u8 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in dek.iter().chain(nonce.iter()) {
        h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
    }
    h = h.wrapping_mul(0x100000001b3).wrapping_add(i as u64);
    (h.wrapping_shr(56) ^ h) as u8
}

fn mac(dek: &[u8; DEK_LEN], nonce: &[u8; NONCE_LEN], aad: &[u8], ct: &[u8]) -> [u8; TAG_LEN] {
    let mut tag = [0u8; TAG_LEN];
    for (i, slot) in tag.iter_mut().enumerate() {
        let mut h: u64 = 0x9e3779b97f4a7c15 ^ (i as u64).wrapping_mul(0x100000001b3);
        for &b in dek.iter().chain(nonce.iter()).chain(aad.iter()).chain(ct.iter()) {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        *slot = (h ^ h.rotate_right(31)) as u8;
    }
    tag
}

/// AEAD seal — XOR-encrypt then MAC.  Test-grade; production swaps in AES-GCM.
pub fn aead_seal(
    dek: &[u8; DEK_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(plaintext.len() + TAG_LEN);
    for (i, &p) in plaintext.iter().enumerate() {
        out.push(p ^ keystream_byte(dek, nonce, i));
    }
    let tag = mac(dek, nonce, aad, &out);
    out.extend_from_slice(&tag);
    out
}

/// AEAD open — verify MAC then XOR-decrypt.  Returns None on auth failure.
pub fn aead_open(
    dek: &[u8; DEK_LEN],
    nonce: &[u8; NONCE_LEN],
    envelope: &[u8],
    aad: &[u8],
) -> Option<Vec<u8>> {
    if envelope.len() < TAG_LEN { return None; }
    let split = envelope.len() - TAG_LEN;
    let (ct, tag) = (&envelope[..split], &envelope[split..]);
    let want = mac(dek, nonce, aad, ct);
    let mut diff: u8 = 0;
    for (a, b) in tag.iter().zip(want.iter()) { diff |= a ^ b; }
    if diff != 0 { return None; }
    Some(ct.iter().enumerate().map(|(i, &b)| b ^ keystream_byte(dek, nonce, i)).collect())
}

// ── Provider-level wrappers (DEK derivation + cache integration) ──────────

fn derive_dek(seed: u64) -> [u8; DEK_LEN] {
    let mut dek = [0u8; DEK_LEN];
    let mut s: u64 = 0x1234_5678_9abc_def0 ^ seed;
    for byte in &mut dek {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *byte = (s >> 56) as u8;
    }
    dek
}

/// Counter for deterministic nonces.  Tests can pin via [`reset_nonce_for_test`].
static NONCE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Pin the nonce counter — only for tests.
pub fn reset_nonce_for_test(seed: u64) {
    NONCE_SEQ.store(seed, std::sync::atomic::Ordering::SeqCst);
}

fn next_nonce() -> [u8; NONCE_LEN] {
    let n = NONCE_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst).wrapping_add(1);
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..8].copy_from_slice(&n.to_be_bytes());
    nonce
}

/// Encrypt `plaintext` under the active KEK with the given AAD.  Stores the
/// freshly-derived DEK in the cache for fast follow-up reads.
pub fn encrypt(
    kms: &dyn KmsProvider,
    cache: &DekCache,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, KmsV2Error> {
    let nonce = next_nonce();
    let seed = u64::from_be_bytes(nonce[..8].try_into().unwrap());
    let dek = derive_dek(seed);
    let (kek_id, wrapped) = kms.wrap_dek(&dek)?;
    cache.put(wrapped.clone(), dek);
    let ciphertext = aead_seal(&dek, &nonce, plaintext, aad);
    encode_envelope(&kek_id, &nonce, &wrapped, &ciphertext)
}

/// Decrypt an envelope.  Looks the DEK up in `cache` first; otherwise
/// goes back to the provider via `unwrap_dek`.
pub fn decrypt(
    kms: &dyn KmsProvider,
    cache: &DekCache,
    envelope: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, KmsV2Error> {
    let view = decode_envelope(envelope)?;
    let dek = if let Some(dek) = cache.get(view.wrapped_dek) {
        dek
    } else {
        let raw = kms.unwrap_dek(&view.kek_id, view.wrapped_dek)
            .map_err(|e| match e {
                KmsError::UnknownKekId(id) => KmsV2Error::UnknownKekId(id),
                other => KmsV2Error::Provider(other),
            })?;
        if raw.len() != DEK_LEN {
            return Err(KmsV2Error::Provider(KmsError::Decrypt(
                format!("unwrapped DEK length {} != {}", raw.len(), DEK_LEN),
            )));
        }
        let mut dek = [0u8; DEK_LEN];
        dek.copy_from_slice(&raw);
        cache.put(view.wrapped_dek.to_vec(), dek);
        dek
    };
    aead_open(&dek, view.nonce, view.ciphertext, aad).ok_or(KmsV2Error::Authentication)
}

/// Rotate: decrypt under the old KEK, re-encrypt under whatever the
/// provider considers active.
pub fn rotate(
    kms: &dyn KmsProvider,
    cache: &DekCache,
    envelope: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, KmsV2Error> {
    let plaintext = decrypt(kms, cache, envelope, aad)?;
    encrypt(kms, cache, &plaintext, aad)
}

// ─────────────────────────────────────────────────────────────────────────
// KMSv2 binary-envelope tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kms::InMemoryKmsProvider;

    fn fixed_kek(byte: u8) -> [u8; 32] { [byte; 32] }

    fn provider() -> InMemoryKmsProvider {
        InMemoryKmsProvider::new("k1", fixed_kek(0xAA))
    }

    fn cache() -> DekCache { DekCache::new(64, Duration::from_secs(60)) }

    // ── Envelope codec ────────────────────────────────────────────────

    #[test]
    fn test_kmsv2_envelope_roundtrip() {
        // cite: KEP-3299 envelope-encryption v2
        let nonce = [0x11u8; NONCE_LEN];
        let env = encode_envelope("k1", &nonce, &[7; 32], &[3; 100]).unwrap();
        let view = decode_envelope(&env).unwrap();
        assert_eq!(view.kek_id, "k1");
        assert_eq!(view.nonce, &nonce);
        assert_eq!(view.wrapped_dek.len(), 32);
        assert_eq!(view.ciphertext.len(), 100);
    }

    #[test]
    fn test_kmsv2_envelope_truncated() {
        // cite: etcd v3.6.10 envelope.go (length-prefixed wire format)
        assert_eq!(decode_envelope(&[]).unwrap_err(), KmsV2Error::Truncated);
        assert_eq!(decode_envelope(&[0x6B]).unwrap_err(), KmsV2Error::Truncated);
    }

    #[test]
    fn test_kmsv2_envelope_bad_magic() {
        // cite: envelope.go (Magic mismatch ⇒ reject)
        let env = encode_envelope("k1", &[0; NONCE_LEN], &[3; 4], &[4; 8]).unwrap();
        let mut tampered = env.clone();
        tampered[0] = 0;
        assert_eq!(decode_envelope(&tampered).unwrap_err(), KmsV2Error::BadMagic);
    }

    #[test]
    fn test_kmsv2_envelope_unsupported_version() {
        // cite: envelope.go (Version mismatch ⇒ reject, never silently upgrade)
        let env = encode_envelope("k1", &[0; NONCE_LEN], &[3; 4], &[4; 8]).unwrap();
        let mut tampered = env.clone();
        tampered[2] = 0xFF;
        assert_eq!(decode_envelope(&tampered).unwrap_err(), KmsV2Error::UnsupportedVersion(0xFF));
    }

    #[test]
    fn test_kmsv2_envelope_long_kek_id_overflow() {
        // cite: envelope.go (length-prefixed fields MUST fit their width)
        let id: String = "k".repeat(300);
        assert_eq!(
            encode_envelope(&id, &[0; NONCE_LEN], &[3; 4], &[4; 8]).unwrap_err(),
            KmsV2Error::LengthOverflow
        );
    }

    #[test]
    fn test_kmsv2_envelope_layout_starts_with_magic() {
        // cite: envelope.go ("k2" + version)
        let env = encode_envelope("k", &[0; NONCE_LEN], &[3; 4], &[4; 8]).unwrap();
        assert_eq!(&env[0..2], &KMSV2_MAGIC);
        assert_eq!(env[2], KMSV2_VERSION);
    }

    // ── DEK cache ─────────────────────────────────────────────────────

    #[test]
    fn test_dek_cache_hit_and_miss() {
        // cite: kms/pkg/encrypt/aes/cache.go (LRU cache for unwrapped DEKs)
        let c = DekCache::new(4, Duration::from_secs(60));
        assert!(c.get(b"x").is_none());
        c.put(b"x".to_vec(), [0x42; DEK_LEN]);
        assert_eq!(c.get(b"x").unwrap(), [0x42; DEK_LEN]);
        assert_eq!(c.hits(), 1);
        assert_eq!(c.misses(), 1);
    }

    #[test]
    fn test_dek_cache_capacity_evicts() {
        // cite: cache.go (bounded cache)
        let c = DekCache::new(2, Duration::from_secs(60));
        c.put(b"a".to_vec(), [1; DEK_LEN]);
        c.put(b"b".to_vec(), [2; DEK_LEN]);
        c.put(b"c".to_vec(), [3; DEK_LEN]);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn test_dek_cache_ttl_expires() {
        // cite: cache.go (TTL invalidation on stale entries)
        let c = DekCache::new(8, Duration::from_millis(1));
        c.put(b"x".to_vec(), [9; DEK_LEN]);
        std::thread::sleep(Duration::from_millis(15));
        assert!(c.get(b"x").is_none());
    }

    #[test]
    fn test_dek_cache_clear_resets_stats() {
        // cite: cache.go (Reset)
        let c = DekCache::new(8, Duration::from_secs(60));
        c.put(b"x".to_vec(), [1; DEK_LEN]);
        let _ = c.get(b"x");
        let _ = c.get(b"missing");
        c.clear();
        assert_eq!(c.hits(), 0);
        assert_eq!(c.misses(), 0);
        assert!(c.is_empty());
    }

    // ── AAD seal/open ─────────────────────────────────────────────────

    #[test]
    fn test_aead_seal_open_roundtrip() {
        // cite: KEP-3299 §6 (AAD binds the ciphertext to a context)
        let dek = [0x33; DEK_LEN];
        let nonce = [0x11u8; NONCE_LEN];
        let sealed = aead_seal(&dek, &nonce, b"hello", b"tenantA");
        assert_eq!(aead_open(&dek, &nonce, &sealed, b"tenantA").unwrap(), b"hello");
    }

    #[test]
    fn test_aead_open_bad_aad_fails() {
        // cite: KEP-3299 §6 (AAD mismatch ⇒ auth failure)
        let dek = [0x33; DEK_LEN];
        let nonce = [0x11u8; NONCE_LEN];
        let sealed = aead_seal(&dek, &nonce, b"hello", b"tenantA");
        assert!(aead_open(&dek, &nonce, &sealed, b"tenantB").is_none());
    }

    #[test]
    fn test_aead_open_bad_key_fails() {
        // cite: KEP-3299 §6 (wrong key ⇒ auth failure)
        let dek = [0x33; DEK_LEN];
        let dek2 = [0x44; DEK_LEN];
        let nonce = [0x11u8; NONCE_LEN];
        let sealed = aead_seal(&dek, &nonce, b"hello", b"aad");
        assert!(aead_open(&dek2, &nonce, &sealed, b"aad").is_none());
    }

    #[test]
    fn test_aead_open_tampered_ct_fails() {
        // cite: KEP-3299 §6 (ciphertext modification ⇒ auth failure)
        let dek = [0x33; DEK_LEN];
        let nonce = [0x11u8; NONCE_LEN];
        let mut sealed = aead_seal(&dek, &nonce, b"hello", b"aad");
        sealed[0] ^= 1;
        assert!(aead_open(&dek, &nonce, &sealed, b"aad").is_none());
    }

    #[test]
    fn test_aead_open_tampered_tag_fails() {
        // cite: KEP-3299 §6 (tag modification ⇒ auth failure)
        let dek = [0x33; DEK_LEN];
        let nonce = [0x11u8; NONCE_LEN];
        let mut sealed = aead_seal(&dek, &nonce, b"hello", b"aad");
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(aead_open(&dek, &nonce, &sealed, b"aad").is_none());
    }

    // ── Tenant AAD ────────────────────────────────────────────────────

    #[test]
    fn test_tenant_aad_separator_disambiguates() {
        // cite: KEP-3299 §6 (AAD must not split ambiguously)
        let a = tenant_aad("ab", b"cd");
        let b = tenant_aad("a", b"bcd");
        assert_ne!(a, b);
    }

    #[test]
    fn test_tenant_aad_round_trip() {
        // cite: KEP-3299 §6
        let aad = tenant_aad("acme", b"my/key");
        assert!(aad.starts_with(b"acme"));
        assert!(aad.contains(&0));
        assert!(aad.ends_with(b"my/key"));
    }

    // ── Provider integration ──────────────────────────────────────────

    #[test]
    fn test_kmsv2_encrypt_decrypt_roundtrip() {
        // cite: etcd v3.6.10 datadir/encryption (provider round-trip)
        reset_nonce_for_test(0x1000);
        let kms = provider();
        let c = cache();
        let aad = tenant_aad("acme", b"k");
        let ct = encrypt(&kms, &c, b"secret", &aad).unwrap();
        assert_eq!(decrypt(&kms, &c, &ct, &aad).unwrap(), b"secret");
    }

    #[test]
    fn test_kmsv2_decrypt_wrong_aad_fails() {
        // cite: KEP-3299 §6 (cross-tenant replay rejected)
        reset_nonce_for_test(0x2000);
        let kms = provider();
        let c = cache();
        let ct = encrypt(&kms, &c, b"v", &tenant_aad("acme", b"k")).unwrap();
        assert_eq!(
            decrypt(&kms, &c, &ct, &tenant_aad("evil", b"k")).unwrap_err(),
            KmsV2Error::Authentication
        );
    }

    #[test]
    fn test_kmsv2_envelope_carries_kek_id() {
        // cite: envelope.go (kek_id persisted for rotation)
        reset_nonce_for_test(0x3000);
        let kms = InMemoryKmsProvider::new("kek-2026-04", fixed_kek(0xCC));
        let c = cache();
        let ct = encrypt(&kms, &c, b"v", b"aad").unwrap();
        let view = decode_envelope(&ct).unwrap();
        assert_eq!(view.kek_id, "kek-2026-04");
    }

    #[test]
    fn test_kmsv2_decrypt_unknown_kek_id_fails() {
        // cite: envelope.go (KEK rotated away ⇒ UnknownKekId)
        reset_nonce_for_test(0x4000);
        let kms = provider();
        let c = cache();
        let ct = encrypt(&kms, &c, b"v", b"aad").unwrap();
        // A fresh provider has a different active KEK and doesn't know "k1".
        let other = InMemoryKmsProvider::new("k99", fixed_kek(0xBB));
        // Clear the cache so we actually hit the provider during decrypt.
        let other_cache = DekCache::new(64, Duration::from_secs(60));
        let err = decrypt(&other, &other_cache, &ct, b"aad").unwrap_err();
        match err { KmsV2Error::UnknownKekId(id) => assert_eq!(id, "k1"), other => panic!("{other:?}") }
    }

    #[test]
    fn test_kmsv2_rotate_changes_kek_id() {
        // cite: ADR-105 §rotation
        reset_nonce_for_test(0x5000);
        let kms = InMemoryKmsProvider::new("k1", fixed_kek(0x11));
        let c = cache();
        let aad = tenant_aad("t", b"k");
        let v1 = encrypt(&kms, &c, b"hello", &aad).unwrap();
        kms.rotate("k2", fixed_kek(0x22));
        let v2 = rotate(&kms, &c, &v1, &aad).unwrap();
        assert_eq!(decode_envelope(&v1).unwrap().kek_id, "k1");
        assert_eq!(decode_envelope(&v2).unwrap().kek_id, "k2");
        assert_eq!(decrypt(&kms, &c, &v2, &aad).unwrap(), b"hello");
    }

    #[test]
    fn test_kmsv2_rotate_old_envelope_still_decryptable() {
        // cite: ADR-105 §rotation (old envelopes must remain readable)
        reset_nonce_for_test(0x6000);
        let kms = InMemoryKmsProvider::new("k1", fixed_kek(0x11));
        let c = cache();
        let v1 = encrypt(&kms, &c, b"old-data", b"aad").unwrap();
        kms.rotate("k2", fixed_kek(0x22));
        // old envelope still decrypts because k1 is still in the registry
        c.clear();
        assert_eq!(decrypt(&kms, &c, &v1, b"aad").unwrap(), b"old-data");
    }

    #[test]
    fn test_kmsv2_dek_cache_hit_after_first_decrypt() {
        // cite: cache.go (subsequent reads skip the provider)
        reset_nonce_for_test(0x7000);
        let kms = provider();
        let c = cache();
        let ct = encrypt(&kms, &c, b"hello", b"aad").unwrap();
        let hits_before = c.hits();
        decrypt(&kms, &c, &ct, b"aad").unwrap();
        assert!(c.hits() > hits_before);
    }

    #[test]
    fn test_kmsv2_two_calls_use_distinct_nonces() {
        // cite: KEP-3299 §6 (nonce uniqueness)
        reset_nonce_for_test(0x8000);
        let kms = provider();
        let c = cache();
        let a = encrypt(&kms, &c, b"x", b"aad").unwrap();
        let b = encrypt(&kms, &c, b"x", b"aad").unwrap();
        assert_ne!(a, b);
        let va = decode_envelope(&a).unwrap();
        let vb = decode_envelope(&b).unwrap();
        assert_ne!(va.nonce, vb.nonce);
    }

    #[test]
    fn test_kmsv2_empty_plaintext_roundtrip() {
        // cite: AEAD must accept empty plaintext (still produce a tag)
        reset_nonce_for_test(0x9000);
        let kms = provider();
        let c = cache();
        let ct = encrypt(&kms, &c, b"", b"aad").unwrap();
        assert!(decrypt(&kms, &c, &ct, b"aad").unwrap().is_empty());
    }

    #[test]
    fn test_kmsv2_large_plaintext_roundtrip() {
        // cite: ADR-105 (large-value tenancy data)
        reset_nonce_for_test(0xA000);
        let kms = provider();
        let c = cache();
        let plain: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
        let ct = encrypt(&kms, &c, &plain, b"aad").unwrap();
        assert_eq!(decrypt(&kms, &c, &ct, b"aad").unwrap(), plain);
    }

    #[test]
    fn test_kmsv2_tenant_isolation_per_key() {
        // cite: KEP-3299 §6 (key-binding via AAD)
        reset_nonce_for_test(0xB000);
        let kms = provider();
        let c = cache();
        let ct_a = encrypt(&kms, &c, b"v", &tenant_aad("acme", b"k1")).unwrap();
        // Decrypting with the *same* tenant but a different key must fail.
        assert_eq!(
            decrypt(&kms, &c, &ct_a, &tenant_aad("acme", b"k2")).unwrap_err(),
            KmsV2Error::Authentication,
        );
    }

    #[test]
    fn test_kmsv2_cross_kek_envelope_with_provider_supporting_both() {
        // cite: ADR-105 (rotation keeps multiple active KEKs)
        reset_nonce_for_test(0xC000);
        let kms = InMemoryKmsProvider::new("k1", fixed_kek(0x11));
        let c = cache();
        let v_old = encrypt(&kms, &c, b"old", b"aad").unwrap();
        kms.rotate("k2", fixed_kek(0x22));
        let v_new = encrypt(&kms, &c, b"new", b"aad").unwrap();
        c.clear();
        assert_eq!(decrypt(&kms, &c, &v_old, b"aad").unwrap(), b"old");
        assert_eq!(decrypt(&kms, &c, &v_new, b"aad").unwrap(), b"new");
        assert_eq!(decode_envelope(&v_old).unwrap().kek_id, "k1");
        assert_eq!(decode_envelope(&v_new).unwrap().kek_id, "k2");
    }
}
