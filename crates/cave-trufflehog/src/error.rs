// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error envelope — mirrors `pkg/detectors.SetVerificationError` taxonomy.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(String),
    #[error("regex: {0}")]
    Regex(String),
    #[error("config: {0}")]
    Config(String),
    #[error("source: {0}")]
    Source(String),
    #[error("verification: {0}")]
    Verification(String),
    #[error("git: {0}")]
    Git(String),
    #[error("serialization: {0}")]
    Serialization(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serialization(e.to_string())
    }
}

impl From<regex::Error> for Error {
    fn from(e: regex::Error) -> Self {
        Error::Regex(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_renders_prefix() {
        assert_eq!(Error::Io("x".into()).to_string(), "io: x");
        assert_eq!(Error::Config("c".into()).to_string(), "config: c");
    }

    #[test]
    fn from_io_carries_message() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let e: Error = io.into();
        assert!(matches!(e, Error::Io(_)));
        assert!(e.to_string().contains("missing"));
    }
}
