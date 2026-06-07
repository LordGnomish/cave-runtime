// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Real PostgreSQL [`SqlExecutor`] over `tokio-postgres`.
//!
//! This is the production end of the hybrid strategy: a [`SqlBackend`] driven
//! by [`PgExecutor`] speaks to a PostgreSQL-wire endpoint — cave-pg /
//! cave-rdbms or stock Postgres. Documents live in a `_jsonb jsonb` column per
//! collection table; the SQL is produced by [`crate::sql`].
//!
//! Gated behind the `pg` feature so the default build and unit suite stay
//! database-free. The DDL builder ([`create_table_ddl`]) is pure and unit
//! tested without a connection.
//!
//! [`SqlBackend`]: crate::backend::SqlBackend

use crate::backend::{ExecOutcome, SqlExecutor};
use crate::sql::SqlQuery;
use async_trait::async_trait;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls};

/// DDL to create a collection's document table if absent.
///
/// Pure (no IO) so it can be asserted in tests without a database.
pub fn create_table_ddl(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS \"{}\" (_jsonb jsonb NOT NULL)",
        table.replace('"', "\"\"")
    )
}

/// A [`SqlExecutor`] backed by a live `tokio-postgres` client.
pub struct PgExecutor {
    client: Client,
}

impl PgExecutor {
    /// Wrap an already-connected client.
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Connect to a PostgreSQL-wire endpoint (cave-pg / cave-rdbms / Postgres).
    ///
    /// The background connection task is spawned on the current tokio runtime.
    pub async fn connect(conn_str: &str) -> Result<Self, String> {
        let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            // The connection drives IO until the client is dropped.
            let _ = connection.await;
        });
        Ok(Self { client })
    }

    /// Ensure a collection's document table exists.
    pub async fn ensure_table(&self, table: &str) -> Result<(), String> {
        self.client
            .batch_execute(&create_table_ddl(table))
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl SqlExecutor for PgExecutor {
    async fn execute(&self, query: &SqlQuery) -> Result<ExecOutcome, String> {
        // Bind all parameters as text; the `::jsonb` / `::numeric` / `::int`
        // casts in the generated SQL coerce server-side (FerretDB pattern).
        let params: Vec<&(dyn ToSql + Sync)> = query
            .params
            .iter()
            .map(|s| s as &(dyn ToSql + Sync))
            .collect();

        if query.sql.trim_start().to_ascii_uppercase().starts_with("SELECT") {
            let rows = self
                .client
                .query(&query.sql, &params)
                .await
                .map_err(|e| e.to_string())?;
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                // The first (and only) projected column is the `_jsonb` doc.
                let v: serde_json::Value = row.try_get(0).map_err(|e| e.to_string())?;
                out.push(v);
            }
            Ok(ExecOutcome::Rows(out))
        } else {
            let n = self
                .client
                .execute(&query.sql, &params)
                .await
                .map_err(|e| e.to_string())?;
            Ok(ExecOutcome::Affected(n))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_quotes_and_escapes_table() {
        assert_eq!(
            create_table_ddl("users"),
            "CREATE TABLE IF NOT EXISTS \"users\" (_jsonb jsonb NOT NULL)"
        );
        assert_eq!(
            create_table_ddl("we\"ird"),
            "CREATE TABLE IF NOT EXISTS \"we\"\"ird\" (_jsonb jsonb NOT NULL)"
        );
    }
}
