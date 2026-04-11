//! Migration runner — each module declares its migrations as (version, sql) pairs.

use crate::CavePool;

/// Run all pending migrations for a module.
pub async fn run_migrations(
    pool: &CavePool,
    module: &str,
    migrations: &[(i32, &str)],
) -> Result<(), String> {
    pool.ensure_schema(module).await?;
    for (version, sql) in migrations {
        pool.migrate(module, *version, sql).await?;
    }
    Ok(())
}
