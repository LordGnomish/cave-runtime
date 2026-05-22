// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workload identity primitives — SPIFFE ID & SVID metadata.
//!
//! Implements the SPIFFE 1.0 ID grammar:
//!     spiffe-id = "spiffe://" trust-domain "/" path
//! where the trust-domain is a DNS-style label set (lowercase letters,
//! digits, '.', '-') and the path is a forward-slash-separated sequence of
//! non-empty path segments.
//!
//! Used by cave-mesh (mTLS peer identity) and cave-auth (token subject
//! claims) so both sides agree on parsing and equality semantics.
//!
//! See https://github.com/spiffe/spiffe/blob/main/standards/SPIFFE-ID.md

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpiffeError {
    #[error("missing 'spiffe://' scheme")]
    MissingScheme,
    #[error("empty trust domain")]
    EmptyTrustDomain,
    #[error("invalid trust-domain character: {0:?}")]
    InvalidTrustDomain(char),
    #[error("trust domain longer than 255 characters")]
    TrustDomainTooLong,
    #[error("invalid path segment: {0:?}")]
    InvalidPathSegment(String),
    #[error("path contains percent-encoded sequence; not supported")]
    PercentEncoded,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SpiffeId {
    trust_domain: String,
    /// Path *without* the leading slash. Empty path is valid (trust-domain
    /// root SPIFFE ID).
    path: String,
}

impl SpiffeId {
    pub fn new(
        trust_domain: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, SpiffeError> {
        let trust_domain = trust_domain.into();
        let path = path.into();
        validate_trust_domain(&trust_domain)?;
        validate_path(&path)?;
        Ok(Self { trust_domain, path })
    }

    pub fn trust_domain(&self) -> &str {
        &self.trust_domain
    }
    pub fn path(&self) -> &str {
        &self.path
    }

    /// True if this ID is a workload under the given trust domain.
    pub fn is_member_of(&self, trust_domain: &str) -> bool {
        self.trust_domain == trust_domain
    }
}

impl fmt::Display for SpiffeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "spiffe://{}", self.trust_domain)
        } else {
            write!(f, "spiffe://{}/{}", self.trust_domain, self.path)
        }
    }
}

impl FromStr for SpiffeId {
    type Err = SpiffeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix("spiffe://")
            .ok_or(SpiffeError::MissingScheme)?;
        if rest.contains('%') {
            return Err(SpiffeError::PercentEncoded);
        }
        let (td, path) = match rest.split_once('/') {
            Some((td, path)) => (td.to_string(), path.to_string()),
            None => (rest.to_string(), String::new()),
        };
        Self::new(td, path)
    }
}

fn validate_trust_domain(td: &str) -> Result<(), SpiffeError> {
    if td.is_empty() {
        return Err(SpiffeError::EmptyTrustDomain);
    }
    if td.len() > 255 {
        return Err(SpiffeError::TrustDomainTooLong);
    }
    for ch in td.chars() {
        let ok =
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '_';
        if !ok {
            return Err(SpiffeError::InvalidTrustDomain(ch));
        }
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), SpiffeError> {
    if path.is_empty() {
        return Ok(());
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(SpiffeError::InvalidPathSegment(seg.to_string()));
        }
    }
    Ok(())
}

/// SVID metadata — pair of identity and validity window. Holders of the
/// SVID also carry the underlying X.509 / JWT material; the kernel only
/// describes shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SvidMetadata {
    pub id: SpiffeId,
    pub not_before: i64,
    pub not_after: i64,
}

impl SvidMetadata {
    pub fn is_valid_at(&self, unix_secs: i64) -> bool {
        unix_secs >= self.not_before && unix_secs < self.not_after
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_root_id() {
        let id: SpiffeId = "spiffe://example.org".parse().unwrap();
        assert_eq!(id.trust_domain(), "example.org");
        assert_eq!(id.path(), "");
        assert_eq!(id.to_string(), "spiffe://example.org");
    }

    #[test]
    fn parse_nested_path() {
        let id: SpiffeId = "spiffe://prod.cave/ns/default/sa/scheduler"
            .parse()
            .unwrap();
        assert_eq!(id.trust_domain(), "prod.cave");
        assert_eq!(id.path(), "ns/default/sa/scheduler");
    }

    #[test]
    fn reject_missing_scheme() {
        let err = "https://example.org".parse::<SpiffeId>().unwrap_err();
        assert_eq!(err, SpiffeError::MissingScheme);
    }

    #[test]
    fn reject_empty_trust_domain() {
        let err = "spiffe:///path".parse::<SpiffeId>().unwrap_err();
        assert_eq!(err, SpiffeError::EmptyTrustDomain);
    }

    #[test]
    fn reject_uppercase_trust_domain() {
        let err = "spiffe://Example.org".parse::<SpiffeId>().unwrap_err();
        assert!(matches!(err, SpiffeError::InvalidTrustDomain('E')));
    }

    #[test]
    fn reject_percent_encoding() {
        let err = "spiffe://example.org/foo%20bar"
            .parse::<SpiffeId>()
            .unwrap_err();
        assert_eq!(err, SpiffeError::PercentEncoded);
    }

    #[test]
    fn reject_empty_path_segment() {
        let err = "spiffe://example.org//foo".parse::<SpiffeId>().unwrap_err();
        assert!(matches!(err, SpiffeError::InvalidPathSegment(_)));
    }

    #[test]
    fn reject_dot_segments() {
        assert!(matches!(
            "spiffe://example.org/./x".parse::<SpiffeId>(),
            Err(SpiffeError::InvalidPathSegment(_))
        ));
        assert!(matches!(
            "spiffe://example.org/../x".parse::<SpiffeId>(),
            Err(SpiffeError::InvalidPathSegment(_))
        ));
    }

    #[test]
    fn round_trip_via_display() {
        let s = "spiffe://prod.cave/ns/default/sa/scheduler";
        let id: SpiffeId = s.parse().unwrap();
        assert_eq!(id.to_string(), s);
    }

    #[test]
    fn is_member_of_checks_trust_domain() {
        let id: SpiffeId = "spiffe://prod.cave/svc".parse().unwrap();
        assert!(id.is_member_of("prod.cave"));
        assert!(!id.is_member_of("dev.cave"));
    }

    #[test]
    fn svid_metadata_validity_window() {
        let id: SpiffeId = "spiffe://prod.cave/svc".parse().unwrap();
        let m = SvidMetadata {
            id,
            not_before: 100,
            not_after: 200,
        };
        assert!(!m.is_valid_at(50));
        assert!(m.is_valid_at(100));
        assert!(m.is_valid_at(150));
        assert!(!m.is_valid_at(200));
    }
}
