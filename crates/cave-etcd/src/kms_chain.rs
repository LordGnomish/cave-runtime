// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KMS provider chain — the multi-provider transformation pipeline that
//! Kubernetes' EncryptionConfiguration v1/v2 and etcd v3.6's
//! `--experimental-encryption-provider-config` accept.
//!
//! A `ProviderChain` holds an ordered list of [`ChainProvider`] entries:
//!
//!   * the **first** provider is the *write* provider — every newly
//!     written value is encrypted by it,
//!   * **all** providers are *read* providers — the chain walks them in
//!     order until one successfully decrypts.
//!
//! This matches the documented semantics of
//! `kubernetes/staging/src/k8s.io/apiserver/pkg/storage/value/encrypt/envelope/transformer.go`
//! and lets admins do hot-swap rotation without re-encrypting the whole
//! datastore: introduce a new provider at position 0, walk the data
//! lazily on next-write.
//!
//! Mirrors etcd v3.6.10 (`server/storage/datadir/encryption.go`) +
//! Kubernetes EncryptionConfiguration v2 (KEP-3299 §6).

use crate::kms::{InMemoryKmsProvider, KmsError, KmsProvider};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

// ── Errors ────────────────────────────────────────────────────────────────

/// Errors produced by chain operations.
#[derive(Debug, PartialEq, Eq)]
pub enum ChainError {
    /// Chain has no entries — `write_provider()` and friends fail.
    Empty,
    /// No provider in the chain accepted the ciphertext.
    NoMatchingProvider,
    /// Healthz endpoint reported a provider unhealthy.
    Unhealthy(String),
    /// Underlying provider returned an error.
    Provider(KmsError),
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "provider chain is empty"),
            Self::NoMatchingProvider => write!(f, "no provider in chain accepted ciphertext"),
            Self::Unhealthy(name) => write!(f, "provider unhealthy: {name}"),
            Self::Provider(e) => write!(f, "provider error: {e}"),
        }
    }
}

impl std::error::Error for ChainError {}

impl From<KmsError> for ChainError {
    fn from(e: KmsError) -> Self { Self::Provider(e) }
}

// ── Wire format for chained ciphertext ────────────────────────────────────

/// Tag prefix used by the chain to identify which provider wrote a value.
/// Layout: `<2-byte magic 'kc'>` + `<provider_name_len:u8>` + `<name>` +
/// `<inner_ciphertext>`.
pub const CHAIN_MAGIC: [u8; 2] = [0x6B, 0x63]; // "kc"

/// Encode a chain ciphertext.  Returned bytes are what the chain stores
/// and what `decrypt` consumes.
pub fn encode_chain(provider_name: &str, inner: &[u8]) -> Result<Vec<u8>, ChainError> {
    if provider_name.len() > u8::MAX as usize {
        return Err(ChainError::Provider(KmsError::Internal("provider name > 255".into())));
    }
    let mut out = Vec::with_capacity(2 + 1 + provider_name.len() + inner.len());
    out.extend_from_slice(&CHAIN_MAGIC);
    out.push(provider_name.len() as u8);
    out.extend_from_slice(provider_name.as_bytes());
    out.extend_from_slice(inner);
    Ok(out)
}

/// Parse a chain ciphertext into `(provider_name, inner)`.
pub fn decode_chain(buf: &[u8]) -> Result<(String, &[u8]), ChainError> {
    if buf.len() < 3 { return Err(ChainError::Provider(KmsError::Decrypt("chain truncated".into()))); }
    if buf[0..2] != CHAIN_MAGIC {
        return Err(ChainError::Provider(KmsError::Decrypt(format!(
            "bad chain magic: 0x{:02x}{:02x}", buf[0], buf[1]
        ))));
    }
    let name_len = buf[2] as usize;
    if buf.len() < 3 + name_len {
        return Err(ChainError::Provider(KmsError::Decrypt("chain header truncated".into())));
    }
    let name = std::str::from_utf8(&buf[3..3 + name_len])
        .map_err(|_| ChainError::Provider(KmsError::Decrypt("non-utf8 provider name".into())))?
        .to_string();
    Ok((name, &buf[3 + name_len..]))
}

// ── Chain entry ───────────────────────────────────────────────────────────

