use thiserror::Error;

pub type DnsResult<T> = Result<T, DnsError>;

#[derive(Debug, Error)]
pub enum DnsError {
    #[error("DNS protocol error: {0}")]
    Protocol(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("Zone error: {0}")]
    Zone(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Name not found: {0}")]
    NotFound(String),

    #[error("Query refused")]
    Refused,

    #[error("Not authoritative for zone: {0}")]
    NotAuth(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("DNSSEC error: {0}")]
    Dnssec(String),

    #[error("Zone transfer error: {0}")]
    Transfer(String),

    #[error("Dynamic update error: {0}")]
    Update(String),

    #[error("Kubernetes error: {0}")]
    Kubernetes(String),

    #[error("Etcd error: {0}")]
    Etcd(String),

    #[error("Operation timed out")]
    Timeout,

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Serialisation error: {0}")]
    Serialise(String),
}

impl From<hickory_proto::error::ProtoError> for DnsError {
    fn from(e: hickory_proto::error::ProtoError) -> Self {
        DnsError::Protocol(e.to_string())
    }
}

impl From<serde_json::Error> for DnsError {
    fn from(e: serde_json::Error) -> Self {
        DnsError::Serialise(e.to_string())
    }
}

impl From<reqwest::Error> for DnsError {
    fn from(e: reqwest::Error) -> Self {
        DnsError::Http(e.to_string())
    }
}
