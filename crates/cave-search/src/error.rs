//! Error types for cave-search.

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SearchError {
    #[error("index not found: {0}")]
    IndexNotFound(String),
    #[error("index already exists: {0}")]
    IndexExists(String),
    #[error("document not found: {0}/{1}")]
    DocumentNotFound(String, String),
    #[error("invalid mapping: {0}")]
    InvalidMapping(String),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("invalid aggregation: {0}")]
    InvalidAggregation(String),
    #[error("type mismatch for field {field}: expected {expected:?}, got {got:?}")]
    TypeMismatch {
        field: String,
        expected: String,
        got: String,
    },
}

pub type SearchResult<T> = Result<T, SearchError>;
