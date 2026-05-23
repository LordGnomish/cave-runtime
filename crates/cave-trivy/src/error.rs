// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Top-level error type for cave-trivy.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrivyError {
    #[error("io error: {0}")]
    Io(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("scan target not found: {0}")]
    TargetNotFound(String),
    #[error("vuln db error: {0}")]
    VulnDb(String),
    #[error("misconfig error: {0}")]
    Misconfig(String),
    #[error("sbom error: {0}")]
    Sbom(String),
    #[error("report error: {0}")]
    Report(String),
    #[error("policy violation: {0}")]
    Policy(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl TrivyError {
    pub fn io(s: impl Into<String>) -> Self {
        Self::Io(s.into())
    }
    pub fn parse(s: impl Into<String>) -> Self {
        Self::Parse(s.into())
    }
}

pub type TrivyResult<T> = Result<T, TrivyError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_format() {
        let e = TrivyError::io("boom");
        assert_eq!(e.to_string(), "io error: boom");
        let e = TrivyError::parse("oops");
        assert_eq!(e.to_string(), "parse error: oops");
        let e = TrivyError::TargetNotFound("x".into());
        assert!(e.to_string().contains("scan target"));
        let e = TrivyError::VulnDb("v".into());
        assert!(e.to_string().contains("vuln db"));
        let e = TrivyError::Misconfig("m".into());
        assert!(e.to_string().contains("misconfig"));
        let e = TrivyError::Sbom("s".into());
        assert!(e.to_string().contains("sbom"));
        let e = TrivyError::Report("r".into());
        assert!(e.to_string().contains("report"));
        let e = TrivyError::Policy("p".into());
        assert!(e.to_string().contains("policy"));
        let e = TrivyError::Unsupported("u".into());
        assert!(e.to_string().contains("unsupported"));
    }

    #[test]
    fn result_type() {
        let r: TrivyResult<u8> = Ok(5);
        assert_eq!(r.unwrap(), 5);
    }
}
