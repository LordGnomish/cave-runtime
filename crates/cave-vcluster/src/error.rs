use thiserror::Error;
pub type VClusterResult<T> = Result<T, VClusterError>;

#[derive(Error, Debug, Clone)]
pub enum VClusterError {
    #[error("Cluster not found: {0}")] ClusterNotFound(String),
    #[error("Quota exceeded: max {max} clusters per namespace")] QuotaExceeded { max: u32 },
    #[error("Cluster already exists: {0}")] AlreadyExists(String),
    #[error("Sync failed: {detail}")] SyncFailed { detail: String },
    #[error("Invalid config: {0}")] InvalidConfig(String),
    #[error("Internal error: {0}")] Internal(String),
}
impl VClusterError {
    pub fn status_code(&self) -> u16 {
        match self {
            VClusterError::ClusterNotFound(_) => 404,
            VClusterError::AlreadyExists(_) | VClusterError::QuotaExceeded { .. } | VClusterError::InvalidConfig(_) => 400,
            _ => 500,
        }
    }
}
