// SPDX-License-Identifier: AGPL-3.0-or-later
//
// DPoP-Nonce — RFC 9449 §8.
//
// The server may challenge a client with `DPoP-Nonce: <nonce>` and
// `WWW-Authenticate: DPoP error="use_dpop_nonce"` to require the client to
// include a `nonce` claim in its next proof.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// In-memory store of "active" nonces. The semantics match Keycloak's
/// default policy: a server-issued nonce is accepted until `ttl` elapses.
#[derive(Clone)]
pub struct NonceStore {
    inner: Arc<Mutex<NonceStoreInner>>,
    ttl: Duration,
}

struct NonceStoreInner {
    /// (nonce -> issued_at)
    nonces: std::collections::HashMap<String, Instant>,
    seen_jtis: HashSet<String>,
}

impl NonceStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(NonceStoreInner {
                nonces: Default::default(),
                seen_jtis: Default::default(),
            })),
            ttl,
        }
    }

    /// Issue a fresh nonce — uses a 24-byte random value, base64url encoded.
    /// Caller is expected to deliver it via the `DPoP-Nonce` response header.
    pub fn issue(&self) -> String {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
        let mut buf = [0u8; 24];
        // We avoid pulling in OsRng directly — rand::random gives us 24
        // bytes of cryptographically secure randomness from the thread RNG.
        for byte in buf.iter_mut() {
            *byte = rand::random();
        }
        let n = B64.encode(buf);
        let mut g = self.inner.lock().unwrap();
        g.nonces.insert(n.clone(), Instant::now());
        n
    }

    /// Verify a client-presented nonce. Returns `true` if it was issued and
    /// has not expired.
    pub fn verify(&self, presented: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        // Expire old entries first (cheap sweep).
        let ttl = self.ttl;
        g.nonces.retain(|_, t| t.elapsed() < ttl);
        g.nonces.contains_key(presented)
    }

    /// Record a `jti` — returns `true` if this is the first time we have
    /// seen it (RFC 9449 §11.1 replay protection).
    pub fn record_jti(&self, jti: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        g.seen_jtis.insert(jti.to_string())
    }
}

/// Render the `WWW-Authenticate` value used in DPoP nonce challenges
/// (RFC 9449 §7.1).
pub fn www_authenticate_use_dpop_nonce() -> &'static str {
    r#"DPoP error="use_dpop_nonce", error_description="resource server requires nonce in DPoP proof""#
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc9449 §8 — a server-issued nonce verifies positive.
    #[test]
    fn issued_nonce_verifies() {
        let s = NonceStore::new(Duration::from_secs(60));
        let n = s.issue();
        assert!(s.verify(&n));
    }

    // upstream: rfc9449 §8 — an unknown nonce is rejected.
    #[test]
    fn unknown_nonce_rejected() {
        let s = NonceStore::new(Duration::from_secs(60));
        assert!(!s.verify("never-issued"));
    }

    // upstream: rfc9449 §11.1 — a jti must be unique per server. The first
    // record returns true, the second returns false (= replay).
    #[test]
    fn jti_replay_detected() {
        let s = NonceStore::new(Duration::from_secs(60));
        assert!(s.record_jti("jti-1"));
        assert!(!s.record_jti("jti-1"));
        assert!(s.record_jti("jti-2"));
    }

    // upstream: rfc9449 §7.1 — challenge header includes
    // `error="use_dpop_nonce"`.
    #[test]
    fn www_authenticate_format() {
        let v = www_authenticate_use_dpop_nonce();
        assert!(v.starts_with("DPoP "));
        assert!(v.contains("error=\"use_dpop_nonce\""));
    }
}
