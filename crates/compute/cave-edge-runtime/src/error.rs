// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type shared across the edge runtime subsystems.

use std::fmt;

/// Errors raised by the edge node agent and its sync/twin subsystems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeError {
    /// A resource (pod, device, metadata key) was not found locally.
    NotFound(String),
    /// A message could not be routed — unknown group or malformed topic.
    Routing(String),
    /// A sync/version invariant was violated (stale write, gap, etc.).
    Sync(String),
    /// A constrained-resource budget was exceeded.
    Resource(String),
    /// A device twin update was rejected (bad version / unknown attr).
    Twin(String),
}

impl fmt::Display for EdgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeError::NotFound(s) => write!(f, "not found: {s}"),
            EdgeError::Routing(s) => write!(f, "routing error: {s}"),
            EdgeError::Sync(s) => write!(f, "sync error: {s}"),
            EdgeError::Resource(s) => write!(f, "resource error: {s}"),
            EdgeError::Twin(s) => write!(f, "twin error: {s}"),
        }
    }
}

impl std::error::Error for EdgeError {}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, EdgeError>;
