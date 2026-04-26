use thiserror::Error;
pub type KedaResult<T> = Result<T, KedaError>;

#[derive(Error, Debug, Clone)]
pub enum KedaError {
    #[error("ScaledObject not found: {0}")] ScaledObjectNotFound(String),
    #[error("ScaledJob not found: {0}")] ScaledJobNotFound(String),
    #[error("TriggerAuth not found: {0}")] TriggerAuthNotFound(String),
    #[error("Invalid trigger: {detail}")] InvalidTrigger { detail: String },
    #[error("Scale failed: {detail}")] ScaleFailed { detail: String },
    #[error("Already exists: {0}")] AlreadyExists(String),
    #[error("Validation error: {0}")] Validation(String),
    #[error("Internal error: {0}")] Internal(String),
}
impl KedaError {
    pub fn status_code(&self) -> u16 {
        match self {
            KedaError::ScaledObjectNotFound(_) | KedaError::ScaledJobNotFound(_) | KedaError::TriggerAuthNotFound(_) => 404,
            KedaError::AlreadyExists(_) | KedaError::InvalidTrigger { .. } | KedaError::Validation(_) => 400,
            _ => 500,
        }
    }
}