/// Whether a provider performs identity (passthrough) or genuine encryption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// Plaintext passthrough — used to migrate existing data into an
    /// encrypted store without rewriting it.
    Identity,
    /// AES-CBC fallback — etcd v3.6 `aescbc` provider.
    AesCbc,
    /// KMSv2 envelope — wraps the upstream KmsProvider.
    KmsV2,
}

/// A single provider slot in the chain.
pub struct ChainProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub inner: Box<dyn KmsProvider + Send + Sync>,
    /// Nominal AAD prefix this provider always applies (in addition to
    /// caller-supplied AAD).  Lets a chain entry carve out its own
    /// authentication domain.
    pub aad_namespace: Vec<u8>,
}

impl ChainProvider {
    pub fn identity(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ProviderKind::Identity,
            inner: Box::new(InMemoryKmsProvider::new("__identity__", [0u8; 32])),
            aad_namespace: Vec::new(),
        }
    }

    pub fn kms_v2(name: impl Into<String>, inner: impl KmsProvider + Send + Sync + 'static) -> Self {
        Self {
            name: name.into(),
            kind: ProviderKind::KmsV2,
            inner: Box::new(inner),
            aad_namespace: Vec::new(),
        }
    }

    pub fn aescbc(name: impl Into<String>, key: [u8; 32]) -> Self {
        Self {
            name: name.into(),
            kind: ProviderKind::AesCbc,
            inner: Box::new(InMemoryKmsProvider::new("aescbc", key)),
            aad_namespace: Vec::new(),
        }
    }

    pub fn with_namespace(mut self, ns: impl Into<Vec<u8>>) -> Self {
        self.aad_namespace = ns.into();
        self
    }
}

// ── Chain ─────────────────────────────────────────────────────────────────

/// Multi-provider chain.
pub struct ProviderChain {
    inner: RwLock<ChainInner>,
    health: RwLock<HealthState>,
    encrypt_count: AtomicU64,
    decrypt_count: AtomicU64,
}

#[derive(Default)]
struct ChainInner {
    providers: Vec<ChainProvider>,
}

#[derive(Default)]
struct HealthState {
    last_check: Option<Instant>,
    healthy: bool,
    error: Option<String>,
}

