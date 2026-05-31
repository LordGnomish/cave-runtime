// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic async resource pool — bounded, idle-aware connection pooling.
//!
//! A pure-`tokio` pool with no `sqlx`/`deadpool`/`bb8` dependency. The
//! shape every cave wire client actually needs: a `max_size` ceiling, a
//! per-acquire wait deadline, and idle-eviction so stale connections
//! don't linger past `idle_timeout`.
//!
//! Mechanics:
//! - A [`tokio::sync::Semaphore`] sized to `max_size` is the hard
//!   concurrency gate. One permit == permission to hold one connection.
//! - A `Mutex<VecDeque<Idle<C::Conn>>>` freelist holds returned
//!   connections. [`Pool::acquire`] pops a still-fresh idle connection
//!   if one exists, otherwise opens a new one via [`Connect::connect`].
//! - The returned [`PooledConn`] guard returns its connection to the
//!   freelist on `Drop` and releases the permit, so capacity is the
//!   number of *live* connections, not the number opened over time.
//! - `acquire` is wrapped in [`tokio::time::timeout`] keyed on
//!   `acquire_timeout`; once the pool is at `max_size` and all
//!   connections are checked out, further acquires wait up to the
//!   deadline then fail with [`PoolError::Timeout`].
//!
//! First-party primitive — no upstream to mirror. Adopters: cave-rdbms
//! (backend connections), cave-docdb, cave-cache wire clients.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// Pool tuning knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    /// Hard ceiling on simultaneously-live connections.
    pub max_size: usize,
    /// Longest a caller waits for a connection before [`PoolError::Timeout`].
    pub acquire_timeout: Duration,
    /// Connections idle in the freelist longer than this are dropped on
    /// the next acquire rather than reused.
    pub idle_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_size: 10,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// `max_size` must be greater than zero.
    #[error("pool max_size must be > 0")]
    InvalidMaxSize,
    /// No connection became available within `acquire_timeout`.
    #[error("timed out after {0:?} waiting for a pooled connection")]
    Timeout(Duration),
    /// The connector failed to open a fresh connection.
    #[error("connect failed: {0}")]
    Connect(String),
}

/// Factory that opens one new connection. Implemented by each backend
/// client; the pool only ever calls [`Connect::connect`] when it has a
/// permit in hand and the freelist had no reusable connection.
#[async_trait]
pub trait Connect: Send + Sync + 'static {
    /// The connection type handed out by the pool.
    type Conn: Send + 'static;

    /// Open one fresh connection. Errors surface as
    /// [`PoolError::Connect`].
    async fn connect(&self) -> Result<Self::Conn, PoolError>;
}

/// A freelist entry: a connection plus the instant it was returned, so
/// we can apply `idle_timeout` on the way back out.
struct Idle<C> {
    conn: C,
    returned_at: Instant,
}

struct Shared<C: Connect> {
    connector: C,
    config: PoolConfig,
    permits: Arc<Semaphore>,
    freelist: Mutex<VecDeque<Idle<C::Conn>>>,
}

/// A bounded async connection pool. Cheap to clone — all clones share
/// one freelist + semaphore via `Arc`.
pub struct Pool<C: Connect> {
    shared: Arc<Shared<C>>,
}

impl<C: Connect> std::fmt::Debug for Pool<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("max_size", &self.shared.config.max_size)
            .field("in_use", &self.in_use())
            .finish()
    }
}

