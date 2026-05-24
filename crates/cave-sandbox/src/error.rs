// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("sandbox not found: {0}")]
    NotFound(String),
    #[error("runtime not supported: {0}")]
    RuntimeUnsupported(String),
    #[error("oci spec invalid: {0}")]
    SpecInvalid(String),
    #[error("vm boot failed: {0}")]
    VmBootFailed(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, SandboxError>;
