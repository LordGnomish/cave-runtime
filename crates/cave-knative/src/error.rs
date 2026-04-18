//! Error types for cave-knative.

use thiserror::Error;

pub type KnativeResult<T> = Result<T, KnativeError>;

#[derive(Error, Debug, Clone)]
pub enum KnativeError {
    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    #[error("Revision not found: {service}/{revision}")]
    RevisionNotFound { service: String, revision: String },

    #[error("Route not found: {0}")]
    RouteNotFound(String),

    #[error("Broker not found: {0}")]
    BrokerNotFound(String),

    #[error("Trigger not found: {0}")]
    TriggerNotFound(String),

    #[error("Source not found: {0}")]
    SourceNotFound(String),

    #[error("Channel not found: {0}")]
    ChannelNotFound(String),

    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),

    #[error("Scale failed: {reason}")]
    ScaleFailed { reason: String },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl KnativeError {
    pub fn status_code(&self) -> u16 {
        match self {
            KnativeError::ServiceNotFound(_)
            | KnativeError::RevisionNotFound { .. }
            | KnativeError::RouteNotFound(_)
            | KnativeError::BrokerNotFound(_)
            | KnativeError::TriggerNotFound(_)
            | KnativeError::SourceNotFound(_)
            | KnativeError::ChannelNotFound(_)
            | KnativeError::SubscriptionNotFound(_) => 404,
            KnativeError::Validation(_) => 400,
            KnativeError::ScaleFailed { .. } | KnativeError::Internal(_) => 500,
        }
    }
}