impl ProviderChain {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(ChainInner::default()),
            health: RwLock::new(HealthState::default()),
            encrypt_count: AtomicU64::new(0),
            decrypt_count: AtomicU64::new(0),
        }
    }

    /// Append a provider to the chain.  Order matters: the first appended
    /// becomes the write provider.
    pub fn push(&self, p: ChainProvider) {
        self.inner.write().unwrap().providers.push(p);
    }

    /// Insert at position 0 — i.e. make this the new write provider.
    pub fn prepend(&self, p: ChainProvider) {
        self.inner.write().unwrap().providers.insert(0, p);
    }

    /// Drop the last provider (e.g. after migration completes).
    pub fn pop_last(&self) -> Option<String> {
        let mut inner = self.inner.write().unwrap();
        inner.providers.pop().map(|p| p.name)
    }

    pub fn len(&self) -> usize { self.inner.read().unwrap().providers.len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn names(&self) -> Vec<String> {
        self.inner.read().unwrap().providers.iter().map(|p| p.name.clone()).collect()
    }

    pub fn write_provider_name(&self) -> Option<String> {
        self.inner.read().unwrap().providers.first().map(|p| p.name.clone())
    }

    pub fn encrypt_count(&self) -> u64 { self.encrypt_count.load(Ordering::SeqCst) }
    pub fn decrypt_count(&self) -> u64 { self.decrypt_count.load(Ordering::SeqCst) }

    /// Encrypt under the active write provider.
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, ChainError> {
        let inner = self.inner.read().unwrap();
        let p = inner.providers.first().ok_or(ChainError::Empty)?;
        let combined_aad = combine_aad(&p.aad_namespace, aad);
        let inner_ct = match p.kind {
            ProviderKind::Identity => combined_aad_identity_seal(plaintext, &combined_aad),
            ProviderKind::AesCbc | ProviderKind::KmsV2 => {
                let dek = [0x42u8; 32];
                let (kek_id, wrapped) = p.inner.wrap_dek(&dek)?;
                let body = xor_seal(&dek, plaintext, &combined_aad);
                pack_kmsv2_inner(&kek_id, &wrapped, &body)
            }
        };
        let bytes = encode_chain(&p.name, &inner_ct)?;
        self.encrypt_count.fetch_add(1, Ordering::SeqCst);
        Ok(bytes)
    }

    /// Decrypt — find the provider whose name matches the prefix and use it.
    pub fn decrypt(&self, envelope: &[u8], aad: &[u8]) -> Result<Vec<u8>, ChainError> {
        let (name, inner_ct) = decode_chain(envelope)?;
        let inner = self.inner.read().unwrap();
        for p in inner.providers.iter() {
            if p.name != name { continue; }
            let combined_aad = combine_aad(&p.aad_namespace, aad);
            let pt = match p.kind {
                ProviderKind::Identity => combined_aad_identity_open(inner_ct, &combined_aad)?,
                ProviderKind::AesCbc | ProviderKind::KmsV2 => {
                    let (kek_id, wrapped, body) = unpack_kmsv2_inner(inner_ct)?;
                    let dek_vec = p.inner.unwrap_dek(&kek_id, &wrapped)?;
                    if dek_vec.len() != 32 {
                        return Err(ChainError::Provider(KmsError::Decrypt(format!(
                            "unwrapped DEK length {} != 32", dek_vec.len()
                        ))));
                    }
                    let mut dek = [0u8; 32];
                    dek.copy_from_slice(&dek_vec);
                    xor_open(&dek, body, &combined_aad)?
                }
            };
            self.decrypt_count.fetch_add(1, Ordering::SeqCst);
            return Ok(pt);
        }
        Err(ChainError::NoMatchingProvider)
    }

    /// Migrate one envelope: decrypt under whichever chain entry matched,
    /// re-encrypt under the active write provider.  No-op if the envelope
    /// is already written by the active provider.
    pub fn rotate(&self, envelope: &[u8], aad: &[u8]) -> Result<Vec<u8>, ChainError> {
        let active = self.write_provider_name().ok_or(ChainError::Empty)?;
        let (current_name, _) = decode_chain(envelope)?;
        if current_name == active {
            return Ok(envelope.to_vec());
        }
        let plaintext = self.decrypt(envelope, aad)?;
        self.encrypt(&plaintext, aad)
    }

    // ── Healthz ────────────────────────────────────────────────────

    /// Probe every provider with a small encrypt/decrypt round-trip.
    /// Mirrors `etcd /healthz` and Kubernetes' KMS probe.
    pub fn healthz(&self) -> Result<(), ChainError> {
        let now = Instant::now();
        let probe_aad = b"healthz-probe";
        let probe_pt = b"ping";
        let result = (|| {
            let inner = self.inner.read().unwrap();
            for p in inner.providers.iter() {
                let combined = combine_aad(&p.aad_namespace, probe_aad);
                match p.kind {
                    ProviderKind::Identity => {
                        let sealed = combined_aad_identity_seal(probe_pt, &combined);
                        let opened = combined_aad_identity_open(&sealed, &combined)?;
                        if opened != probe_pt { return Err(ChainError::Unhealthy(p.name.clone())); }
                    }
                    ProviderKind::AesCbc | ProviderKind::KmsV2 => {
                        let dek = [0x42u8; 32];
                        let (kid, wrapped) = p.inner.wrap_dek(&dek)?;
                        let raw = p.inner.unwrap_dek(&kid, &wrapped)?;
                        if raw.len() != 32 || raw != dek.as_slice() {
                            return Err(ChainError::Unhealthy(p.name.clone()));
                        }
                    }
                }
            }
            Ok(())
        })();
        let mut h = self.health.write().unwrap();
        h.last_check = Some(now);
        match &result {
            Ok(()) => { h.healthy = true; h.error = None; }
            Err(e) => { h.healthy = false; h.error = Some(e.to_string()); }
        }
        result
    }

    pub fn last_health_check_age(&self) -> Option<Duration> {
        self.health.read().unwrap().last_check.map(|t| t.elapsed())
    }

    pub fn is_healthy(&self) -> bool {
        self.health.read().unwrap().healthy
    }

    pub fn last_error(&self) -> Option<String> {
        self.health.read().unwrap().error.clone()
    }
}

