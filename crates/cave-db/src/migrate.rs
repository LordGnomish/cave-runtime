//! Migration trait — each module implements this to declare its DB migrations.

use crate::CavePool;

/// Trait for module migrations.
#[async_trait::async_trait]
pub trait CaveMigrations {
    /// Module name (used as schema name prefix).
    fn module_name(&self) -> &'static str;

    /// List of (version, sql) pairs in order.
    fn migrations(&self) -> Vec<(i32, &'static str)>;

    /// Run all pending migrations.
    async fn run(&self, pool: &CavePool) -> Result<(), String> {
        pool.ensure_schema(self.module_name()).await?;
        for (version, sql) in self.migrations() {
            pool.migrate(self.module_name(), version, sql).await?;
        }
        Ok(())
    }
}
