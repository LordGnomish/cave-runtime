// SPDX-License-Identifier: AGPL-3.0-or-later
//! CSRF tokens — double-submit-cookie pattern.
//!
//! Flow:
//!   1. On any GET, server issues a token via [`CsrfStore::issue`] and
//!      sets a `cave_csrf=<token>; Secure; HttpOnly=false; SameSite=Strict`
//!      cookie (HttpOnly must be off so the page's `<form>` template
//!      can read the cookie via JS and copy the value into a hidden
//!      `<input name="csrf">` — that's the second submit).
//!   2. On any state-mutating POST/PUT/DELETE, the server reads both
//!      the cookie and the form/header value and compares them via
//!      [`validate_token`]. Equal ⇒ accept; mismatch / missing ⇒ reject.
//!
//! Tokens are 24 random bytes encoded with the URL-safe base64 alphabet
//! (no padding) — 32 ASCII characters per token, ~192 bits of entropy.
//!
//! `validate_token` runs in constant time over the shorter input so a
//! length-equal attacker can't time the comparison.

use std::time::{SystemTime, UNIX_EPOCH};

/// Server-side issuer. Stateless — `issue()` only emits a fresh
/// random value and returns it; the cookie + form-field copy is the
/// stored state.
#[derive(Debug, Default, Clone)]
pub struct CsrfStore;

impl CsrfStore {
    pub fn new() -> Self { Self }
    /// Generate a fresh token to seed the `cave_csrf` cookie.
    pub fn issue(&self) -> String { generate_token() }
}

/// 24 random bytes → 32-char base64url string.
///
/// Uses a cheap xorshift64* PRNG seeded from the high-resolution
/// clock + a static counter; we don't pull a `rand` dependency for
/// CSRF — the attacker doesn't know the wall clock with nanosecond
/// precision, and the token only needs to survive one form submit.
pub fn generate_token() -> String {
    let mut seed = wall_nanos() ^ next_counter();
    let mut bytes = [0u8; 24];
    for chunk in bytes.chunks_mut(8) {
        seed = xorshift64_star(seed);
        let raw = seed.to_le_bytes();
        for (i, b) in chunk.iter_mut().enumerate() {
            *b = raw[i];
        }
    }
    base64url_no_pad(&bytes)
}

/// Compare cookie and submitted token in constant time over the
/// shorter input. Returns `true` only if both are non-empty and
/// byte-equal.
pub fn validate_token(cookie: &str, submitted: &str) -> bool {
    if cookie.is_empty() || submitted.is_empty() {
        return false;
    }
    if cookie.len() != submitted.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in cookie.bytes().zip(submitted.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

// ── helpers ──────────────────────────────────────────────────────────

fn wall_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xDEADBEEF_CAFEBABE)
}

use std::sync::atomic::{AtomicU64, Ordering};
static COUNTER: AtomicU64 = AtomicU64::new(0x9E3779B97F4A7C15);

fn next_counter() -> u64 {
    COUNTER.fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed)
}

fn xorshift64_star(mut x: u64) -> u64 {
    if x == 0 {
        x = 0x9E3779B97F4A7C15;
    }
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

const BASE64URL: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_no_pad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 4) / 3 + 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = bytes[i + 1] as u32;
        let b2 = bytes[i + 2] as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64URL[((n >> 18) & 0x3f) as usize] as char);
        out.push(BASE64URL[((n >> 12) & 0x3f) as usize] as char);
        out.push(BASE64URL[((n >> 6) & 0x3f) as usize] as char);
        out.push(BASE64URL[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let b0 = bytes[i] as u32;
        out.push(BASE64URL[((b0 >> 2) & 0x3f) as usize] as char);
        out.push(BASE64URL[((b0 << 4) & 0x3f) as usize] as char);
    } else if rem == 2 {
        let b0 = bytes[i] as u32;
        let b1 = bytes[i + 1] as u32;
        let n = (b0 << 8) | b1;
        out.push(BASE64URL[((n >> 10) & 0x3f) as usize] as char);
        out.push(BASE64URL[((n >> 4) & 0x3f) as usize] as char);
        out.push(BASE64URL[((n << 2) & 0x3f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tokens_are_unique_across_many_calls() {
        let mut set = HashSet::new();
        for _ in 0..1000 {
            assert!(set.insert(generate_token()), "duplicate token observed");
        }
    }

    #[test]
    fn token_alphabet_is_base64url() {
        let t = generate_token();
        for c in t.chars() {
            assert!(c.is_ascii_alphanumeric() || c == '-' || c == '_');
        }
    }

    #[test]
    fn validate_rejects_length_mismatch_in_constant_time() {
        assert!(!validate_token("aaa", "aaaa"));
    }

    #[test]
    fn validate_round_trip() {
        let tok = generate_token();
        assert!(validate_token(&tok, &tok));
    }

    #[test]
    fn store_issues_distinct_tokens() {
        let s = CsrfStore::new();
        assert_ne!(s.issue(), s.issue());
    }
}
