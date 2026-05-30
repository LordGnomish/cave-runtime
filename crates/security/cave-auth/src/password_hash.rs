// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! PBKDF2 password hashing.
//!
//! Line-ported from Keycloak (Apache-2.0):
//!   `server-spi-private/.../credential/hash/Pbkdf2PasswordHashProvider.java`.
//!
//! The credential string is the standard-Base64 encoding of the PBKDF2-derived
//! key, and [`verify`](Pbkdf2PasswordHashProvider::verify) reproduces the hash
//! from the stored salt + iteration count + derived-key length — matching the
//! upstream `encodedCredential` / `verify` / `keySize(credential)` contract.

use std::num::NonZeroU32;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::pbkdf2;

/// PBKDF2 PRF — mirrors Keycloak's `PBKDF2WithHmacSHA1 / SHA256 / SHA512`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pbkdf2Alg {
    HmacSha1,
    HmacSha256,
    HmacSha512,
}

impl Pbkdf2Alg {
    fn ring_alg(self) -> pbkdf2::Algorithm {
        match self {
            Pbkdf2Alg::HmacSha1 => pbkdf2::PBKDF2_HMAC_SHA1,
            Pbkdf2Alg::HmacSha256 => pbkdf2::PBKDF2_HMAC_SHA256,
            Pbkdf2Alg::HmacSha512 => pbkdf2::PBKDF2_HMAC_SHA512,
        }
    }
}

/// Keycloak's `DEFAULT_DERIVED_KEY_SIZE` (bits).
pub const DEFAULT_DERIVED_KEY_SIZE: usize = 512;

/// Port of `Pbkdf2PasswordHashProvider`.
#[derive(Debug, Clone)]
pub struct Pbkdf2PasswordHashProvider {
    algorithm: Pbkdf2Alg,
    default_iterations: u32,
    derived_key_size_bits: usize,
}

impl Pbkdf2PasswordHashProvider {
    pub fn new(algorithm: Pbkdf2Alg, default_iterations: u32, derived_key_size_bits: usize) -> Self {
        Self { algorithm, default_iterations, derived_key_size_bits }
    }

    /// Raw PBKDF2 derivation — `dk_len_bytes` octets of output.
    pub fn derive(&self, password: &[u8], salt: &[u8], iterations: u32, dk_len_bytes: usize) -> Vec<u8> {
        let iters = NonZeroU32::new(iterations).expect("iterations must be > 0");
        let mut out = vec![0u8; dk_len_bytes];
        pbkdf2::derive(self.algorithm.ring_alg(), iters, salt, password, &mut out);
        out
    }

    /// Port of `encodedCredential(rawPassword, iterations, salt, derivedKeySize)`.
    ///
    /// Returns the standard-Base64 encoding of the derived key using the
    /// provider's configured derived-key size. `iterations == -1` (here: any
    /// caller using the sentinel via [`encode_default`]) falls back to the default.
    pub fn encode(&self, raw_password: &str, iterations: u32, salt: &[u8]) -> String {
        self.encode_with_size(raw_password, iterations, salt, self.derived_key_size_bits)
    }

    /// Encode with an explicit derived-key size (bits) — used when re-deriving a
    /// credential stored at a non-default size.
    pub fn encode_with_size(
        &self,
        raw_password: &str,
        iterations: u32,
        salt: &[u8],
        derived_key_size_bits: usize,
    ) -> String {
        let key = self.derive(raw_password.as_bytes(), salt, iterations, derived_key_size_bits / 8);
        STANDARD.encode(key)
    }

    /// Encode using the provider's default iteration count.
    pub fn encode_default(&self, raw_password: &str, salt: &[u8]) -> String {
        self.encode(raw_password, self.default_iterations, salt)
    }

    /// Port of `verify(rawPassword, credential)`.
    ///
    /// Re-derives at the SAME key size as the stored hash (`keySize(credential)`
    /// in Keycloak = decoded-bytes × 8) and compares the Base64 strings.
    pub fn verify(&self, raw_password: &str, salt: &[u8], iterations: u32, encoded: &str) -> bool {
        let stored = match STANDARD.decode(encoded) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let key_size_bits = stored.len() * 8;
        let candidate = self.encode_with_size(raw_password, iterations, salt, key_size_bits);
        candidate == encoded
    }

    pub fn default_iterations(&self) -> u32 {
        self.default_iterations
    }
}
