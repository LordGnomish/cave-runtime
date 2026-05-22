// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Exponential-backoff retry helper for provider calls.
//!
//! Pure-function policy decisions kept separate from the sleep/timing so
//! tests can poke the math directly. Errors flagged as
//! [`GatewayError::UpstreamError`] with 4xx status are *not* retried — those
//! are caller errors.

use crate::error::{GatewayError, GatewayResult};
use std::future::Future;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_secs(2),
            multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    pub fn backoff_for(&self, attempt: u32) -> Duration {
        // attempt is 1-indexed (attempt 1 → initial_backoff, attempt 2 → initial*mul, ...)
        if attempt == 0 {
            return Duration::ZERO;
        }
        let base = self.initial_backoff.as_secs_f64();
        let computed = base * self.multiplier.powi((attempt - 1) as i32);
        let capped = computed.min(self.max_backoff.as_secs_f64());
        Duration::from_secs_f64(capped)
    }

    pub fn should_retry(&self, attempt: u32, err: &GatewayError) -> bool {
        if attempt >= self.max_attempts {
            return false;
        }
        match err {
            GatewayError::ProviderUnavailable { .. } => true,
            GatewayError::UpstreamError { status, .. } => {
                // Retry 408/429/5xx; never retry 4xx caller errors.
                *status == 408 || *status == 429 || *status >= 500
            }
            GatewayError::HttpClient(_) => true,
            _ => false,
        }
    }
}

/// Run `op` with retry until the policy gives up.
pub async fn with_retry<F, Fut, T>(policy: &RetryPolicy, mut op: F) -> GatewayResult<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = GatewayResult<T>>,
{
    let mut attempt = 1u32;
    loop {
        match op(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if policy.should_retry(attempt, &e) {
                    let sleep = policy.backoff_for(attempt);
                    tokio::time::sleep(sleep).await;
                    attempt += 1;
                } else {
                    return Err(e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn backoff_grows_exponentially_within_cap() {
        let p = RetryPolicy::default();
        let b1 = p.backoff_for(1);
        let b2 = p.backoff_for(2);
        let b3 = p.backoff_for(3);
        assert!(b1 < b2);
        assert!(b2 < b3);
        assert!(b3 <= p.max_backoff);
    }

    #[test]
    fn backoff_for_zero_is_zero() {
        assert_eq!(RetryPolicy::default().backoff_for(0), Duration::ZERO);
    }

    #[test]
    fn should_retry_on_provider_unavailable() {
        let p = RetryPolicy::default();
        let e = GatewayError::ProviderUnavailable {
            provider: "x".into(),
            reason: "boom".into(),
        };
        assert!(p.should_retry(1, &e));
    }

    #[test]
    fn should_not_retry_on_4xx_upstream() {
        let p = RetryPolicy::default();
        let e = GatewayError::UpstreamError {
            status: 401,
            body: "bad-key".into(),
        };
        assert!(!p.should_retry(1, &e));
    }

    #[test]
    fn should_retry_on_429_and_503() {
        let p = RetryPolicy::default();
        assert!(p.should_retry(
            1,
            &GatewayError::UpstreamError {
                status: 429,
                body: "throttle".into()
            }
        ));
        assert!(p.should_retry(
            1,
            &GatewayError::UpstreamError {
                status: 503,
                body: "down".into()
            }
        ));
    }

    #[test]
    fn should_stop_after_max_attempts() {
        let p = RetryPolicy::default();
        let e = GatewayError::ProviderUnavailable {
            provider: "x".into(),
            reason: "boom".into(),
        };
        assert!(!p.should_retry(p.max_attempts, &e));
    }

    #[tokio::test]
    async fn with_retry_recovers_after_transient_failures() {
        let p = RetryPolicy {
            max_attempts: 4,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
            multiplier: 2.0,
        };
        let calls = AtomicU32::new(0);
        let v = with_retry(&p, |_| async {
            let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 3 {
                Err(GatewayError::ProviderUnavailable {
                    provider: "x".into(),
                    reason: "retry me".into(),
                })
            } else {
                Ok::<u32, GatewayError>(42)
            }
        })
        .await
        .unwrap();
        assert_eq!(v, 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_retry_propagates_non_retryable_error() {
        let p = RetryPolicy::default();
        let calls = AtomicU32::new(0);
        let err = with_retry(&p, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async {
                Err::<(), _>(GatewayError::UpstreamError {
                    status: 400,
                    body: "bad-req".into(),
                })
            }
        })
        .await
        .unwrap_err();
        assert!(matches!(err, GatewayError::UpstreamError { status: 400, .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
