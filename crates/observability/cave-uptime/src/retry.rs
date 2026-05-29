// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Retry logic with exponential back-off for probe execution.
//!
//! Matches Uptime Kuma's configurable retry/resend behaviour.

use std::future::Future;
use std::time::Duration;

/// Configuration for retry behaviour.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of attempts (including the first).
    pub max_attempts: u32,
    /// Base delay between retries in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum delay cap in milliseconds.
    pub max_delay_ms: u64,
    /// Multiplicative factor applied after each failure.
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        RetryConfig {
            max_attempts: 3,
            base_delay_ms: 500,
            max_delay_ms: 30_000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Compute the sleep duration before `attempt` (0-indexed).
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        let factor = self.backoff_multiplier.powi(attempt as i32);
        let ms = (self.base_delay_ms as f64 * factor) as u64;
        ms.min(self.max_delay_ms)
    }
}

/// The result of executing a retriable operation.
#[derive(Debug)]
pub enum RetryResult<T, E> {
    Ok(T),
    Err { last_error: E, attempts: u32 },
}

/// Execute `f` up to `config.max_attempts` times.
///
/// If the closure returns `Err`, it is retried after exponential back-off.
/// Returns `Ok(value)` on the first success or `Err(last_error)` after all
/// attempts are exhausted.
pub async fn execute_with_retry<F, Fut, T, E>(config: &RetryConfig, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    let mut last_err: Option<E> = None;
    for attempt in 0..config.max_attempts {
        if attempt > 0 {
            let delay = config.delay_for_attempt(attempt - 1);
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = Some(e);
            }
        }
    }
    Err(last_err.expect("at least one attempt was made"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_grows_with_attempts() {
        let cfg = RetryConfig {
            max_attempts: 5,
            base_delay_ms: 100,
            max_delay_ms: 10_000,
            backoff_multiplier: 2.0,
        };
        let d0 = cfg.delay_for_attempt(0);
        let d1 = cfg.delay_for_attempt(1);
        let d2 = cfg.delay_for_attempt(2);
        assert_eq!(d0, 100);
        assert_eq!(d1, 200);
        assert_eq!(d2, 400);
    }

    #[test]
    fn delay_capped_at_max() {
        let cfg = RetryConfig {
            max_attempts: 10,
            base_delay_ms: 1000,
            max_delay_ms: 2000,
            backoff_multiplier: 10.0,
        };
        assert_eq!(cfg.delay_for_attempt(5), 2000);
    }

    #[tokio::test]
    async fn retry_success_first_try() {
        let cfg = RetryConfig {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 10,
            backoff_multiplier: 1.0,
        };
        let result = execute_with_retry(&cfg, || async { Ok::<u32, &str>(1) }).await;
        assert_eq!(result.unwrap(), 1);
    }

    #[tokio::test]
    async fn retry_eventual_success() {
        let cfg = RetryConfig {
            max_attempts: 3,
            base_delay_ms: 1,
            max_delay_ms: 5,
            backoff_multiplier: 1.0,
        };
        let mut n = 0u32;
        let result = execute_with_retry(&cfg, || {
            n += 1;
            let c = n;
            async move {
                if c < 3 { Err("no") } else { Ok::<u32, &str>(c) }
            }
        })
        .await;
        assert!(result.is_ok());
    }
}
