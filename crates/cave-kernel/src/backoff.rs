// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Backoff strategies — pure-function delay schedules.
//!
//! Distinct from `retrypolicy::BackoffStrategy` (which is owned by
//! the retry executor and includes jitter for retried operations).
//! This module ships strategies the *outlier-ejection* path needs
//! — Envoy-style "wait longer before re-introducing a host" cadence,
//! sweep-005's deferred unblocker.
//!
//! Strategies:
//! * [`Backoff::Constant`] — fixed delay.
//! * [`Backoff::Linear`] — `base * (n+1)`.
//! * [`Backoff::Exponential`] — `base * 2^n`, capped.
//! * [`Backoff::Fibonacci`] — golden-ratio growth (`base * F(n+2)`),
//!   capped; matches AWS SDK's recommendation for "kinder" growth
//!   between linear and exponential.
//!
//! All variants cap at `Duration::MAX` and saturate, not panic, on
//! overflow.
//!
//! Adopters: cave-mesh outlier ejection (sweep-011 close-out).

use std::time::Duration;

/// Backoff schedule. Each variant takes a `base` step and any
/// extra parameters (cap, jitter range) inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backoff {
    Constant(Duration),
    Linear { base: Duration, cap: Duration },
    Exponential { base: Duration, cap: Duration },
    Fibonacci { base: Duration, cap: Duration },
}

impl Backoff {
    /// Delay before retry `n` (0-based — `delay_for(0)` is the
    /// FIRST retry's wait, not the initial attempt).
    pub fn delay_for(&self, n: u32) -> Duration {
        match *self {
            Backoff::Constant(d) => d,
            Backoff::Linear { base, cap } => {
                let mult = (n as u64).saturating_add(1);
                base.checked_mul(mult.try_into().unwrap_or(u32::MAX))
                    .unwrap_or(cap)
                    .min(cap)
            }
            Backoff::Exponential { base, cap } => {
                let shift = n.min(63);
                let mult: u64 = 1u64 << shift;
                let mult_u32: u32 = mult.try_into().unwrap_or(u32::MAX);
                base.checked_mul(mult_u32).unwrap_or(cap).min(cap)
            }
            Backoff::Fibonacci { base, cap } => {
                let f = fib(n + 1);
                let mult: u32 = f.try_into().unwrap_or(u32::MAX);
                base.checked_mul(mult).unwrap_or(cap).min(cap)
            }
        }
    }

    /// Generate the schedule for the first `n` retries.
    pub fn schedule(&self, n: u32) -> Vec<Duration> {
        (0..n).map(|i| self.delay_for(i)).collect()
    }
}

/// Fibonacci(n) — F(0) = 0, F(1) = 1. Returned as u64 so the
/// multiplier doesn't overflow until `n > 92` (the F(93) point).
fn fib(n: u32) -> u64 {
    let mut a: u64 = 0;
    let mut b: u64 = 1;
    for _ in 0..n {
        let c = a.saturating_add(b);
        a = b;
        b = c;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_returns_same_delay_for_every_n() {
        let b = Backoff::Constant(Duration::from_millis(100));
        for n in 0..10 {
            assert_eq!(b.delay_for(n), Duration::from_millis(100));
        }
    }

    #[test]
    fn linear_grows_by_one_step_each_retry() {
        let b = Backoff::Linear {
            base: Duration::from_millis(100),
            cap: Duration::from_secs(10),
        };
        assert_eq!(b.delay_for(0), Duration::from_millis(100));
        assert_eq!(b.delay_for(1), Duration::from_millis(200));
        assert_eq!(b.delay_for(2), Duration::from_millis(300));
    }

    #[test]
    fn linear_caps_at_max() {
        let b = Backoff::Linear {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(3),
        };
        assert_eq!(b.delay_for(0), Duration::from_secs(1));
        assert_eq!(b.delay_for(1), Duration::from_secs(2));
        assert_eq!(b.delay_for(2), Duration::from_secs(3));
        assert_eq!(b.delay_for(5), Duration::from_secs(3));
    }

    #[test]
    fn exponential_doubles_each_step() {
        let b = Backoff::Exponential {
            base: Duration::from_millis(50),
            cap: Duration::from_secs(10),
        };
        assert_eq!(b.delay_for(0), Duration::from_millis(50));
        assert_eq!(b.delay_for(1), Duration::from_millis(100));
        assert_eq!(b.delay_for(2), Duration::from_millis(200));
        assert_eq!(b.delay_for(3), Duration::from_millis(400));
    }

    #[test]
    fn exponential_caps_at_max() {
        let b = Backoff::Exponential {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(8),
        };
        assert_eq!(b.delay_for(3), Duration::from_secs(8));
        assert_eq!(b.delay_for(20), Duration::from_secs(8));
    }

    #[test]
    fn exponential_saturates_on_huge_n() {
        let b = Backoff::Exponential {
            base: Duration::from_millis(1),
            cap: Duration::from_secs(60),
        };
        // n=63 would overflow u32 multiplier — must saturate to cap.
        assert_eq!(b.delay_for(63), Duration::from_secs(60));
        assert_eq!(b.delay_for(u32::MAX), Duration::from_secs(60));
    }

    #[test]
    fn fibonacci_grows_at_phi_ratio() {
        let b = Backoff::Fibonacci {
            base: Duration::from_millis(100),
            cap: Duration::from_secs(60),
        };
        // F(1)=1, F(2)=1, F(3)=2, F(4)=3, F(5)=5
        assert_eq!(b.delay_for(0), Duration::from_millis(100)); // base * F(1)=1
        assert_eq!(b.delay_for(1), Duration::from_millis(100)); // base * F(2)=1
        assert_eq!(b.delay_for(2), Duration::from_millis(200)); // base * F(3)=2
        assert_eq!(b.delay_for(3), Duration::from_millis(300)); // base * F(4)=3
        assert_eq!(b.delay_for(4), Duration::from_millis(500)); // base * F(5)=5
    }

    #[test]
    fn fibonacci_caps_at_max() {
        let b = Backoff::Fibonacci {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(10),
        };
        // F(11) = 89, base*89 > cap → cap
        assert_eq!(b.delay_for(10), Duration::from_secs(10));
    }

    #[test]
    fn schedule_emits_n_delays() {
        let b = Backoff::Linear {
            base: Duration::from_millis(10),
            cap: Duration::from_secs(1),
        };
        let s = b.schedule(5);
        assert_eq!(s.len(), 5);
        assert_eq!(s[0], Duration::from_millis(10));
        assert_eq!(s[4], Duration::from_millis(50));
    }

    #[test]
    fn schedule_zero_returns_empty() {
        assert!(
            Backoff::Constant(Duration::from_millis(1))
                .schedule(0)
                .is_empty()
        );
    }

    #[test]
    fn fib_helper_handles_zero_and_one() {
        assert_eq!(fib(0), 0);
        assert_eq!(fib(1), 1);
        assert_eq!(fib(10), 55);
    }
}
