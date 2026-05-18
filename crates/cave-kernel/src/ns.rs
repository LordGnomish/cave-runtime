// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tenant / namespace identity primitive.
//!
//! Every multi-tenant request in CAVE flows past a `TenantId`. cave-apiserver,
//! cave-net (CNI), and cave-portal each used to ship their own ad-hoc
//! `tenant_id: String` arguments — the kernel `TenantId` newtype gives them
//! a single canonical type with a shared validation / propagation contract.
//!
//! Validation matches DNS-1123 label rules (RFC 1123 §2.1) so tenant
//! identifiers can safely be used as hostname components, Kubernetes
//! namespaces, and Postgres schema names without re-escaping.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TenantError {
    #[error("tenant id must be 1..=63 characters; got {0}")]
    BadLength(usize),
    #[error("tenant id must be lowercase alphanumeric or '-'; bad char {0:?}")]
    BadChar(char),
    #[error("tenant id must start and end with an alphanumeric")]
    BadEdge,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(String);

impl TenantId {
    /// Header name carrying the tenant id over HTTP/gRPC. Match Cortex /
    /// Loki convention; cave-portal SSE uses the same.
    pub const HEADER: &'static str = "X-Scope-OrgID";

    /// Default tenant for non-multi-tenant flows ("anonymous" in
    /// cave-alerts, "default" in cave-apiserver). The kernel doesn't
    /// pick a default; consumers can `TenantId::system()` for the
    /// system-tenant case.
    pub fn system() -> Self {
        TenantId("system".to_string())
    }

    pub fn new(s: impl Into<String>) -> Result<Self, TenantError> {
        let s = s.into();
        validate(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str { &self.0 }
    pub fn into_inner(self) -> String { self.0 }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for TenantId {
    fn as_ref(&self) -> &str { &self.0 }
}

impl FromStr for TenantId {
    type Err = TenantError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

fn validate(s: &str) -> Result<(), TenantError> {
    let len = s.len();
    if !(1..=63).contains(&len) {
        return Err(TenantError::BadLength(len));
    }
    for ch in s.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
        if !ok {
            return Err(TenantError::BadChar(ch));
        }
    }
    let bytes = s.as_bytes();
    let edge_ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !edge_ok(bytes[0]) || !edge_ok(bytes[len - 1]) {
        return Err(TenantError::BadEdge);
    }
    Ok(())
}

/// Per-request scope: tenant id + optional sub-namespace (e.g. Kubernetes
/// namespace within a tenant). Routing layers stash this on extensions /
/// task locals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantScope {
    pub tenant: TenantId,
    pub namespace: Option<String>,
}

impl TenantScope {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, namespace: None }
    }

    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_id() {
        let t: TenantId = "acme-corp".parse().unwrap();
        assert_eq!(t.as_str(), "acme-corp");
        assert_eq!(t.to_string(), "acme-corp");
    }

    #[test]
    fn rejects_empty() {
        assert_eq!("".parse::<TenantId>().unwrap_err(), TenantError::BadLength(0));
    }

    #[test]
    fn rejects_too_long() {
        let s = "a".repeat(64);
        assert_eq!(s.parse::<TenantId>().unwrap_err(), TenantError::BadLength(64));
    }

    #[test]
    fn rejects_uppercase() {
        let err = "AcmeCorp".parse::<TenantId>().unwrap_err();
        assert!(matches!(err, TenantError::BadChar('A')));
    }

    #[test]
    fn rejects_underscore() {
        let err = "acme_corp".parse::<TenantId>().unwrap_err();
        assert_eq!(err, TenantError::BadChar('_'));
    }

    #[test]
    fn rejects_leading_dash() {
        assert_eq!("-acme".parse::<TenantId>().unwrap_err(), TenantError::BadEdge);
    }

    #[test]
    fn rejects_trailing_dash() {
        assert_eq!("acme-".parse::<TenantId>().unwrap_err(), TenantError::BadEdge);
    }

    #[test]
    fn header_constant_matches_cortex_loki() {
        assert_eq!(TenantId::HEADER, "X-Scope-OrgID");
    }

    #[test]
    fn scope_carries_optional_namespace() {
        let t: TenantId = "acme".parse().unwrap();
        let s = TenantScope::new(t.clone()).with_namespace("billing");
        assert_eq!(s.tenant, t);
        assert_eq!(s.namespace.as_deref(), Some("billing"));
    }

    #[test]
    fn round_trips_through_serde() {
        let t: TenantId = "acme-corp".parse().unwrap();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "\"acme-corp\"");
        let back: TenantId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }
}
