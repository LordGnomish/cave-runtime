// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-forensics error type — single source of failure across all modules.

use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ForensicsError>;

/// Errors raised by tracing-policy parsing, policy filtering, enforcement,
/// event ingestion, and evidence chain-of-custody validation.
#[derive(Debug, Error)]
pub enum ForensicsError {
    #[error("invalid tracing policy: {0}")]
    InvalidPolicy(String),

    #[error("invalid selector: {0}")]
    InvalidSelector(String),

    #[error("invalid filter spec: {0}")]
    InvalidFilter(String),

    #[error("policy filter operator `{0}` rejected for field `{1}`")]
    FilterOpRejected(String, String),

    #[error("enforcement action `{0}` not supported in this context")]
    EnforcementUnsupported(String),

    #[error("kernel event missing required field `{0}`")]
    EventMissingField(&'static str),

    #[error("event encode failed: {0}")]
    Encode(String),

    #[error("event decode failed: {0}")]
    Decode(String),

    #[error("case not found: {0}")]
    CaseNotFound(String),

    #[error("evidence not found: {0}")]
    EvidenceNotFound(String),

    #[error("chain-of-custody integrity violation: {0}")]
    ChainBroken(String),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("regex: {0}")]
    Regex(#[from] regex::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_policy_display() {
        let e = ForensicsError::InvalidPolicy("missing name".into());
        assert!(e.to_string().contains("missing name"));
    }

    #[test]
    fn test_event_missing_field_display() {
        let e = ForensicsError::EventMissingField("pid");
        assert!(e.to_string().contains("pid"));
    }

    #[test]
    fn test_filter_op_rejected_includes_both() {
        let e = ForensicsError::FilterOpRejected("Glob".into(), "pid".into());
        let s = e.to_string();
        assert!(s.contains("Glob"));
        assert!(s.contains("pid"));
    }

    #[test]
    fn test_from_serde_json_error() {
        let bad: std::result::Result<serde_json::Value, _> = serde_json::from_str("{");
        let e: ForensicsError = bad.unwrap_err().into();
        assert!(matches!(e, ForensicsError::Serde(_)));
    }

    #[test]
    fn test_from_regex_error() {
        let bad = regex::Regex::new("(");
        let e: ForensicsError = bad.unwrap_err().into();
        assert!(matches!(e, ForensicsError::Regex(_)));
    }
}
