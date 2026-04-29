use thiserror::Error;

pub type CdcResult<T> = Result<T, CdcError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CdcError {
    /// Cite: debezium-connector-common
    /// `BaseSourceTask::doStart` connection-establish failure.
    #[error("source not connected: {0}")]
    NotConnected(String),
    /// Cite: debezium-connector-postgres
    /// `PostgresStreamingChangeEventSource` invalid LSN handling.
    #[error("invalid LSN '{0}': {1}")]
    InvalidLsn(String, String),
    /// Cite: debezium-connector-mysql
    /// `MySqlStreamingChangeEventSource` invalid binlog position.
    #[error("invalid binlog position: file='{file}' pos={pos}")]
    InvalidBinlogPosition { file: String, pos: u64 },
    /// Cite: debezium-storage `FileSchemaHistory` schema-history corrupt.
    #[error("schema history corrupt: {0}")]
    SchemaHistoryCorrupt(String),
    /// Cite: debezium `EventDispatcher` schema-incompatibility error.
    #[error("schema incompatibility: {0}")]
    SchemaIncompatibility(String),
    /// Cite: debezium-connector-postgres outbox `OutboxEventRouter`
    /// — when a duplicate event id is replayed.
    #[error("duplicate outbox event id: {0}")]
    DuplicateOutboxEventId(String),
    /// cave multi-tenant invariant.
    #[error("cross-tenant access denied: store='{store}' request='{req}'")]
    CrossTenantDenied { store: String, req: String },
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("source already running")]
    AlreadyRunning,
}
