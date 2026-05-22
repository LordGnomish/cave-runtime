// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Async semaphore — bounded concurrency primitive.
//!
//! Mirrors `tokio::sync::Semaphore` reduced to the surface every
//! cave module actually consumes: `acquire`, `try_acquire`,
//! `available_permits`, plus a `Permit` RAII guard that releases
//! on drop. Re-exported behaviour matches tokio so callers can swap
//! in a tokio semaphore without code changes.
//!
//! The sweep-006 recon flagged cave-mesh's `rate_limit.rs` as a
//! semaphore-shaped use site (max-concurrent destination connections
//! rather than rate-of-arrival). This primitive is the kernel home
//! for that shape; per-destination rate-of-arrival keeps using the
//! existing `cave_kernel::ratelimiter::TokenBucket`.
//!
//! Adopters: cave-mesh `proxy::DestinationLimiter` (sweep-010).

use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore as TokioSemaphore};

/// Bounded async concurrency limiter. Internally backed by
/// `tokio::sync::Semaphore` so cancellation + fairness inherit
/// tokio's guarantees.
#[derive(Debug, Clone)]
pub struct Semaphore {
    inner: Arc<TokioSemaphore>,
    capacity: usize,
}

/// One outstanding permit. The wrapped tokio permit releases on
/// drop, returning the slot to the pool. `Permit` is `Send` so
/// callers can hand it across an `await` boundary.
#[derive(Debug)]
pub struct Permit {
    _inner: OwnedSemaphorePermit,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AcquireError {
    /// No permit available right now — the caller can retry.
    #[error("semaphore exhausted ({available}/{capacity} available)")]
    NoPermits { available: usize, capacity: usize },
}

impl Semaphore {
    /// New semaphore with `permits` slots. `permits == 0` builds a
    /// permanently-blocked semaphore; tokio handles that without
    /// special-casing.
    pub fn new(permits: usize) -> Self {
        Semaphore {
            inner: Arc::new(TokioSemaphore::new(permits)),
            capacity: permits,
        }
    }

    /// Capacity the semaphore was constructed with — never changes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of permits currently held by callers.
    pub fn in_use(&self) -> usize {
        self.capacity.saturating_sub(self.inner.available_permits())
    }

    /// Permits currently available — at most [`capacity`].
    pub fn available_permits(&self) -> usize {
        self.inner.available_permits()
    }

    /// Asynchronously acquire one permit. Resolves when a slot is
    /// free; the returned [`Permit`] releases on drop.
    pub async fn acquire(&self) -> Permit {
        let inner = self
            .inner
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");
        Permit { _inner: inner }
    }

    /// Try to acquire a permit without waiting. Returns
    /// [`AcquireError::NoPermits`] when the semaphore is full.
    pub fn try_acquire(&self) -> Result<Permit, AcquireError> {
        match self.inner.clone().try_acquire_owned() {
            Ok(p) => Ok(Permit { _inner: p }),
            Err(_) => Err(AcquireError::NoPermits {
                available: self.inner.available_permits(),
                capacity: self.capacity,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_starts_with_full_capacity() {
        let s = Semaphore::new(3);
        assert_eq!(s.capacity(), 3);
        assert_eq!(s.available_permits(), 3);
        assert_eq!(s.in_use(), 0);
    }

    #[tokio::test]
    async fn try_acquire_succeeds_until_exhausted() {
        let s = Semaphore::new(2);
        let _a = s.try_acquire().unwrap();
        let _b = s.try_acquire().unwrap();
        let err = s.try_acquire().unwrap_err();
        assert!(matches!(
            err,
            AcquireError::NoPermits {
                available: 0,
                capacity: 2
            }
        ));
    }

    #[tokio::test]
    async fn permit_drop_releases_slot() {
        let s = Semaphore::new(1);
        {
            let _p = s.try_acquire().unwrap();
            assert_eq!(s.available_permits(), 0);
        }
        assert_eq!(s.available_permits(), 1);
        // And the now-empty slot is acquireable again.
        let _p2 = s.try_acquire().unwrap();
    }

    #[tokio::test]
    async fn acquire_waits_then_returns_when_slot_freed() {
        let s = Semaphore::new(1);
        let held = s.try_acquire().unwrap();
        let s2 = s.clone();
        let task = tokio::spawn(async move { s2.acquire().await });
        // Give the task a chance to enter the wait queue.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(!task.is_finished());
        drop(held);
        let _p = task.await.unwrap();
        assert_eq!(s.available_permits(), 0);
    }

    #[tokio::test]
    async fn zero_capacity_semaphore_never_succeeds_try_acquire() {
        let s = Semaphore::new(0);
        assert!(matches!(
            s.try_acquire().unwrap_err(),
            AcquireError::NoPermits { .. }
        ));
    }

    #[tokio::test]
    async fn in_use_tracks_outstanding_permits() {
        let s = Semaphore::new(3);
        let _a = s.try_acquire().unwrap();
        assert_eq!(s.in_use(), 1);
        let _b = s.try_acquire().unwrap();
        assert_eq!(s.in_use(), 2);
    }

    #[tokio::test]
    async fn semaphore_clone_shares_state() {
        let s = Semaphore::new(2);
        let s2 = s.clone();
        let _a = s.try_acquire().unwrap();
        // The clone observes the same outstanding permit.
        assert_eq!(s2.available_permits(), 1);
    }

    #[tokio::test]
    async fn acquire_error_surfaces_capacity() {
        let s = Semaphore::new(1);
        let _held = s.try_acquire().unwrap();
        match s.try_acquire().unwrap_err() {
            AcquireError::NoPermits {
                available,
                capacity,
            } => {
                assert_eq!(available, 0);
                assert_eq!(capacity, 1);
            }
        }
    }
}
