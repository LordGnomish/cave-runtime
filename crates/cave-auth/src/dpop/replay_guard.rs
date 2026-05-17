// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/+ RFC 9449 §11.1
//
//! In-memory `jti` replay-guard for DPoP proofs.
//!
//! Per RFC 9449 §11.1, a server MUST reject a DPoP proof whose `jti` has been
//! seen within the acceptance window. This implementation:
//!
//!   - Uses a `HashMap<String, i64>` of `jti -> first-seen-iat`.
//!   - Garbage-collects expired entries on each `record_or_replay()` call.
//!   - Default window of 60 seconds, matching Keycloak's `dpop.lifespan`.

use std::collections::HashMap;
use std::sync::Mutex;

/// Result of attempting to record a `jti` against the replay window.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplayResult {
    /// First time we've seen this `jti` — proceed.
    Fresh,
    /// Within the window AND we've seen this `jti` before — REJECT.
    Replayed,
}

/// In-memory replay guard.
///
/// `clock_skew_seconds` defines both the acceptance window (we reject proofs
/// older than `clock_skew_seconds` AND any seen-jti within the window).
pub struct ReplayGuard {
    window_seconds: i64,
    seen: Mutex<HashMap<String, i64>>,
}

impl ReplayGuard {
    pub fn new(window_seconds: i64) -> Self {
        Self {
            window_seconds,
            seen: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_default_window() -> Self {
        Self::new(60)
    }

    /// Records a `jti` observation. Returns [`ReplayResult::Replayed`] if the
    /// `jti` is already in the table AND the observation is still within the
    /// window.
    ///
    /// `now_seconds` is taken explicitly so tests can drive a deterministic
    /// clock — production callers pass `chrono::Utc::now().timestamp()`.
    pub fn record_or_replay(&self, jti: &str, now_seconds: i64) -> ReplayResult {
        let mut guard = self.seen.lock().expect("ReplayGuard mutex poisoned");
        // GC: drop any entry whose first-seen-iat is older than the window.
        guard.retain(|_, iat| now_seconds - *iat <= self.window_seconds);
        if guard.contains_key(jti) {
            return ReplayResult::Replayed;
        }
        guard.insert(jti.to_string(), now_seconds);
        ReplayResult::Fresh
    }

    /// Number of `jti`s currently tracked (after GC).
    #[cfg(test)]
    pub fn len(&self, now_seconds: i64) -> usize {
        let mut guard = self.seen.lock().unwrap();
        guard.retain(|_, iat| now_seconds - *iat <= self.window_seconds);
        guard.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_observation_is_fresh() {
        let g = ReplayGuard::with_default_window();
        assert_eq!(g.record_or_replay("jti-1", 100), ReplayResult::Fresh);
    }

    #[test]
    fn repeat_within_window_is_replay() {
        let g = ReplayGuard::with_default_window();
        assert_eq!(g.record_or_replay("jti-1", 100), ReplayResult::Fresh);
        assert_eq!(g.record_or_replay("jti-1", 110), ReplayResult::Replayed);
    }

    #[test]
    fn distinct_jtis_are_independent() {
        let g = ReplayGuard::with_default_window();
        assert_eq!(g.record_or_replay("a", 100), ReplayResult::Fresh);
        assert_eq!(g.record_or_replay("b", 100), ReplayResult::Fresh);
        assert_eq!(g.record_or_replay("c", 100), ReplayResult::Fresh);
    }

    #[test]
    fn observation_outside_window_evicts_old_jti() {
        let g = ReplayGuard::new(60);
        g.record_or_replay("jti-old", 100);
        // 200 - 100 = 100s > 60s window → should be evicted on next call.
        assert_eq!(g.record_or_replay("jti-new", 200), ReplayResult::Fresh);
        assert_eq!(
            g.record_or_replay("jti-old", 200),
            ReplayResult::Fresh,
            "old jti should be considered evicted"
        );
    }

    #[test]
    fn gc_shrinks_storage() {
        let g = ReplayGuard::new(10);
        for i in 0..5 {
            g.record_or_replay(&format!("jti-{i}"), 100);
        }
        assert_eq!(g.len(100), 5);
        // Move past the window
        assert_eq!(g.len(200), 0);
    }
}
