// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("check not found: {0}")]
    CheckNotFound(String),
    #[error("scan failed: {0}")]
    ScanFailed(String),
    #[error("control invalid: {0}")]
    ControlInvalid(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, BenchError>;
