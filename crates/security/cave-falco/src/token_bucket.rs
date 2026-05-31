// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Token-bucket rate limiter.
//!
//! NOTICE: upstream is falcosecurity/libs `userspace/libsinsp/token_bucket.cpp`
//! (Apache-2.0). Falco's `falco_outputs` uses this bucket to throttle the
//! alert stream (`outputs: { rate, max_burst }`). This is a faithful,
//! `now`-injectable port (no wall-clock dependency) so it can be tested and
//! driven deterministically by the event timeline.

const NS_PER_SEC: f64 = 1_000_000_000.0;

/// A leaky/token bucket: `rate` tokens accrue per second up to `max_tokens`,
/// and each `claim` debits one (or more) token(s) if available.
///
/// Mirrors `libsinsp::token_bucket` (`m_rate`, `m_max_tokens`, `m_tokens`,
/// `m_last_seen`). The wall-clock `m_timer` is replaced by an explicit
/// `now_ns` argument so the bucket is deterministic.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenBucket {
    rate: f64,
    max_tokens: f64,
    tokens: f64,
    last_seen: u64,
}

impl TokenBucket {
    /// `init(rate, max_tokens, now)` — the bucket starts **full**
    /// (`tokens = max_tokens`), matching libsinsp's `init`.
    pub fn new(rate: f64, max_tokens: f64, now_ns: u64) -> Self {
        Self { rate, max_tokens, tokens: max_tokens, last_seen: now_ns }
    }

    /// Re-initialise in place (libsinsp `init`).
    pub fn init(&mut self, rate: f64, max_tokens: f64, now_ns: u64) {
        self.rate = rate;
        self.max_tokens = max_tokens;
        self.tokens = max_tokens;
        self.last_seen = now_ns;
    }

    /// Attempt to claim `tokens` at time `now_ns`. First accrues
    /// `rate * elapsed_seconds` (capped at `max_tokens`), then debits the
    /// requested amount if the balance covers it.
    ///
    /// Returns `true` if the claim succeeded, `false` if throttled.
    pub fn claim_at(&mut self, tokens: f64, now_ns: u64) -> bool {
        let elapsed_ns = now_ns.saturating_sub(self.last_seen) as f64;
        let gained = self.rate * (elapsed_ns / NS_PER_SEC);
        self.last_seen = now_ns;
        self.tokens += gained;
        if self.tokens > self.max_tokens {
            self.tokens = self.max_tokens;
        }
        if tokens <= self.tokens {
            self.tokens -= tokens;
            true
        } else {
            false
        }
    }

    /// Claim a single token (libsinsp `claim()`).
    pub fn claim(&mut self, now_ns: u64) -> bool {
        self.claim_at(1.0, now_ns)
    }

    pub fn tokens(&self) -> f64 { self.tokens }
    pub fn last_seen(&self) -> u64 { self.last_seen }
    pub fn rate(&self) -> f64 { self.rate }
    pub fn max_tokens(&self) -> f64 { self.max_tokens }
}

impl Default for TokenBucket {
    /// libsinsp default constructor calls `init(1, 1)`.
    fn default() -> Self {
        Self::new(1.0, 1.0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: u64 = 1_000_000_000;

    #[test]
    fn new_bucket_starts_full() {
        let tb = TokenBucket::new(1.0, 5.0, 0);
        assert_eq!(tb.tokens(), 5.0);
        assert_eq!(tb.last_seen(), 0);
    }

    #[test]
    fn claim_when_full_succeeds_and_decrements() {
        let mut tb = TokenBucket::new(1.0, 5.0, 0);
        assert!(tb.claim(0));
        assert_eq!(tb.tokens(), 4.0);
    }

    #[test]
    fn claim_fails_when_drained_within_same_instant() {
        // max_burst = 2, rate 1/s. Two claims at t=0 drain it; the third
        // (still t=0, no tokens regenerated) must fail.
        let mut tb = TokenBucket::new(1.0, 2.0, 0);
        assert!(tb.claim(0));
        assert!(tb.claim(0));
        assert!(!tb.claim(0));
    }

    #[test]
    fn tokens_regenerate_at_rate_over_elapsed_ns() {
        let mut tb = TokenBucket::new(2.0, 10.0, 0);
        // drain to 0
        for _ in 0..10 { assert!(tb.claim(0)); }
        assert!(!tb.claim(0));
        // after 1 second at rate=2, 2 tokens accrue
        assert!(tb.claim(SEC));
        assert!(tb.claim(SEC));
        assert!(!tb.claim(SEC));
    }

    #[test]
    fn tokens_are_capped_at_max() {
        let mut tb = TokenBucket::new(1.0, 3.0, 0);
        // 100 seconds at rate 1 would give 100, but cap is 3.
        assert!(tb.claim_at(0.0, 100 * SEC)); // accrue + cap, claim 0
        assert_eq!(tb.tokens(), 3.0);
    }

    #[test]
    fn claim_arbitrary_amount() {
        let mut tb = TokenBucket::new(1.0, 10.0, 0);
        assert!(tb.claim_at(7.0, 0));
        assert_eq!(tb.tokens(), 3.0);
        assert!(!tb.claim_at(4.0, 0));
        assert!(tb.claim_at(3.0, 0));
    }

    #[test]
    fn default_is_rate_one_max_one() {
        let tb = TokenBucket::default();
        assert_eq!(tb.tokens(), 1.0);
    }
}
