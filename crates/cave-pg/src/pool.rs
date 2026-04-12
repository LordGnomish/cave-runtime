//! Connection pool — PgBouncer-equivalent session pooling for cave-pg.
//!
//! Maintains a pool of `Executor` instances that can be borrowed by server
//! connections, avoiding the cost of re-creating session state on every
//! connection.  Three pooling modes are supported:
//!
//! - **Session** — each client connection gets a dedicated backend for the
//!   lifetime of the connection.
//! - **Transaction** — a backend is checked out only for the duration of a
//!   transaction and returned to the pool on COMMIT/ROLLBACK.
//! - **Statement** — a backend is checked out and returned after each query
//!   (limited use cases; disables multi-statement transactions).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::error::{Error, PgError, Result, SqlState};
use crate::executor::Executor;
use crate::storage::Engine;

// ─────────────────────────────────────────────────────────────────────────────
// Pooling mode
// ─────────────────────────────────────────────────────────────────────────────

/// Connection pooling mode (mirrors PgBouncer pool_mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolMode {
    /// One backend per client connection for its lifetime.
    Session,
    /// Backend released to the pool after each transaction.
    Transaction,
    /// Backend released after each statement.
    Statement,
}

// ─────────────────────────────────────────────────────────────────────────────
// Pool configuration
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of pooled backends.
    pub max_size: usize,
    /// Minimum number of idle backends to maintain.
    pub min_idle: usize,
    /// Pooling mode.
    pub mode: PoolMode,
    /// How long to wait for an available backend before returning an error.
    pub acquire_timeout: Duration,
    /// Maximum lifetime of a pooled backend.
    pub max_lifetime: Duration,
    /// How long an idle backend can remain before being removed.
    pub idle_timeout: Duration,
    /// Database name for new connections.
    pub database: String,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 10,
            min_idle: 1,
            mode: PoolMode::Session,
            acquire_timeout: Duration::from_secs(30),
            max_lifetime: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(600),
            database: "postgres".to_string(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pooled backend
// ─────────────────────────────────────────────────────────────────────────────

struct PooledBackend {
    executor: Executor,
    created_at: Instant,
    last_used: Instant,
}

impl PooledBackend {
    fn new(engine: Arc<Engine>, db: &str) -> Self {
        let mut executor = Executor::new(engine);
        executor.config.current_database = db.to_string();
        Self {
            executor,
            created_at: Instant::now(),
            last_used: Instant::now(),
        }
    }

    fn is_expired(&self, config: &PoolConfig) -> bool {
        self.created_at.elapsed() > config.max_lifetime
            || self.last_used.elapsed() > config.idle_timeout
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Connection pool
// ─────────────────────────────────────────────────────────────────────────────

/// An async connection pool for cave-pg executors.
pub struct ConnectionPool {
    engine: Arc<Engine>,
    config: PoolConfig,
    idle: Mutex<VecDeque<PooledBackend>>,
    /// Limits concurrent checkouts to max_size.
    semaphore: Arc<Semaphore>,
}

impl ConnectionPool {
    /// Create a new pool with the given engine and configuration.
    pub fn new(engine: Arc<Engine>, config: PoolConfig) -> Arc<Self> {
        let semaphore = Arc::new(Semaphore::new(config.max_size));
        let pool = Arc::new(Self {
            engine: engine.clone(),
            config: config.clone(),
            idle: Mutex::new(VecDeque::new()),
            semaphore,
        });

        // Pre-warm min_idle backends
        {
            let mut idle = pool.idle.lock();
            for _ in 0..config.min_idle.min(config.max_size) {
                idle.push_back(PooledBackend::new(engine.clone(), &config.database));
            }
        }

        pool
    }

    /// Acquire an executor from the pool.
    ///
    /// Blocks up to `config.acquire_timeout`. Returns a `PoolGuard` that
    /// automatically returns the executor when dropped.
    pub async fn acquire(self: &Arc<Self>) -> Result<PoolGuard> {
        // Try to acquire the semaphore within the timeout
        let permit = tokio::time::timeout(
            self.config.acquire_timeout,
            self.semaphore.clone().acquire_owned(),
        ).await
        .map_err(|_| Error::Pg(PgError::error(
            SqlState::TOO_MANY_CONNECTIONS,
            "connection pool exhausted: could not acquire a connection within the timeout",
        )))?
        .map_err(|_| Error::Pg(PgError::error(
            SqlState::CONNECTION_FAILURE,
            "connection pool closed",
        )))?;

        // Try to pop an idle backend
        let backend = {
            let mut idle = self.idle.lock();
            // Remove expired backends
            idle.retain(|b| !b.is_expired(&self.config));
            idle.pop_front()
        };

        let backend = match backend {
            Some(b) => b,
            None => {
                debug!("pool: creating new backend");
                PooledBackend::new(self.engine.clone(), &self.config.database)
            }
        };

        Ok(PoolGuard {
            backend: Some(backend),
            pool: self.clone(),
            _permit: permit,
        })
    }

    /// Return a backend to the pool after use.
    fn release(&self, mut backend: PooledBackend) {
        backend.last_used = Instant::now();

        // Don't return expired backends
        if backend.is_expired(&self.config) {
            debug!("pool: discarding expired backend");
            return;
        }

        // Reset the executor state for transaction/statement pools
        match self.config.mode {
            PoolMode::Transaction | PoolMode::Statement => {
                // Roll back any open transaction
                if let crate::executor::TxnState::InTransaction { xid, .. } = &backend.executor.txn_state {
                    let xid = *xid;
                    if let Some(db) = backend.executor.db() {
                        db.txn_manager.abort(xid);
                    }
                    backend.executor.txn_state = crate::executor::TxnState::Idle;
                }
                // Clear portals (prepared statements can be kept for performance)
                backend.executor.portals.clear();
            }
            PoolMode::Session => {}
        }

        let mut idle = self.idle.lock();
        if idle.len() < self.config.max_size {
            idle.push_back(backend);
        }
        // If pool is full, the backend is dropped and the semaphore permit is
        // returned automatically by `PoolGuard`'s Drop impl.
    }

    /// Pool statistics.
    pub fn stats(&self) -> PoolStats {
        let idle = self.idle.lock();
        PoolStats {
            idle_count: idle.len(),
            max_size: self.config.max_size,
            available_permits: self.semaphore.available_permits(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pool guard (RAII handle)
// ─────────────────────────────────────────────────────────────────────────────

/// RAII handle for a pooled `Executor`. Returns the executor to the pool
/// when dropped.
pub struct PoolGuard {
    backend: Option<PooledBackend>,
    pool: Arc<ConnectionPool>,
    /// Held while this guard is live; released when dropped.
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl PoolGuard {
    /// Borrow the executor.
    pub fn executor(&mut self) -> &mut Executor {
        &mut self.backend.as_mut().expect("executor already returned").executor
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(backend) = self.backend.take() {
            self.pool.release(backend);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stats
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PoolStats {
    pub idle_count: usize,
    pub max_size: usize,
    /// Available permit slots (= max_size - active checkouts).
    pub available_permits: usize,
}

impl PoolStats {
    pub fn active_count(&self) -> usize {
        self.max_size - self.available_permits
    }
}
