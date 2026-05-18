// SPDX-License-Identifier: AGPL-3.0-or-later
//! Retry policy with exponential backoff + jitter, async retry executor.
//!
//! Upstream cite: AWS SDK `@aws-sdk/util-retry` for jitter strategies (full,
//! equal, decorrelated) — see "Exponential Backoff And Jitter"
//! (https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/).
//! `resilience4j` Retry pattern for retryable error classification.
//!
//! Strategies:
//!   - `Constant(d)`        — fixed delay
//!   - `Exponential`         — base * 2^attempt, no jitter
//!   - `FullJitter`          — uniform [0, base * 2^attempt]
//!   - `EqualJitter`         — base * 2^attempt / 2 + uniform [0, base * 2^attempt / 2]
//!   - `DecorrelatedJitter`  — uniform [base, prev * 3], capped
//!
//! Classification:
//!   - `Transient` — retry
//!   - `Permanent` — fail immediately

use rand::Rng;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Transient,
    Permanent,
}

#[derive(Debug, Clone, Copy)]
pub enum BackoffStrategy {
    Constant(Duration),
    Exponential { base: Duration, cap: Duration },
    FullJitter { base: Duration, cap: Duration },
    EqualJitter { base: Duration, cap: Duration },
    DecorrelatedJitter { base: Duration, cap: Duration },
}

