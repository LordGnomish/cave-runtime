// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("instance not found: {0}")]
    InstanceNotFound(String),
    #[error("instance already exists: {0}")]
    InstanceExists(String),
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("backup failed: {0}")]
    BackupFailed(String),
    #[error("replication error: {0}")]
    ReplicationError(String),
    #[error("user error: {0}")]
    UserError(String),
    #[error("config error: {0}")]
    ConfigError(String),
    #[error("operation timeout")]
    Timeout,
    #[error("postgres error: {0}")]
    Postgres(#[from] tokio_postgres::Error),
}

pub type PgResult<T> = Result<T, PgError>;
