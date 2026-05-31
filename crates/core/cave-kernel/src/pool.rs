// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

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