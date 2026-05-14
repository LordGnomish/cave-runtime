// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic reconcile loop primitive.
//!
//! Used by cave-controller-manager, cave-cri, cave-net (CNI plugin), and
//! anywhere else that needs the Kubernetes "level-triggered, requeue on
//! error" reconciliation pattern.
//!
//! Provides:
//!   - `Reconciler` trait with `reconcile(key) -> ReconcileResult`
//!   - `ReconcileResult` carrying optional requeue delay
//!   - `run_reconciler` task runner with bounded queue + cancellation
//!
//! Backoff is delegated to `crate::retrypolicy::BackoffStrategy` so all
//! controllers share the same exponential-with-jitter behavior.

use crate::retrypolicy::BackoffStrategy;
use async_trait::async_trait;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::time::sleep;

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("reconciler returned error: {0}")]
    Failed(String),
    #[error("queue closed")]
    Closed,
}

#[derive(Debug, Clone)]
pub enum ReconcileOutcome {
    /// Object reached desired state; no further action needed.
    Done,
    /// Re-enqueue the same key after `delay`.
    Requeue { delay: Duration },
    /// Transient failure — re-enqueue with controller-managed backoff.
    Retry { reason: String },
}

pub type ReconcileResult = Result<ReconcileOutcome, ReconcileError>;

#[async_trait]
pub trait Reconciler: Send + Sync + 'static {
    type Key: Clone + Send + Sync + std::fmt::Debug + 'static;

    async fn reconcile(&self, key: Self::Key) -> ReconcileResult;
}

#[derive(Clone)]
pub struct ReconcileQueue<K: Clone + Send + Sync + 'static> {
    tx: mpsc::Sender<K>,
}

impl<K: Clone + Send + Sync + 'static> ReconcileQueue<K> {
    pub async fn enqueue(&self, key: K) -> Result<(), ReconcileError> {
        self.tx.send(key).await.map_err(|_| ReconcileError::Closed)
    }

    pub fn try_enqueue(&self, key: K) -> Result<(), ReconcileError> {
        self.tx
            .try_send(key)
            .map_err(|_| ReconcileError::Closed)
    }
}

pub struct ReconcileLoopConfig {
    pub queue_capacity: usize,
    pub backoff: BackoffStrategy,
    pub max_attempts: u32,
}

impl Default for ReconcileLoopConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 256,
            backoff: BackoffStrategy::Exponential {
                base: Duration::from_millis(100),
                cap: Duration::from_secs(60),
            },
            max_attempts: 5,
        }
    }
}

/// Spawn a reconcile loop. Returns a handle for enqueueing keys and a
/// cancellation token; the loop terminates when the queue is closed (handle
/// dropped) or `cancel.cancelled()` resolves.
pub fn run_reconciler<R: Reconciler>(
    reconciler: Arc<R>,
    config: ReconcileLoopConfig,
    cancel: tokio_util::sync::CancellationToken,
) -> (ReconcileQueue<R::Key>, tokio::task::JoinHandle<()>)
where
    R::Key: 'static,
{
    let (tx, mut rx) = mpsc::channel::<R::Key>(config.queue_capacity);
    let queue = ReconcileQueue { tx: tx.clone() };
    let internal_tx = tx;

    let handle = tokio::spawn(async move {
        let mut rng = StdRng::from_entropy();
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                maybe_key = rx.recv() => {
                    let Some(key) = maybe_key else { break };
                    process_one(&*reconciler, key, &config, &internal_tx, &mut rng).await;
                }
            }
        }
    });

    (queue, handle)
}

async fn process_one<R: Reconciler>(
    reconciler: &R,
    key: R::Key,
    config: &ReconcileLoopConfig,
    tx: &mpsc::Sender<R::Key>,
    rng: &mut StdRng,
) {
    let mut attempt: u32 = 0;
    let mut prev = Duration::from_millis(0);
    loop {
        match reconciler.reconcile(key.clone()).await {
            Ok(ReconcileOutcome::Done) => return,
            Ok(ReconcileOutcome::Requeue { delay }) => {
                let key = key.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    sleep(delay).await;
                    let _ = tx.send(key).await;
                });
                return;
            }
            Ok(ReconcileOutcome::Retry { .. }) | Err(_) => {
                attempt = attempt.saturating_add(1);
                if attempt >= config.max_attempts {
                    return;
                }
                let delay = config.backoff.delay_for(attempt - 1, prev, rng);
                prev = delay;
                sleep(delay).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio_util::sync::CancellationToken;

    struct CountingReconciler {
        calls: Arc<AtomicUsize>,
        outcome: ReconcileOutcome,
    }

    #[async_trait]
    impl Reconciler for CountingReconciler {
        type Key = u32;
        async fn reconcile(&self, _key: u32) -> ReconcileResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.outcome.clone())
        }
    }

    #[tokio::test]
    async fn done_outcome_runs_once_per_key() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = Arc::new(CountingReconciler {
            calls: calls.clone(),
            outcome: ReconcileOutcome::Done,
        });
        let cancel = CancellationToken::new();
        let (queue, handle) = run_reconciler(
            r,
            ReconcileLoopConfig::default(),
            cancel.clone(),
        );
        queue.enqueue(1).await.unwrap();
        queue.enqueue(2).await.unwrap();
        queue.enqueue(3).await.unwrap();
        // Allow the loop to drain.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        cancel.cancel();
        handle.await.unwrap();
    }

    struct FailReconciler {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Reconciler for FailReconciler {
        type Key = u32;
        async fn reconcile(&self, _key: u32) -> ReconcileResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(ReconcileError::Failed("boom".into()))
        }
    }

    #[tokio::test]
    async fn retry_caps_at_max_attempts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = Arc::new(FailReconciler { calls: calls.clone() });
        let cfg = ReconcileLoopConfig {
            max_attempts: 3,
            backoff: BackoffStrategy::Constant(Duration::from_millis(1)),
            queue_capacity: 8,
        };
        let cancel = CancellationToken::new();
        let (queue, handle) = run_reconciler(r, cfg, cancel.clone());
        queue.enqueue(42).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        cancel.cancel();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_terminates_loop() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = Arc::new(CountingReconciler {
            calls,
            outcome: ReconcileOutcome::Done,
        });
        let cancel = CancellationToken::new();
        let (_queue, handle) = run_reconciler(
            r,
            ReconcileLoopConfig::default(),
            cancel.clone(),
        );
        cancel.cancel();
        handle.await.unwrap();
    }
}
