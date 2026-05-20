// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared types for all controllers in cave-controller-manager.
//!
//! Every controller carries a [`TenantId`] for multi-tenant isolation, and
//! every test in this crate annotates itself with a [`Cite`] pointing at the
//! upstream Kubernetes source it mirrors.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Pinned upstream Kubernetes release this scaffold tracks.
///
/// The expectation is that file/test/function citations refer to symbols as
/// they exist in this exact tag.
pub const UPSTREAM_VERSION: &str = "v1.36.0";

/// Stable upstream module path for kube-controller-manager.
pub const UPSTREAM_PKG: &str = "k8s.io/kubernetes/pkg/controller";

/// Multi-tenant identifier — re-exported from `cave_kernel::ns` (sweep-002
/// F2-G adoption, 2026-05-01). Controllers MUST scope all reconciliation,
/// metrics, and audit log entries to the tenant that owns the workload; the
/// kernel newtype provides the single DNS-1123-validated surface.
pub use cave_kernel::ns::TenantId;

/// A citation pointing at the upstream Kubernetes symbol or test that a piece
/// of local code is a parity port of.
///
/// `path` is relative to the kubernetes/kubernetes repo (e.g.
/// `pkg/controller/deployment/sync.go`); `symbol` is the function, type or
/// test name; `version` defaults to [`UPSTREAM_VERSION`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cite {
    pub path: &'static str,
    pub symbol: &'static str,
    pub version: &'static str,
}

impl Cite {
    pub const fn new(path: &'static str, symbol: &'static str) -> Self {
        Self {
            path,
            symbol,
            version: UPSTREAM_VERSION,
        }
    }

    pub fn url(&self) -> String {
        format!(
            "https://github.com/kubernetes/kubernetes/blob/{}/{}",
            self.version, self.path
        )
    }
}

impl fmt::Display for Cite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{} @ {}", self.path, self.symbol, self.version)
    }
}

/// Outcome of one reconciliation pass.
///
/// Mirrors the `Result` half of `controller.Reconciler.Reconcile` in
/// `sigs.k8s.io/controller-runtime`, but flattened to a Rust enum to keep the
/// scaffold self-contained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reconcile {
    /// Nothing to do — desired state matches observed state.
    NoOp,
    /// Controller would create N child objects (pods, endpointslices, …).
    Create(u32),
    /// Controller would delete N child objects.
    Delete(u32),
    /// Controller would patch / update N existing children.
    Update(u32),
    /// Controller is waiting on an external signal (timer, status, etc.).
    Requeue,
}

/// Standard error type for every controller in this crate.
#[derive(Debug, thiserror::Error)]
pub enum ControllerError {
    #[error("invalid spec for {kind}: {reason}")]
    InvalidSpec { kind: &'static str, reason: String },
    #[error("tenant {tenant} not authorized for {kind}/{name}")]
    TenantDenied {
        tenant: TenantId,
        kind: &'static str,
        name: String,
    },
    #[error("not yet implemented: {0}")]
    Unimplemented(&'static str),
}

/// Helper used by every test in this crate.
///
/// Bundles the `Cite` and `TenantId` together so the test name plus this
/// macro give an auditor everything they need to find the upstream source.
///
/// Returns the `(Cite, TenantId)` tuple so the test can assert against them.
#[macro_export]
macro_rules! test_ctx {
    ($path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::types::Cite::new($path, $symbol);
        let tenant = $crate::types::TenantId::new($tenant).expect("test fixture");
        // sanity: cite must point at the pinned upstream tag
        assert_eq!(cite.version, $crate::types::UPSTREAM_VERSION);
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        (cite, tenant)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cite_url_uses_pinned_version() {
        let (cite, tenant) = test_ctx!(
            "pkg/controller/types.go",
            "Reconciler",
            "tenant-shared-types"
        );
        assert!(cite.url().contains(UPSTREAM_VERSION));
        assert_eq!(tenant.as_str(), "tenant-shared-types");
    }

    #[test]
    fn tenant_id_round_trips_through_serde() {
        let (_cite, tenant) = test_ctx!("pkg/controller/types.go", "TenantId", "tenant-serde");
        let json = serde_json::to_string(&tenant).unwrap();
        let back: TenantId = serde_json::from_str(&json).unwrap();
        assert_eq!(tenant, back);
    }

    #[test]
    fn reconcile_variants_serialize() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/controller_utils.go",
            "Reconcile",
            "tenant-reconcile"
        );
        for r in [
            Reconcile::NoOp,
            Reconcile::Create(3),
            Reconcile::Delete(1),
            Reconcile::Update(2),
            Reconcile::Requeue,
        ] {
            let _ = serde_json::to_string(&r).unwrap();
        }
    }
}
