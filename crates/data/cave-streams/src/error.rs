// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-streams.

use thiserror::Error;

pub type StreamsResult<T> = Result<T, StreamsError>;

#[derive(Error, Debug)]
pub enum StreamsError {
    #[error("Unknown topic or partition: topic={topic}, partition={partition}")]
    UnknownTopicOrPartition { topic: String, partition: i32 },

    #[error("Topic already exists: {0}")]
    TopicAlreadyExists(String),

    #[error("Invalid topic name: {0}")]
    InvalidTopicName(String),

    #[error("Invalid replication factor {factor} for topic {topic}")]
    InvalidReplicationFactor { topic: String, factor: i16 },

    #[error("Not enough replicas: required={required}, available={available}")]
    NotEnoughReplicas { required: i16, available: i16 },

    #[error("Consumer group not found: {0}")]
    GroupNotFound(String),

    #[error("Member not found in group {group}: member={member}")]
    MemberNotFound { group: String, member: String },

    #[error("Rebalance in progress for group: {0}")]
    RebalanceInProgress(String),

    #[error("Invalid generation for group {group}: expected={expected}, got={got}")]
    IllegalGeneration {
        group: String,
        expected: i32,
        got: i32,
    },

    #[error("Unknown protocol type: {0}")]
    InconsistentGroupProtocol(String),

    #[error("Producer ID not found: {0}")]
    ProducerIdNotFound(i64),

    #[error("Duplicate sequence: producer={producer_id}, topic={topic}, partition={partition}")]
    DuplicateSequenceNumber {
        producer_id: i64,
        topic: String,
        partition: i32,
    },

    #[error("Invalid transaction state: {0}")]
    InvalidTxnState(String),

    #[error("Schema not found: id={0}")]
    SchemaNotFound(i32),

    #[error("Subject not found: {0}")]
    SubjectNotFound(String),

    #[error("Schema incompatible with subject {subject}: {reason}")]
    SchemaIncompatible { subject: String, reason: String },

    #[error("Connector not found: {0}")]
    ConnectorNotFound(String),

    #[error("Connector already exists: {0}")]
    ConnectorAlreadyExists(String),

    #[error("ACL not found for resource {resource}")]
    AclNotFound { resource: String },

    #[error("Quota exceeded for {principal}: {reason}")]
    QuotaViolation { principal: String, reason: String },

    #[error("Compression error ({codec}): {message}")]
    Compression { codec: String, message: String },

    #[error("Protocol decode error: {0}")]
    ProtocolDecode(String),

    #[error("Protocol encode error: {0}")]
    ProtocolEncode(String),

    #[error("Offset out of range: topic={topic}, partition={partition}, offset={offset}")]
    OffsetOutOfRange {
        topic: String,
        partition: i32,
        offset: i64,
    },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Kafka error codes (subset of official codes used in responses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum KafkaErrorCode {
    None = 0,
    OffsetOutOfRange = 1,
    CorruptMessage = 2,
    UnknownTopicOrPartition = 3,
    InvalidFetchSize = 4,
    NotLeaderOrFollower = 6,
    RequestTimedOut = 7,
    NotEnoughReplicas = 19,
    NotEnoughReplicasAfterAppend = 20,
    InvalidRequiredAcks = 21,
    IllegalGeneration = 22,
    InconsistentGroupProtocol = 23,
    InvalidGroupId = 24,
    UnknownMemberId = 25,
    InvalidSessionTimeout = 26,
    RebalanceInProgress = 27,
    TopicAlreadyExists = 36,
    InvalidTopicException = 17,
    RecordListTooLarge = 18,
    GroupAuthorizationFailed = 30,
    ClusterAuthorizationFailed = 31,
    InvalidTimestamp = 32,
    DuplicateSequenceNumber = 46,
    InvalidProducerIdMapping = 47,
    InvalidTransactionTimeout = 48,
    ConcurrentTransactions = 49,
    TransactionCoordinatorFenced = 52,
    TransactionalIdAuthorizationFailed = 53,
    SecurityDisabled = 54,
    ProducerFenced = 56,
    UnknownProducerId = 59,
    SchemasNotCompatible = -1000,
}

impl StreamsError {
    pub fn kafka_error_code(&self) -> i16 {
        match self {
            Self::UnknownTopicOrPartition { .. } => KafkaErrorCode::UnknownTopicOrPartition as i16,
            Self::TopicAlreadyExists(_) => KafkaErrorCode::TopicAlreadyExists as i16,
            Self::OffsetOutOfRange { .. } => KafkaErrorCode::OffsetOutOfRange as i16,
            Self::RebalanceInProgress(_) => KafkaErrorCode::RebalanceInProgress as i16,
            Self::IllegalGeneration { .. } => KafkaErrorCode::IllegalGeneration as i16,
            Self::InconsistentGroupProtocol(_) => KafkaErrorCode::InconsistentGroupProtocol as i16,
            Self::MemberNotFound { .. } => KafkaErrorCode::UnknownMemberId as i16,
            Self::DuplicateSequenceNumber { .. } => KafkaErrorCode::DuplicateSequenceNumber as i16,
            Self::ProducerIdNotFound(_) => KafkaErrorCode::UnknownProducerId as i16,
            Self::SchemaIncompatible { .. } => KafkaErrorCode::SchemasNotCompatible as i16,
            Self::NotEnoughReplicas { .. } => KafkaErrorCode::NotEnoughReplicas as i16,
            _ => -1,
        }
    }
}