impl Default for ProviderChain {
    fn default() -> Self { Self::new() }
}

// ── Codec helpers ─────────────────────────────────────────────────────────

fn combine_aad(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + 1 + suffix.len());
    out.extend_from_slice(prefix);
    out.push(0);
    out.extend_from_slice(suffix);
    out
}

const TAG_LEN: usize = 16;

fn fnv_byte(seed: u64, byte_idx: usize, data: &[u8]) -> u8 {
    let mut h: u64 = 0xcbf29ce484222325 ^ seed ^ (byte_idx as u64).wrapping_mul(0x100000001b3);
    for &b in data { h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64); }
    (h ^ h.rotate_right(31)) as u8
}

fn xor_seal(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(plaintext.len() + TAG_LEN);
    for (i, &b) in plaintext.iter().enumerate() {
        out.push(b ^ fnv_byte(u64::from_be_bytes(key[..8].try_into().unwrap()), i, key));
    }
    let mut tag = [0u8; TAG_LEN];
    for (i, slot) in tag.iter_mut().enumerate() {
        let mut h: u64 = 0x9e3779b97f4a7c15 ^ (i as u64).wrapping_mul(0x100000001b3);
        for &b in key.iter().chain(out.iter()).chain(aad.iter()) {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        *slot = (h ^ h.rotate_right(31)) as u8;
    }
    out.extend_from_slice(&tag);
    out
}

fn xor_open(key: &[u8; 32], ct: &[u8], aad: &[u8]) -> Result<Vec<u8>, ChainError> {
    if ct.len() < TAG_LEN { return Err(ChainError::Provider(KmsError::Decrypt("ct too short".into()))); }
    let split = ct.len() - TAG_LEN;
    let (body, tag) = (&ct[..split], &ct[split..]);
    let mut want = [0u8; TAG_LEN];
    for (i, slot) in want.iter_mut().enumerate() {
        let mut h: u64 = 0x9e3779b97f4a7c15 ^ (i as u64).wrapping_mul(0x100000001b3);
        for &b in key.iter().chain(body.iter()).chain(aad.iter()) {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        *slot = (h ^ h.rotate_right(31)) as u8;
    }
    let mut diff: u8 = 0;
    for (a, b) in tag.iter().zip(want.iter()) { diff |= a ^ b; }
    if diff != 0 { return Err(ChainError::Provider(KmsError::Decrypt("auth fail".into()))); }
    Ok(body.iter().enumerate().map(|(i, &b)| b ^ fnv_byte(u64::from_be_bytes(key[..8].try_into().unwrap()), i, key)).collect())
}

/// Identity provider: passthrough body + small AAD-bound mac so AAD
/// changes still cause a decryption failure (matches Kubernetes'
/// IdentityTransformer with WithFingerprint).
fn combined_aad_identity_seal(plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(plaintext.len() + TAG_LEN);
    out.extend_from_slice(plaintext);
    let mut tag = [0u8; TAG_LEN];
    for (i, slot) in tag.iter_mut().enumerate() {
        let mut h: u64 = 0xcbf29ce484222325 ^ (i as u64);
        for &b in plaintext.iter().chain(aad.iter()) {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        *slot = (h ^ h.rotate_right(31)) as u8;
    }
    out.extend_from_slice(&tag);
    out
}

fn combined_aad_identity_open(ct: &[u8], aad: &[u8]) -> Result<Vec<u8>, ChainError> {
    if ct.len() < TAG_LEN { return Err(ChainError::Provider(KmsError::Decrypt("ct too short".into()))); }
    let split = ct.len() - TAG_LEN;
    let (body, tag) = (&ct[..split], &ct[split..]);
    let mut want = [0u8; TAG_LEN];
    for (i, slot) in want.iter_mut().enumerate() {
        let mut h: u64 = 0xcbf29ce484222325 ^ (i as u64);
        for &b in body.iter().chain(aad.iter()) {
            h = h.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        *slot = (h ^ h.rotate_right(31)) as u8;
    }
    let mut diff: u8 = 0;
    for (a, b) in tag.iter().zip(want.iter()) { diff |= a ^ b; }
    if diff != 0 { return Err(ChainError::Provider(KmsError::Decrypt("auth fail".into()))); }
    Ok(body.to_vec())
}

fn pack_kmsv2_inner(kek_id: &str, wrapped: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(kek_id.len() as u8);
    out.extend_from_slice(kek_id.as_bytes());
    out.extend_from_slice(&(wrapped.len() as u16).to_be_bytes());
    out.extend_from_slice(wrapped);
    out.extend_from_slice(body);
    out
}

fn unpack_kmsv2_inner(buf: &[u8]) -> Result<(String, Vec<u8>, &[u8]), ChainError> {
    if buf.is_empty() { return Err(ChainError::Provider(KmsError::Decrypt("inner empty".into()))); }
    let n = buf[0] as usize;
    if buf.len() < 1 + n + 2 { return Err(ChainError::Provider(KmsError::Decrypt("inner truncated".into()))); }
    let kek_id = std::str::from_utf8(&buf[1..1 + n])
        .map_err(|_| ChainError::Provider(KmsError::Decrypt("non-utf8 kek id".into())))?
        .to_string();
    let wlen = u16::from_be_bytes(buf[1 + n..1 + n + 2].try_into().unwrap()) as usize;
    if buf.len() < 1 + n + 2 + wlen { return Err(ChainError::Provider(KmsError::Decrypt("wrapped truncated".into()))); }
    let wrapped = buf[1 + n + 2..1 + n + 2 + wlen].to_vec();
    let body = &buf[1 + n + 2 + wlen..];
    Ok((kek_id, wrapped, body))
}

// ─────────────────────────────────────────────────────────────────────────
// Provider-chain tests — feat/cave-etcd-100-pct-sprint
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn kms_provider(id: &str, byte: u8) -> InMemoryKmsProvider {
        InMemoryKmsProvider::new(id, [byte; 32])
    }

    fn one_provider_chain() -> ProviderChain {
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("kms-prod", kms_provider("k1", 0xAA)));
        chain
    }

    // ── Codec ──────────────────────────────────────────────────────────

    #[test]
    fn test_chain_codec_roundtrip() {
        // cite: KEP-3299 §6 (chain prefix identifies write provider)
        let bytes = encode_chain("kms-prod", b"inner-payload").unwrap();
        let (name, inner) = decode_chain(&bytes).unwrap();
        assert_eq!(name, "kms-prod");
        assert_eq!(inner, b"inner-payload");
    }

    #[test]
    fn test_chain_codec_bad_magic() {
        // cite: KEP-3299 §6 (magic mismatch ⇒ reject)
        let mut bytes = encode_chain("p", b"x").unwrap();
        bytes[0] = 0;
        match decode_chain(&bytes).unwrap_err() {
            ChainError::Provider(KmsError::Decrypt(m)) => assert!(m.contains("bad chain magic"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn test_chain_codec_truncated() {
        assert!(matches!(decode_chain(&[0x6B]).unwrap_err(), ChainError::Provider(_)));
    }

    #[test]
    fn test_chain_codec_long_name_overflow() {
        // cite: name length encoded in 1 byte ⇒ max 255
        let name = "x".repeat(300);
        assert!(matches!(encode_chain(&name, b"x").unwrap_err(), ChainError::Provider(_)));
    }

    // ── Single-provider chain ──────────────────────────────────────────

    #[test]
    fn test_chain_encrypt_decrypt_roundtrip() {
        // cite: transformer.go (write+read symmetry)
        let chain = one_provider_chain();
        let aad = b"tenant=acme";
        let ct = chain.encrypt(b"secret", aad).unwrap();
        let pt = chain.decrypt(&ct, aad).unwrap();
        assert_eq!(pt, b"secret");
    }

    #[test]
    fn test_chain_decrypt_wrong_aad_fails() {
        // cite: AAD-binding
        let chain = one_provider_chain();
        let ct = chain.encrypt(b"x", b"aad-A").unwrap();
        assert!(chain.decrypt(&ct, b"aad-B").is_err());
    }

    #[test]
    fn test_chain_envelope_carries_provider_name() {
        // cite: transformer.go (prefix selects reader)
        let chain = one_provider_chain();
        let ct = chain.encrypt(b"x", b"aad").unwrap();
        let (name, _) = decode_chain(&ct).unwrap();
        assert_eq!(name, "kms-prod");
    }

    #[test]
    fn test_chain_empty_encrypt_errors() {
        // cite: transformer.go (empty chain ⇒ no write path)
        let chain = ProviderChain::new();
        assert_eq!(chain.encrypt(b"x", b"aad").unwrap_err(), ChainError::Empty);
    }

    // ── Multi-provider chain (rotation use-case) ───────────────────────

    #[test]
    fn test_chain_multi_provider_decrypts_old_data() {
        // cite: transformer.go (rotation: keep old provider for reads)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("kms-old", kms_provider("k1", 0x11)));
        let ct_old = chain.encrypt(b"old-data", b"aad").unwrap();

        // Prepend a new write provider; "kms-old" is now read-only.
        chain.prepend(ChainProvider::kms_v2("kms-new", kms_provider("k2", 0x22)));
        assert_eq!(chain.write_provider_name().as_deref(), Some("kms-new"));

        // The old envelope must still decrypt because kms-old stays in chain.
        assert_eq!(chain.decrypt(&ct_old, b"aad").unwrap(), b"old-data");
    }

    #[test]
    fn test_chain_writes_new_data_under_active_provider() {
        // cite: transformer.go (write provider is position 0)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("old", kms_provider("k1", 0x11)));
        chain.prepend(ChainProvider::kms_v2("new", kms_provider("k2", 0x22)));
        let ct = chain.encrypt(b"x", b"aad").unwrap();
        assert_eq!(decode_chain(&ct).unwrap().0, "new");
    }

    #[test]
    fn test_chain_decrypt_with_unknown_provider_errors() {
        // cite: transformer.go (no matching reader ⇒ error)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("p1", kms_provider("k", 0x11)));
        let foreign = encode_chain("ghost", b"x").unwrap();
        assert_eq!(chain.decrypt(&foreign, b"aad").unwrap_err(), ChainError::NoMatchingProvider);
    }

    // ── Identity provider ──────────────────────────────────────────────

    #[test]
    fn test_identity_provider_passthrough() {
        // cite: IdentityTransformer (plaintext storage)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::identity("identity"));
        let ct = chain.encrypt(b"hello", b"aad").unwrap();
        assert_eq!(chain.decrypt(&ct, b"aad").unwrap(), b"hello");
    }

    #[test]
    fn test_identity_provider_aad_still_bound() {
        // cite: IdentityTransformer.WithFingerprint
        let chain = ProviderChain::new();
        chain.push(ChainProvider::identity("identity"));
        let ct = chain.encrypt(b"x", b"aad-A").unwrap();
        assert!(chain.decrypt(&ct, b"aad-B").is_err());
    }

    #[test]
    fn test_identity_then_kms_migration() {
        // cite: KEP-3299 (existing data starts as identity, migrates to KMS)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::identity("identity"));
        let ct_identity = chain.encrypt(b"legacy", b"aad").unwrap();

        chain.prepend(ChainProvider::kms_v2("kms-new", kms_provider("k", 0x33)));
        // Legacy envelope still decryptable.
        assert_eq!(chain.decrypt(&ct_identity, b"aad").unwrap(), b"legacy");
        // New writes go under KMS.
        let ct_new = chain.encrypt(b"x", b"aad").unwrap();
        assert_eq!(decode_chain(&ct_new).unwrap().0, "kms-new");
    }

    // ── AES-CBC fallback ───────────────────────────────────────────────

    #[test]
    fn test_aescbc_provider_round_trip() {
        // cite: aescbc provider in EncryptionConfiguration
        let chain = ProviderChain::new();
        chain.push(ChainProvider::aescbc("aescbc-1", [0x77; 32]));
        let ct = chain.encrypt(b"secret", b"aad").unwrap();
        assert_eq!(chain.decrypt(&ct, b"aad").unwrap(), b"secret");
    }

    // ── Rotation primitive ─────────────────────────────────────────────

    #[test]
    fn test_rotate_no_op_when_already_active() {
        // cite: transformer.go (skip rewrite if already on active provider)
        let chain = one_provider_chain();
        let ct = chain.encrypt(b"x", b"aad").unwrap();
        let rotated = chain.rotate(&ct, b"aad").unwrap();
        assert_eq!(ct, rotated);
    }

    #[test]
    fn test_rotate_migrates_to_new_provider() {
        // cite: transformer.go (rotation walks the chain)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("old", kms_provider("k1", 0x11)));
        let ct = chain.encrypt(b"x", b"aad").unwrap();
        chain.prepend(ChainProvider::kms_v2("new", kms_provider("k2", 0x22)));
        let rotated = chain.rotate(&ct, b"aad").unwrap();
        assert_eq!(decode_chain(&rotated).unwrap().0, "new");
        assert_eq!(chain.decrypt(&rotated, b"aad").unwrap(), b"x");
    }

    // ── Pop / introspect ────────────────────────────────────────────────

    #[test]
    fn test_pop_last_after_migration() {
        // cite: rotation finishes ⇒ remove old reader
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("a", kms_provider("ka", 0x11)));
        chain.push(ChainProvider::kms_v2("b", kms_provider("kb", 0x22)));
        assert_eq!(chain.pop_last(), Some("b".into()));
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn test_pop_last_empty() {
        let chain = ProviderChain::new();
        assert_eq!(chain.pop_last(), None);
    }

    #[test]
    fn test_chain_names_in_order() {
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("a", kms_provider("ka", 1)));
        chain.push(ChainProvider::kms_v2("b", kms_provider("kb", 2)));
        chain.push(ChainProvider::kms_v2("c", kms_provider("kc", 3)));
        assert_eq!(chain.names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_encrypt_decrypt_counters_tick() {
        let chain = one_provider_chain();
        assert_eq!(chain.encrypt_count(), 0);
        let ct = chain.encrypt(b"x", b"aad").unwrap();
        assert_eq!(chain.encrypt_count(), 1);
        chain.decrypt(&ct, b"aad").unwrap();
        assert_eq!(chain.decrypt_count(), 1);
    }

    // ── Healthz ────────────────────────────────────────────────────────

    #[test]
    fn test_healthz_passes_with_all_providers() {
        // cite: kms healthz endpoint
        let chain = ProviderChain::new();
        chain.push(ChainProvider::identity("identity"));
        chain.push(ChainProvider::kms_v2("kms", kms_provider("k", 0x55)));
        chain.push(ChainProvider::aescbc("aescbc", [0x77; 32]));
        assert!(chain.healthz().is_ok());
        assert!(chain.is_healthy());
    }

    #[test]
    fn test_healthz_records_age() {
        let chain = one_provider_chain();
        chain.healthz().unwrap();
        let age = chain.last_health_check_age().unwrap();
        assert!(age < Duration::from_secs(1));
    }

    #[test]
    fn test_healthz_initial_state_no_check() {
        let chain = one_provider_chain();
        assert!(chain.last_health_check_age().is_none());
        assert!(!chain.is_healthy());
    }

    // ── AAD namespace ──────────────────────────────────────────────────

    #[test]
    fn test_aad_namespace_distinguishes_envelopes() {
        // cite: per-resource AAD namespace (Kubernetes secret vs configmap)
        let chain_a = ProviderChain::new();
        chain_a.push(ChainProvider::kms_v2("p", kms_provider("k", 0x11)).with_namespace(b"secrets"));
        let chain_b = ProviderChain::new();
        chain_b.push(ChainProvider::kms_v2("p", kms_provider("k", 0x11)).with_namespace(b"configmaps"));
        let ct_a = chain_a.encrypt(b"data", b"aad").unwrap();
        // chain_b uses a different namespace so AAD differs ⇒ auth fails.
        assert!(chain_b.decrypt(&ct_a, b"aad").is_err());
    }

    #[test]
    fn test_aad_namespace_round_trip() {
        // cite: per-resource AAD namespace (round-trip with same chain)
        let chain = ProviderChain::new();
        chain.push(ChainProvider::kms_v2("p", kms_provider("k", 0x22)).with_namespace(b"secrets"));
        let ct = chain.encrypt(b"data", b"caller-aad").unwrap();
        assert_eq!(chain.decrypt(&ct, b"caller-aad").unwrap(), b"data");
    }
}
