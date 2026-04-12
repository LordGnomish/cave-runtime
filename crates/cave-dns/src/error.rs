#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("zone not found: {0}")]
    ZoneNotFound(String),
    #[error("record not found: {0}")]
    RecordNotFound(String),
    #[error("zone already exists: {0}")]
    ZoneExists(String),
    #[error("invalid name: {0}")]
    InvalidName(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("format error")]
    FormatError,
    #[error("server failure")]
    ServerFailure,
    #[error("not implemented")]
    NotImplemented,
    #[error("refused")]
    Refused,
    #[error("io error: {0}")]
    Io(String),
}

pub type DnsResult<T> = Result<T, DnsError>;