impl BackoffStrategy {
    /// Returns the delay to wait before attempt `attempt` (zero-based).
    /// `prev` is the previous delay (used by decorrelated jitter; ignored otherwise).
    pub fn delay_for(&self, attempt: u32, prev: Duration, rng: &mut impl Rng) -> Duration {
        match *self {
            BackoffStrategy::Constant(d) => d,
            BackoffStrategy::Exponential { base, cap } => {
                let raw = base.saturating_mul(1u32 << attempt.min(20));
                raw.min(cap)
            }
            BackoffStrategy::FullJitter { base, cap } => {
                let exp = base.saturating_mul(1u32 << attempt.min(20)).min(cap);
                let exp_ms = exp.as_millis().max(1) as u64;
                Duration::from_millis(rng.gen_range(0..=exp_ms))
            }
            BackoffStrategy::EqualJitter { base, cap } => {
                let exp = base.saturating_mul(1u32 << attempt.min(20)).min(cap);
                let half = exp / 2;
                let half_ms = half.as_millis().max(1) as u64;
                let jit = Duration::from_millis(rng.gen_range(0..=half_ms));
                half + jit
            }
            BackoffStrategy::DecorrelatedJitter { base, cap } => {
                let prev_ms = prev.as_millis().max(base.as_millis()) as u64;
                let upper = (prev_ms * 3).max(base.as_millis() as u64);
                let chosen = rng.gen_range(base.as_millis() as u64..=upper);
                Duration::from_millis(chosen).min(cap)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub max_elapsed: Option<Duration>,
    pub strategy: BackoffStrategy,
}

impl RetryPolicy {
    pub fn new(max_attempts: u32, strategy: BackoffStrategy) -> Self {
        assert!(max_attempts >= 1, "max_attempts must be ≥ 1");
        Self {
            max_attempts,
            max_elapsed: None,
            strategy,
        }
    }

    pub fn with_max_elapsed(mut self, d: Duration) -> Self {
        self.max_elapsed = Some(d);
        self
    }

    /// Compute the schedule (sequence of delays) for `max_attempts - 1` retries.
    /// Useful for tests + scheduling preview.
    pub fn schedule(&self, rng: &mut impl Rng) -> Vec<Duration> {
        let mut prev = Duration::ZERO;
        let mut out = Vec::with_capacity(self.max_attempts as usize - 1);
        for i in 0..self.max_attempts.saturating_sub(1) {
            let d = self.strategy.delay_for(i, prev, rng);
            prev = d;
            out.push(d);
        }
        out
    }

    /// Decide whether a retry should occur given attempt number and elapsed time.
    pub fn should_retry(&self, next_attempt: u32, elapsed: Duration) -> bool {
        if next_attempt >= self.max_attempts {
            return false;
        }
        if let Some(max) = self.max_elapsed {
            if elapsed >= max {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Error)]
pub enum RetryError<E> {
    #[error("attempt {attempt}: {source}")]
    LastError { attempt: u32, source: E },
    #[error("permanent error after {attempt} attempts: {source}")]
    Permanent { attempt: u32, source: E },
    #[error("max elapsed {elapsed_ms} ms exceeded after {attempt} attempts: {source}")]
    Deadline {
        attempt: u32,
        elapsed_ms: u128,
        source: E,
    },
}

impl<E> RetryError<E> {
    pub fn into_source(self) -> E {
        match self {
            RetryError::LastError { source, .. }
            | RetryError::Permanent { source, .. }
            | RetryError::Deadline { source, .. } => source,
        }
    }
}

/// Async retry executor. Executes `f` up to `max_attempts` times with backoff.
///
/// `classify` decides if a returned error should be retried. `Transient` retries
/// (subject to backoff + max_elapsed); `Permanent` aborts immediately.
pub async fn retry<F, Fut, T, E>(
    policy: &RetryPolicy,
    classify: impl Fn(&E) -> ErrorClass,
    mut f: F,
) -> Result<T, RetryError<E>>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let started = tokio::time::Instant::now();
    let mut rng = rand::thread_rng();
    let mut prev_delay = Duration::ZERO;
    let mut attempt: u32 = 0;
    loop {
        match f(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if classify(&e) == ErrorClass::Permanent {
                    return Err(RetryError::Permanent {
                        attempt: attempt + 1,
                        source: e,
                    });
                }
                let elapsed = started.elapsed();
                let next_attempt = attempt + 1;
                if !policy.should_retry(next_attempt, elapsed) {
                    if let Some(max) = policy.max_elapsed {
                        if elapsed >= max {
                            return Err(RetryError::Deadline {
                                attempt: next_attempt,
                                elapsed_ms: elapsed.as_millis(),
                                source: e,
                            });
                        }
                    }
                    return Err(RetryError::LastError {
                        attempt: next_attempt,
                        source: e,
                    });
                }
                let delay = policy.strategy.delay_for(attempt, prev_delay, &mut rng);
                prev_delay = delay;
                tokio::time::sleep(delay).await;
                attempt = next_attempt;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(42)
    }

    /// cite: AWS exponential backoff — Constant returns same delay each attempt
    #[test]
    fn retry_acme_constant_strategy_returns_fixed_delay() {
        let tenant_id = "acme";
        let s = BackoffStrategy::Constant(Duration::from_millis(50));
        let mut r = rng();
        for i in 0..5 {
            assert_eq!(s.delay_for(i, Duration::ZERO, &mut r), Duration::from_millis(50));
        }
        let _ = tenant_id;
    }

    /// cite: AWS exponential backoff — Exponential doubles per attempt
    #[test]
    fn retry_acme_exponential_doubles_per_attempt() {
        let tenant_id = "acme";
        let s = BackoffStrategy::Exponential {
            base: Duration::from_millis(10),
            cap: Duration::from_secs(60),
        };
        let mut r = rng();
        assert_eq!(s.delay_for(0, Duration::ZERO, &mut r), Duration::from_millis(10));
        assert_eq!(s.delay_for(1, Duration::ZERO, &mut r), Duration::from_millis(20));
        assert_eq!(s.delay_for(2, Duration::ZERO, &mut r), Duration::from_millis(40));
        assert_eq!(s.delay_for(3, Duration::ZERO, &mut r), Duration::from_millis(80));
        let _ = tenant_id;
    }

    /// cite: AWS exponential backoff — Exponential capped at `cap`
    #[test]
    fn retry_globex_exponential_capped() {
        let tenant_id = "globex";
        let s = BackoffStrategy::Exponential {
            base: Duration::from_millis(10),
            cap: Duration::from_millis(100),
        };
        let mut r = rng();
        // 10 * 2^10 = 10240, capped at 100
        assert_eq!(s.delay_for(10, Duration::ZERO, &mut r), Duration::from_millis(100));
        let _ = tenant_id;
    }

    /// cite: AWS full jitter — delay ∈ [0, exp]
    #[test]
    fn retry_acme_full_jitter_within_bounds() {
        let tenant_id = "acme";
        let s = BackoffStrategy::FullJitter {
            base: Duration::from_millis(20),
            cap: Duration::from_millis(1000),
        };
        let mut r = rng();
        for attempt in 0..5 {
            let exp = (20u128 * (1u128 << attempt)).min(1000);
            for _ in 0..50 {
                let d = s.delay_for(attempt, Duration::ZERO, &mut r);
                assert!(d.as_millis() <= exp, "tenant {tenant_id} d={d:?} exp={exp}");
            }
        }
    }

    /// cite: AWS equal jitter — delay ∈ [exp/2, exp]
    #[test]
    fn retry_globex_equal_jitter_within_bounds() {
        let tenant_id = "globex";
        let s = BackoffStrategy::EqualJitter {
            base: Duration::from_millis(40),
            cap: Duration::from_millis(2000),
        };
        let mut r = rng();
        for attempt in 0..4 {
            let exp = (40u128 * (1u128 << attempt)).min(2000);
            for _ in 0..50 {
                let d = s.delay_for(attempt, Duration::ZERO, &mut r);
                assert!(
                    d.as_millis() >= exp / 2 && d.as_millis() <= exp + 1,
                    "tenant {tenant_id} d={d:?} exp={exp}"
                );
            }
        }
    }

    /// cite: AWS decorrelated jitter — delay ∈ [base, prev * 3]
    #[test]
    fn retry_acme_decorrelated_jitter_within_bounds() {
        let tenant_id = "acme";
        let s = BackoffStrategy::DecorrelatedJitter {
            base: Duration::from_millis(50),
            cap: Duration::from_secs(5),
        };
        let mut r = rng();
        let mut prev = Duration::from_millis(50);
        for _ in 0..10 {
            let d = s.delay_for(0, prev, &mut r);
            assert!(d >= Duration::from_millis(50), "tenant {tenant_id} d={d:?}");
            assert!(d <= Duration::from_secs(5));
            prev = d;
        }
    }

    /// cite: retry policy — schedule emits N-1 delays
    #[test]
    fn retry_acme_schedule_emits_n_minus_1_delays() {
        let tenant_id = "acme";
        let p = RetryPolicy::new(
            5,
            BackoffStrategy::Exponential {
                base: Duration::from_millis(1),
                cap: Duration::from_secs(60),
            },
        );
        let mut r = rng();
        let s = p.schedule(&mut r);
        assert_eq!(s.len(), 4);
        let _ = tenant_id;
    }

    /// cite: retry policy — should_retry false when attempt cap reached
    #[test]
    fn retry_globex_should_retry_caps_at_max_attempts() {
        let tenant_id = "globex";
        let p = RetryPolicy::new(3, BackoffStrategy::Constant(Duration::ZERO));
        assert!(p.should_retry(1, Duration::ZERO));
        assert!(p.should_retry(2, Duration::ZERO));
        assert!(!p.should_retry(3, Duration::ZERO));
        assert!(!p.should_retry(99, Duration::ZERO));
        let _ = tenant_id;
    }

    /// cite: retry policy — max_elapsed enforced
    #[test]
    fn retry_initech_should_retry_caps_at_max_elapsed() {
        let tenant_id = "initech";
        let p = RetryPolicy::new(10, BackoffStrategy::Constant(Duration::ZERO))
            .with_max_elapsed(Duration::from_secs(5));
        assert!(p.should_retry(1, Duration::from_secs(1)));
        assert!(!p.should_retry(1, Duration::from_secs(6)));
        let _ = tenant_id;
    }

    /// cite: classification — Permanent errors fail immediately, no retry attempted
    #[tokio::test(start_paused = true)]
    async fn retry_acme_permanent_error_aborts() {
        let tenant_id = "acme";
        let p = RetryPolicy::new(5, BackoffStrategy::Constant(Duration::from_millis(1)));
        let calls = Arc::new(AtomicU32::new(0));
        let calls_c = calls.clone();
        let res: Result<(), RetryError<&'static str>> = retry(
            &p,
            |_| ErrorClass::Permanent,
            |_attempt| {
                let c = calls_c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err::<(), &'static str>("auth failed")
                }
            },
        )
        .await;
        assert!(matches!(res, Err(RetryError::Permanent { .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = tenant_id;
    }

    /// cite: classification — Transient errors retry up to max_attempts
    #[tokio::test(start_paused = true)]
    async fn retry_globex_transient_retries_to_max() {
        let tenant_id = "globex";
        let p = RetryPolicy::new(4, BackoffStrategy::Constant(Duration::from_millis(1)));
        let calls = Arc::new(AtomicU32::new(0));
        let calls_c = calls.clone();
        let res: Result<(), RetryError<&'static str>> = retry(
            &p,
            |_| ErrorClass::Transient,
            |_attempt| {
                let c = calls_c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err::<(), &'static str>("network blip")
                }
            },
        )
        .await;
        assert!(matches!(res, Err(RetryError::LastError { .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        let _ = tenant_id;
    }

    /// cite: classification — success on second attempt returns Ok
    #[tokio::test(start_paused = true)]
    async fn retry_acme_succeeds_on_second_attempt() {
        let tenant_id = "acme";
        let p = RetryPolicy::new(3, BackoffStrategy::Constant(Duration::from_millis(1)));
        let calls = Arc::new(AtomicU32::new(0));
        let calls_c = calls.clone();
        let res: Result<&str, RetryError<&'static str>> = retry(
            &p,
            |_| ErrorClass::Transient,
            |attempt| {
                let c = calls_c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    if attempt == 0 {
                        Err::<&str, &'static str>("blip")
                    } else {
                        Ok("ok")
                    }
                }
            },
        )
        .await;
        assert_eq!(res.unwrap(), "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let _ = tenant_id;
    }

    /// cite: classification — single attempt success short-circuits
    #[tokio::test(start_paused = true)]
    async fn retry_initech_first_attempt_success_no_loop() {
        let tenant_id = "initech";
        let p = RetryPolicy::new(5, BackoffStrategy::Constant(Duration::from_millis(1)));
        let calls = Arc::new(AtomicU32::new(0));
        let calls_c = calls.clone();
        let res: Result<i32, RetryError<&'static str>> = retry(
            &p,
            |_| ErrorClass::Transient,
            |_attempt| {
                let c = calls_c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<i32, &'static str>(42)
                }
            },
        )
        .await;
        assert_eq!(res.unwrap(), 42);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = tenant_id;
    }

    /// cite: retry policy — max_elapsed deadline returns Deadline variant
    #[tokio::test(start_paused = true)]
    async fn retry_acme_deadline_variant_when_elapsed_exceeds() {
        let tenant_id = "acme";
        let p = RetryPolicy::new(100, BackoffStrategy::Constant(Duration::from_millis(50)))
            .with_max_elapsed(Duration::from_millis(120));
        let res: Result<(), RetryError<&'static str>> = retry(
            &p,
            |_| ErrorClass::Transient,
            |_attempt| async { Err::<(), &'static str>("blip") },
        )
        .await;
        assert!(
            matches!(res, Err(RetryError::Deadline { .. })),
            "tenant {tenant_id} got {res:?}"
        );
    }

    /// cite: retry error — into_source extracts underlying error
    #[test]
    fn retry_acme_error_into_source_recovers_value() {
        let tenant_id = "acme";
        let e = RetryError::LastError {
            attempt: 3,
            source: "boom",
        };
        assert_eq!(e.into_source(), "boom");
        let _ = tenant_id;
    }

    /// cite: retry policy — schedule with 1 attempt has zero delays
    #[test]
    fn retry_globex_schedule_with_one_attempt_is_empty() {
        let tenant_id = "globex";
        let p = RetryPolicy::new(1, BackoffStrategy::Constant(Duration::from_millis(10)));
        let mut r = rng();
        assert!(p.schedule(&mut r).is_empty());
        let _ = tenant_id;
    }
}
