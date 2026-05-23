// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Unified error type for cave-k8s control-plane surface.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("resource already exists: {0}")]
    AlreadyExists(String),

    #[error("invalid configuration: {0}")]
    Invalid(String),

    #[error("admission rejected: {0}")]
    AdmissionRejected(String),

    #[error("authentication failed: {0}")]
    Unauthenticated(String),

    #[error("authorization denied: {0}")]
    Forbidden(String),

    #[error("namespace {namespace} terminating; new {kind} writes blocked")]
    NamespaceTerminating { namespace: String, kind: String },

    #[error("ResourceQuota {quota} would be exceeded: {detail}")]
    QuotaExceeded { quota: String, detail: String },

    #[error("subsystem error: {component}: {detail}")]
    Subsystem { component: String, detail: String },

    #[error("conflict on resource version: {0}")]
    Conflict(String),

    #[error("signature verification failed: {0}")]
    Signature(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl Error {
    pub fn subsystem(component: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Subsystem {
            component: component.into(),
            detail: detail.into(),
        }
    }

    /// Map an error to an HTTP status code per K8s API conventions.
    /// Mirrors `staging/src/k8s.io/apimachinery/pkg/api/errors/errors.go`.
    pub fn http_status(&self) -> u16 {
        match self {
            Error::NotFound(_) => 404,
            Error::AlreadyExists(_) => 409,
            Error::Conflict(_) => 409,
            Error::Invalid(_) => 422,
            Error::AdmissionRejected(_) => 400,
            Error::Unauthenticated(_) => 401,
            Error::Forbidden(_) => 403,
            Error::NamespaceTerminating { .. } => 403,
            Error::QuotaExceeded { .. } => 403,
            Error::Signature(_) => 400,
            Error::Subsystem { .. } | Error::Internal(_) => 500,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Error::Conflict(_) | Error::Subsystem { .. } | Error::Internal(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_matches_k8s_conventions() {
        assert_eq!(Error::NotFound("x".into()).http_status(), 404);
        assert_eq!(Error::AlreadyExists("x".into()).http_status(), 409);
        assert_eq!(Error::Conflict("x".into()).http_status(), 409);
        assert_eq!(Error::Invalid("x".into()).http_status(), 422);
        assert_eq!(Error::Unauthenticated("x".into()).http_status(), 401);
        assert_eq!(Error::Forbidden("x".into()).http_status(), 403);
        assert_eq!(Error::Internal("x".into()).http_status(), 500);
        assert_eq!(
            Error::QuotaExceeded {
                quota: "q".into(),
                detail: "d".into()
            }
            .http_status(),
            403
        );
        assert_eq!(
            Error::NamespaceTerminating {
                namespace: "n".into(),
                kind: "Pod".into()
            }
            .http_status(),
            403
        );
        assert_eq!(Error::AdmissionRejected("x".into()).http_status(), 400);
        assert_eq!(Error::Signature("x".into()).http_status(), 400);
    }

    #[test]
    fn retryable_classification() {
        assert!(Error::Conflict("x".into()).is_retryable());
        assert!(Error::subsystem("etcd", "io").is_retryable());
        assert!(Error::Internal("x".into()).is_retryable());
        assert!(!Error::NotFound("x".into()).is_retryable());
        assert!(!Error::Invalid("x".into()).is_retryable());
        assert!(!Error::Forbidden("x".into()).is_retryable());
    }

    #[test]
    fn subsystem_constructor_threads_components() {
        let e = Error::subsystem("scheduler", "no schedulable nodes");
        let s = format!("{e}");
        assert!(s.contains("scheduler"));
        assert!(s.contains("no schedulable nodes"));
    }

    #[test]
    fn namespace_terminating_carries_kind() {
        let e = Error::NamespaceTerminating {
            namespace: "alpha".into(),
            kind: "Pod".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("alpha"));
        assert!(s.contains("Pod"));
    }
}
