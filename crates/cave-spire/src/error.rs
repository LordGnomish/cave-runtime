use thiserror::Error;
pub type SpireResult<T> = Result<T, SpireError>;

#[derive(Error, Debug, Clone)]
pub enum SpireError {
    #[error("Trust domain not found: {0}")] TrustDomainNotFound(String),
    #[error("Registration entry not found: {0}")] EntryNotFound(String),
    #[error("Agent not found: {0}")] AgentNotFound(String),
    #[error("SVID not found: {0}")] SvidNotFound(String),
    #[error("Federation bundle not found: {0}")] FederationNotFound(String),
    #[error("Already exists: {0}")] AlreadyExists(String),
    #[error("Attestation failed: {detail}")] AttestationFailed { detail: String },
    #[error("Rotation failed: {detail}")] RotationFailed { detail: String },
    #[error("Validation error: {0}")] Validation(String),
    #[error("Internal error: {0}")] Internal(String),
}
impl SpireError {
    pub fn status_code(&self) -> u16 {
        match self {
            SpireError::TrustDomainNotFound(_) | SpireError::EntryNotFound(_) |
            SpireError::AgentNotFound(_) | SpireError::SvidNotFound(_) |
            SpireError::FederationNotFound(_) => 404,
            SpireError::AlreadyExists(_) | SpireError::Validation(_) => 400,
            _ => 500,
        }
    }
}
