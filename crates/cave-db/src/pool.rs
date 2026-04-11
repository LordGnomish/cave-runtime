//! Connection pool management using deadpool-postgres.

use cave_core::config::DatabaseConfig;
use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::NoTls;
use tracing::info;

/// Shared database connection pool for all modules.
pub struct CavePool {
    pool: Pool,
}

impl CavePool {
    /// Create a new connection pool from config.
    pub fn new(config: &DatabaseConfig) -> Result<Self, String> {
        let mut pg_config = Config::new();
        pg_config.url = Some(config.url.clone());

        let pool = pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("Failed to create pool: {e}"))?;

        info!(
            max_size = config.max_pool_size.unwrap_or(20),
            "Database pool created"
        );

        Ok(Self { pool })
    }

    /// Get a connection from the pool.
    pub async fn get(
        &self,
    ) -> Result<deadpool_postgres::Object, deadpool_postgres::PoolError> {
        self.pool.get().await
    }

    /// Ensure a module schema exists.
    pub async fn ensure_schema(&self, module: &str) -> Result<(), String> {
        let client = self.get().await.map_err(|e| e.to_string())?;
        let schema = format!("cave_{module}");
        client
            .execute(
                &format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\""),
                &[],
            )
            .await
            .map_err(|e| format!("Failed to create schema {schema}: {e}"))?;
        info!(schema = %schema, "Schema ensured");
        Ok(())
    }

    /// Run a migration SQL string within a module's schema.
    pub async fn migrate(&self, module: &str, version: i32, sql: &str) -> Result<(), String> {
        let client = self.get().await.map_err(|e| e.to_string())?;
        let schema = format!("cave_{module}");

        // Create migration tracking table if not exists
        client
            .execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS \"{schema}\".migrations (
                        version INT PRIMARY KEY,
                        applied_at TIMESTAMPTZ DEFAULT NOW()
                    )"
                ),
                &[],
            )
            .await
            .map_err(|e| e.to_string())?;

        // Check if already applied
        let row = client
            .query_opt(
                &format!(
                    "SELECT version FROM \"{schema}\".migrations WHERE version = $1"
                ),
                &[&version],
            )
            .await
            .map_err(|e| e.to_string())?;

        if row.is_some() {
            return Ok(()); // Already applied
        }

        // Run migration
        client
            .batch_execute(&format!("SET search_path TO \"{schema}\"; {sql}"))
            .await
            .map_err(|e| format!("Migration v{version} failed for {module}: {e}"))?;

        // Record migration
        client
            .execute(
                &format!(
                    "INSERT INTO \"{schema}\".migrations (version) VALUES ($1)"
                ),
                &[&version],
            )
            .await
            .map_err(|e| e.to_string())?;

        info!(module = module, version = version, "Migration applied");
        Ok(())
    }
}
