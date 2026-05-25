// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-bench — kube-bench (CIS) + kubescape (NSA + MITRE).
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("check not found: {0}")]
    CheckNotFound(String),
    #[error("control not found: {0}")]
    ControlNotFound(String),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("scan failed: {0}")]
    ScanFailed(String),
    #[error("control invalid: {0}")]
    ControlInvalid(String),
    #[error("target missing: {0}")]
    TargetMissing(String),
    #[error("io: {0}")]
    Io(String),
    #[error("yaml parse: {0}")]
    YamlParse(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, BenchError>;

impl From<std::io::Error> for BenchError {
    fn from(e: std::io::Error) -> Self {
        BenchError::Io(e.to_string())
    }
}

impl From<serde_yaml::Error> for BenchError {
    fn from(e: serde_yaml::Error) -> Self {
        BenchError::YamlParse(e.to_string())
    }
}

impl From<serde_json::Error> for BenchError {
    fn from(e: serde_json::Error) -> Self {
        BenchError::Internal(format!("json: {e}"))
    }
}
