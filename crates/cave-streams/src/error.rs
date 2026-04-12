//! Error types for cave-streams.

use thiserror::Error;

pub type StreamResult<T> = Result<T, StreamError>;

#[derive(Debug, Error, Clone)]
pub enum StreamError {
    #[error("Topic not found: {0}")]
    TopicNotFound(String),

    #[error("Topic already exists: {0}")]
    TopicExists(String),

    #[error("Partition {partition} not found in topic {topic}")]
    PartitionNotFound { topic: String, partition: u32 },

    #[error("Schema not found: id={0}")]
    SchemaNotFound(u32),

    #[error("Schema subject not found: {0}")]
    SubjectNotFound(String),

    #[error("Schema compatibility violation: {0}")]
    SchemaCompatibility(String),

    #[error("Schema validation failed: {0}")]
    SchemaValidation(String),

    #[error("Duplicate sequence: producer={producer_id}, partition={partition}, seq={sequence}")]
    DuplicateSequence {
        producer_id: i64,
        partition: u32,
        sequence: i32,
    },

    #[error("Out-of-order sequence: producer={producer_id}, expected={expected}, got={got}")]
    OutOfOrderSequence {
        producer_id: i64,
        expected: i32,
        got: i32,
    },

    #[error("Invalid producer epoch: expected={expected}, got={got}")]
    InvalidEpoch { expected: i16, got: i16 },

    #[error("Transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("Invalid transaction state: {0}")]
    InvalidTransactionState(String),

    #[error("Consumer group not found: {0}")]
    GroupNotFound(String),

    #[error("Member not found in group {group}: {member_id}")]
    MemberNotFound { group: String, member_id: String },

    #[error("Rebalance in progress for group {0}")]
    RebalanceInProgress(String),

    #[error("Connector not found: {0}")]
    ConnectorNotFound(String),

    #[error("Connector already exists: {0}")]
    ConnectorExists(String),

    #[error("Pipeline not found: {0}")]
    PipelineNotFound(String),

    #[error("Message too large: {size} > max {max}")]
    MessageTooLarge { size: usize, max: usize },

    #[error("Offset out of range: offset={offset}, log_start={log_start}, high_watermark={high_watermark}")]
    OffsetOutOfRange {
        offset: i64,
        log_start: i64,
        high_watermark: i64,
    },

    #[error("Producer error: {0}")]
    Producer(String),

    #[error("Transaction error: {0}")]
    Transaction(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl StreamError {
    /// Map to an HTTP status code.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::TopicNotFound(_)
            | Self::PartitionNotFound { .. }
            | Self::SchemaNotFound(_)
            | Self::SubjectNotFound(_)
            | Self::TransactionNotFound(_)
            | Self::GroupNotFound(_)
            | Self::MemberNotFound { .. }
            | Self::ConnectorNotFound(_)
            | Self::PipelineNotFound(_) => 404,

            Self::TopicExists(_) | Self::ConnectorExists(_) => 409,

            Self::SchemaCompatibility(_)
            | Self::SchemaValidation(_)
            | Self::DuplicateSequence { .. }
            | Self::OutOfOrderSequence { .. }
            | Self::InvalidEpoch { .. }
            | Self::InvalidTransactionState(_)
            | Self::MessageTooLarge { .. }
            | Self::OffsetOutOfRange { .. }
            | Self::Validation(_) => 422,

            Self::RebalanceInProgress(_) => 503,

            _ => 500,
        }
    }
}
