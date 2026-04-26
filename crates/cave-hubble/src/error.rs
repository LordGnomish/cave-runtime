use thiserror::Error;
pub type HubbleResult<T> = Result<T, HubbleError>;

#[derive(Error, Debug, Clone)]
pub enum HubbleError {
    #[error("Flow not found: {0}")] FlowNotFound(String),
    #[error("Invalid filter: {0}")] InvalidFilter(String),
    #[error("Internal error: {0}")] Internal(String),
}
impl HubbleError {
    pub fn status_code(&self) -> u16 {
        match self {
            HubbleError::FlowNotFound(_) => 404,
            HubbleError::InvalidFilter(_) => 400,
            HubbleError::Internal(_) => 500,
        }
    }
}
