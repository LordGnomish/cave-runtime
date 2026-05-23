// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Crate-wide error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    Conflict(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("policy violation: {0}")]
    PolicyViolation(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("upstream: {0}")]
    Upstream(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_renders() {
        let e = Error::NotFound("project=42".into());
        assert!(e.to_string().contains("project=42"));
    }

    #[test]
    fn parse_renders_message() {
        let e = Error::Parse("bad cyclonedx".into());
        assert!(e.to_string().contains("bad cyclonedx"));
    }

    #[test]
    fn io_wraps_std() {
        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let e: Error = inner.into();
        assert!(matches!(e, Error::Io(_)));
    }

    #[test]
    fn json_wraps_serde() {
        let parsed: std::result::Result<serde_json::Value, _> = serde_json::from_str("{");
        let e: Error = parsed.unwrap_err().into();
        assert!(matches!(e, Error::Json(_)));
    }
}