impl<C: Connect> Clone for Pool<C> {
    fn clone(&self) -> Self {
        Pool {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<C: Connect> Pool<C> {
    /// Build a pool over `connector` with `config`. Fails fast if
    /// `max_size == 0`.
    pub fn new(connector: C, config: PoolConfig) -> Result<Self, PoolError> {
        if config.max_size == 0 {
            return Err(PoolError::InvalidMaxSize);
        }
        Ok(Pool {
            shared: Arc::new(Shared {
                connector,
                config,
                permits: Arc::new(Semaphore::new(config.max_size)),
                freelist: Mutex::new(VecDeque::new()),
            }),
        })
    }

    /// Maximum number of live connections.
    pub fn max_size(&self) -> usize {
        self.shared.config.max_size
    }

    /// Connections currently checked out by callers.
    pub fn in_use(&self) -> usize {
        self.shared
            .config
            .max_size
            .saturating_sub(self.shared.permits.available_permits())
    }

    /// Connections sitting idle in the freelist, available for reuse.
    pub async fn idle(&self) -> usize {
        self.shared.freelist.lock().await.len()
    }

    /// Acquire a connection, waiting up to `acquire_timeout`.
    ///
    /// Returns a [`PooledConn`] guard whose `Drop` returns the
    /// connection to the freelist and releases the capacity permit. If
    /// the pool is saturated (all `max_size` connections checked out)
    /// for longer than `acquire_timeout`, returns [`PoolError::Timeout`].
    pub async fn acquire(&self) -> Result<PooledConn<C>, PoolError> {
        let timeout = self.shared.config.acquire_timeout;
        let permit = match tokio::time::timeout(
            timeout,
            Arc::clone(&self.shared.permits).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            // The semaphore is never closed, so the inner Result is
            // always Ok; an Err here means the timeout fired.
            Ok(Err(_)) => return Err(PoolError::Timeout(timeout)),
            Err(_elapsed) => return Err(PoolError::Timeout(timeout)),
        };

        // We hold a permit: take a fresh idle connection if one exists,
        // discarding any that aged past idle_timeout.
        let idle_timeout = self.shared.config.idle_timeout;
        let now = Instant::now();
        let reused = {
            let mut freelist = self.shared.freelist.lock().await;
            loop {
                match freelist.pop_front() {
                    Some(entry) => {
                        if now.duration_since(entry.returned_at) <= idle_timeout {
                            break Some(entry.conn);
                        }
                        // Too old — drop it and keep looking.
                        drop(entry.conn);
                    }
                    None => break None,
                }
            }
        };

        let conn = match reused {
            Some(conn) => conn,
            None => self.shared.connector.connect().await?,
        };

        Ok(PooledConn {
            conn: Some(conn),
            shared: Arc::clone(&self.shared),
            _permit: permit,
        })
    }
}

/// RAII guard for a pooled connection. Deref/DerefMut to the underlying
/// connection; returns it to the freelist on `Drop`.
pub struct PooledConn<C: Connect> {
    conn: Option<C::Conn>,
    shared: Arc<Shared<C>>,
    // Held for the guard's lifetime; released on drop, freeing a slot.
    _permit: OwnedSemaphorePermit,
}

impl<C: Connect> PooledConn<C> {
    /// Discard this connection instead of returning it to the pool.
    /// Use when the connection is known-broken so a fresh one opens on
    /// the next acquire. The capacity permit is still released.
    pub fn discard(mut self) {
        self.conn = None;
    }
}

impl<C: Connect> std::fmt::Debug for PooledConn<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledConn")
            .field("present", &self.conn.is_some())
            .finish()
    }
}

impl<C: Connect> std::ops::Deref for PooledConn<C> {
    type Target = C::Conn;
    fn deref(&self) -> &Self::Target {
        self.conn
            .as_ref()
            .expect("connection present until drop/discard")
    }
}

impl<C: Connect> std::ops::DerefMut for PooledConn<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn
            .as_mut()
            .expect("connection present until drop/discard")
    }
}

