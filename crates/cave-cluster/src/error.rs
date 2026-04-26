//! Error types for cave-cluster.

use thiserror::Error;

pub type ClusterResult<T> = Result<T, ClusterError>;

#[derive(Error, Debug)]
pub enum ClusterError {
    #[error("Cluster not found: {0}")]
    NotFound(String),

    #[error("Cluster already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid cluster name '{name}': {reason}")]
    InvalidName { name: String, reason: String },

    #[error("Unsupported Kubernetes version: {0}")]
    UnsupportedVersion(String),

    #[error("Cannot upgrade from {from} to {to}: {reason}")]
    InvalidUpgrade {
        from: String,
        to: String,
        reason: String,
    },

    #[error("Node pool not found: cluster={cluster}, pool={pool}")]
    NodePoolNotFound { cluster: String, pool: String },

    #[error("Node pool already exists: {0}")]
    NodePoolAlreadyExists(String),

    #[error("Cannot delete last node pool in cluster {0}")]
    LastNodePool(String),

    #[error("Cluster not in expected state: cluster={cluster}, expected={expected}, actual={actual}")]
    InvalidState {
        cluster: String,
        expected: String,
        actual: String,
    },

    #[error("etcd backup failed: {0}")]
    EtcdBackupFailed(String),

    #[error("etcd restore failed: {0}")]
    EtcdRestoreFailed(String),

    #[error("kubeconfig generation failed: {0}")]
    KubeconfigFailed(String),

    #[error("RBAC bootstrap failed: {0}")]
    RbacFailed(String),

    #[error("Addon not found: {0}")]
    AddonNotFound(String),

    #[error("Tenant not found: {0}")]
    TenantNotFound(String),

    #[error("Tenant already exists: {0}")]
    TenantAlreadyExists(String),

    #[error("Kubernetes API error: {0}")]
    KubeApi(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}
