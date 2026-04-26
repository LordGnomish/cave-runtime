use thiserror::Error;
pub type EsoResult<T> = Result<T, EsoError>;

#[derive(Error, Debug, Clone)]
pub enum EsoError {
    #[error("SecretStore not found: {0}")] SecretStoreNotFound(String),
    #[error("ExternalSecret not found: {0}")] ExternalSecretNotFound(String),
    #[error("PushSecret not found: {0}")] PushSecretNotFound(String),
    #[error("Provider config not found: {0}")] ProviderConfigNotFound(String),
    #[error("Already exists: {0}")] AlreadyExists(String),
    #[error("Sync failed: {detail}")] SyncFailed { detail: String },
    #[error("Provider error: {detail}")] ProviderError { detail: String },
    #[error("Validation error: {0}")] Validation(String),
    #[error("Internal error: {0}")] Internal(String),
}
impl EsoError {
    pub fn status_code(&self) -> u16 {
        match self {
            EsoError::SecretStoreNotFound(_) | EsoError::ExternalSecretNotFound(_) |
            EsoError::PushSecretNotFound(_) | EsoError::ProviderConfigNotFound(_) => 404,
            EsoError::AlreadyExists(_) | EsoError::Validation(_) => 400,
            _ => 500,
        }
    }
}
