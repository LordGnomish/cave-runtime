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