impl<C: Connect> Drop for PooledConn<C> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let entry = Idle {
                conn,
                returned_at: Instant::now(),
            };
            // Return to the freelist. `try_lock` keeps Drop non-blocking;
            // if contended, push via a blocking lock — the freelist is
            // only ever held briefly, so this can't deadlock.
            match self.shared.freelist.try_lock() {
                Ok(mut fl) => fl.push_back(entry),
                Err(_) => {
                    let shared = Arc::clone(&self.shared);
                    // Move the connection onto a detached task that puts
                    // it back once the brief lock is free.
                    tokio::spawn(async move {
                        shared.freelist.lock().await.push_back(entry);
                    });
                }
            }
        }
        // `_permit` drops here, releasing the slot for the next waiter.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Connector that hands out monotonically increasing ids and counts
    /// how many real connections it opened.
    struct CountingConnector {
        opened: Arc<AtomicUsize>,
    }

    impl CountingConnector {
        fn new() -> (Self, Arc<AtomicUsize>) {
            let opened = Arc::new(AtomicUsize::new(0));
            (
                CountingConnector {
                    opened: Arc::clone(&opened),
                },
                opened,
            )
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct TestConn {
        id: usize,
    }

    #[async_trait]
    impl Connect for CountingConnector {
        type Conn = TestConn;
        async fn connect(&self) -> Result<Self::Conn, PoolError> {
            let id = self.opened.fetch_add(1, Ordering::SeqCst);
            Ok(TestConn { id })
        }
    }

    /// Connector that always fails — proves error propagation.
    struct FailingConnector;

    #[async_trait]
    impl Connect for FailingConnector {
        type Conn = TestConn;
        async fn connect(&self) -> Result<Self::Conn, PoolError> {
            Err(PoolError::Connect("backend refused".into()))
        }
    }

    fn cfg(max_size: usize, acquire_ms: u64, idle_ms: u64) -> PoolConfig {
        PoolConfig {
            max_size,
            acquire_timeout: Duration::from_millis(acquire_ms),
            idle_timeout: Duration::from_millis(idle_ms),
        }
    }

    // --- RED-first behavioral tests -------------------------------------

    #[tokio::test]
    async fn new_rejects_zero_max_size() {
        let (c, _) = CountingConnector::new();
        let err = Pool::new(c, cfg(0, 100, 1000)).unwrap_err();
        assert!(matches!(err, PoolError::InvalidMaxSize));
    }

    #[tokio::test]
    async fn acquire_under_limit_opens_connection() {
        let (c, opened) = CountingConnector::new();
        let pool = Pool::new(c, cfg(2, 100, 10_000)).unwrap();
        assert_eq!(pool.in_use(), 0);

        let g = pool.acquire().await.unwrap();
        assert_eq!(g.id, 0);
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        assert_eq!(pool.in_use(), 1);
    }

    #[tokio::test]
    async fn max_size_blocks_until_a_conn_is_freed() {
        let (c, _) = CountingConnector::new();
        let pool = Pool::new(c, cfg(1, 1_000, 10_000)).unwrap();

        let first = pool.acquire().await.unwrap();
        assert_eq!(pool.in_use(), 1);

        // Second acquire must block while the single slot is held.
        let pool2 = pool.clone();
        let waiter = tokio::spawn(async move { pool2.acquire().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished(), "acquire must block at max_size");

        // Freeing the first connection unblocks the waiter.
        drop(first);
        let second = waiter.await.unwrap().unwrap();
        assert_eq!(pool.in_use(), 1);
        drop(second);
    }

    #[tokio::test]
    async fn acquire_times_out_when_exhausted() {
        let (c, _) = CountingConnector::new();
        let pool = Pool::new(c, cfg(1, 30, 10_000)).unwrap();

        let _held = pool.acquire().await.unwrap();
        let err = pool.acquire().await.unwrap_err();
        match err {
            PoolError::Timeout(d) => assert_eq!(d, Duration::from_millis(30)),
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn conn_is_reused_from_freelist() {
        let (c, opened) = CountingConnector::new();
        let pool = Pool::new(c, cfg(2, 100, 10_000)).unwrap();

        // Open and return one connection.
        let first = pool.acquire().await.unwrap();
        let first_id = first.id;
        drop(first);
        assert_eq!(pool.idle().await, 1);

        // Next acquire reuses it — no new connection opened.
        let second = pool.acquire().await.unwrap();
        assert_eq!(second.id, first_id);
        assert_eq!(
            opened.load(Ordering::SeqCst),
            1,
            "freelist hit must not open a new connection"
        );
        assert_eq!(pool.idle().await, 0);
    }

    #[tokio::test]
    async fn idle_timeout_evicts_stale_connection() {
        let (c, opened) = CountingConnector::new();
        // 10ms idle window so the returned conn ages out quickly.
        let pool = Pool::new(c, cfg(2, 100, 10)).unwrap();

        let g = pool.acquire().await.unwrap();
        drop(g);
        assert_eq!(pool.idle().await, 1);

        // Let it age past idle_timeout, then acquire: stale entry is
        // discarded and a fresh connection is opened.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let g2 = pool.acquire().await.unwrap();
        assert_eq!(g2.id, 1, "stale conn evicted, fresh one opened");
        assert_eq!(opened.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn connect_error_propagates() {
        let pool = Pool::new(FailingConnector, cfg(1, 100, 1000)).unwrap();
        let err = pool.acquire().await.unwrap_err();
        match err {
            PoolError::Connect(msg) => assert_eq!(msg, "backend refused"),
            other => panic!("expected Connect, got {other:?}"),
        }
        // The permit was released back, so capacity isn't leaked.
        assert_eq!(pool.in_use(), 0);
    }

    #[tokio::test]
    async fn discard_does_not_return_conn_to_freelist() {
        let (c, opened) = CountingConnector::new();
        let pool = Pool::new(c, cfg(2, 100, 10_000)).unwrap();

        let g = pool.acquire().await.unwrap();
        g.discard();
        // Discarded conn is gone; freelist empty, slot released.
        assert_eq!(pool.idle().await, 0);
        assert_eq!(pool.in_use(), 0);

        // Next acquire opens a brand-new connection.
        let g2 = pool.acquire().await.unwrap();
        assert_eq!(g2.id, 1);
        assert_eq!(opened.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn deref_mut_exposes_underlying_connection() {
        let (c, _) = CountingConnector::new();
        let pool = Pool::new(c, cfg(1, 100, 10_000)).unwrap();
        let mut g = pool.acquire().await.unwrap();
        g.id = 99;
        assert_eq!(g.id, 99);
    }

    #[tokio::test]
    async fn clone_shares_capacity() {
        let (c, _) = CountingConnector::new();
        let pool = Pool::new(c, cfg(1, 30, 10_000)).unwrap();
        let clone = pool.clone();

        let _held = pool.acquire().await.unwrap();
        // The clone sees the same exhausted semaphore and times out.
        let err = clone.acquire().await.unwrap_err();
        assert!(matches!(err, PoolError::Timeout(_)));
    }

    #[tokio::test]
    async fn max_size_three_allows_three_concurrent() {
        let (c, opened) = CountingConnector::new();
        let pool = Pool::new(c, cfg(3, 50, 10_000)).unwrap();

        let a = pool.acquire().await.unwrap();
        let b = pool.acquire().await.unwrap();
        let c_guard = pool.acquire().await.unwrap();
        assert_eq!(pool.in_use(), 3);
        assert_eq!(opened.load(Ordering::SeqCst), 3);

        // Fourth blocks/times out.
        let err = pool.acquire().await.unwrap_err();
        assert!(matches!(err, PoolError::Timeout(_)));
        drop((a, b, c_guard));
    }
}