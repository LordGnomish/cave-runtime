//! Self-contained PostgreSQL connection pool for cave-flags.
//!
//! Mirrors the interface of `cave-db::CavePool` but lives entirely within this crate
//! so that cave-flags can compile independently of the cave-core dependency chain.

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;
use tracing::info;

/// PostgreSQL connection pool scoped to the `cave_flags` schema.
pub struct FlagsPool {
    pool: Pool,
}

impl FlagsPool {
    /// Create a new pool from a PostgreSQL connection URL.
    ///
    /// # Arguments
    /// * `url`      — `postgres://user:pass@host/db`
    /// * `max_size` — connection pool cap (default: 20)
    pub fn new(url: impl Into<String>, max_size: Option<usize>) -> Result<Self, String> {
        let mut pg_config = Config::new();
        pg_config.url = Some(url.into());
        if let Some(max) = max_size {
            pg_config.pool = Some(deadpool_postgres::PoolConfig {
                max_size: max,
                ..Default::default()
            });
        }
        let pool = pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("cave-flags: failed to create pool: {e}"))?;
        info!(max_size = max_size.unwrap_or(20), "cave-flags pool created");
        Ok(Self { pool })
    }

    /// Obtain a connection from the pool.
    pub async fn get(
        &self,
    ) -> Result<deadpool_postgres::Object, deadpool_postgres::PoolError> {
        self.pool.get().await
    }

    /// Ensure the `cave_flags` schema exists.
    pub async fn ensure_schema(&self) -> Result<(), String> {
        let client = self.get().await.map_err(|e| e.to_string())?;
        client
            .execute(
                "CREATE SCHEMA IF NOT EXISTS \"cave_flags\"",
                &[],
            )
            .await
            .map_err(|e| format!("Failed to create cave_flags schema: {e}"))?;
        info!("cave_flags schema ensured");
        Ok(())
    }

    /// Apply a migration idempotently within the `cave_flags` schema.
    pub async fn migrate(&self, version: i32, sql: &str) -> Result<(), String> {
        let client = self.get().await.map_err(|e| e.to_string())?;

        // Create migration tracking table
        client
            .execute(
                "CREATE TABLE IF NOT EXISTS \"cave_flags\".migrations (
                    version    INT PRIMARY KEY,
                    applied_at TIMESTAMPTZ DEFAULT NOW()
                )",
                &[],
            )
            .await
            .map_err(|e| e.to_string())?;

        // Idempotency check
        let row = client
            .query_opt(
                "SELECT version FROM \"cave_flags\".migrations WHERE version = $1",
                &[&version],
            )
            .await
            .map_err(|e| e.to_string())?;

        if row.is_some() {
            return Ok(());
        }

        // Execute migration SQL
        client
            .batch_execute(&format!(
                "SET search_path TO \"cave_flags\"; {sql}"
            ))
            .await
            .map_err(|e| format!("Flags migration v{version} failed: {e}"))?;

        // Record applied version
        client
            .execute(
                "INSERT INTO \"cave_flags\".migrations (version) VALUES ($1)",
                &[&version],
            )
            .await
            .map_err(|e| e.to_string())?;

        info!(version = version, "cave-flags migration applied");
        Ok(())
    }

    /// Run all pending migrations from a versioned list.
    pub async fn run_migrations(&self, migrations: &[(i32, &str)]) -> Result<(), String> {
        self.ensure_schema().await?;
        for (version, sql) in migrations {
            self.migrate(*version, sql).await?;
        }
        Ok(())
    }
}
