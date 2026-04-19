//! Error types for cave-cri.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CriError {
    #[error("container not found: {0}")]
    NotFound(String),

    #[error("invalid container state: {0}")]
    InvalidState(String),

    #[error("namespace error: {0}")]
    Namespace(String),

    #[error("cgroup error: {0}")]
    Cgroup(String),

    #[error("registry error: {0}")]
    Registry(String),

    #[error("rootfs error: {0}")]
    Rootfs(String),

    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("image error: {0}")]
    Image(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type CriResult<T> = Result<T, CriError>;
