// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared types for the Ambient-mode parity batch.
//!
//! Mirrors the conventions used by `cave-controller-manager` and
//! `cave-cloud-controller-manager`: every test asserts `Cite` + `TenantId`,
//! and the `Cite::version` is checked against the pinned upstream tag.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Pinned upstream Istio release this batch tracks.
pub const UPSTREAM_VERSION: &str = "1.29.2";

/// Upstream repo (without the leading `https://github.com/`).
pub const UPSTREAM_REPO: &str = "istio/istio";

/// Multi-tenant identifier — re-exported from `cave_kernel::ns` (sweep-002
/// F2-G adoption, 2026-05-01). Every Ambient-mode object (HBONE tunnel,
/// AuthorizationPolicy, VirtualService, …) is scoped to a tenant.
pub use cave_kernel::ns::TenantId;

/// Citation pointing at an upstream Istio symbol. `repo` defaults to
/// `istio/istio`; `version` defaults to [`UPSTREAM_VERSION`]. The `Cite::ext`
/// constructor accepts an explicit repo for the out-of-tree `istio/ztunnel`
/// component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cite {
    pub repo: &'static str,
    pub path: &'static str,
    pub symbol: &'static str,
    pub version: &'static str,
}

impl Cite {
    pub const fn istio(path: &'static str, symbol: &'static str) -> Self {
        Self { repo: UPSTREAM_REPO, path, symbol, version: UPSTREAM_VERSION }
    }
    pub const fn ext(
        repo: &'static str,
        path: &'static str,
        symbol: &'static str,
        version: &'static str,
    ) -> Self {
        Self { repo, path, symbol, version }
    }
    pub fn url(&self) -> String {
        format!("https://github.com/{}/blob/{}/{}", self.repo, self.version, self.path)
    }
}

impl fmt::Display for Cite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}::{} @ {}", self.repo, self.path, self.symbol, self.version)
    }
}

/// Test-only macro: builds a `(Cite, TenantId)` pair and asserts both for
/// shape (non-empty tenant, version starts with a digit). Two arms:
///
/// * `(path, symbol, tenant)` — istio/istio at the pinned tag.
/// * `(ext: repo, version, path, symbol, tenant)` — out-of-tree Ambient repo.
#[macro_export]
macro_rules! ambient_test_ctx {
    ($path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::ambient::types::Cite::istio($path, $symbol);
        let tenant = $crate::ambient::types::TenantId::new($tenant).expect("test fixture");
        assert_eq!(cite.version, $crate::ambient::types::UPSTREAM_VERSION);
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        (cite, tenant)
    }};
    (ext: $repo:expr, $version:expr, $path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::ambient::types::Cite::ext($repo, $path, $symbol, $version);
        let tenant = $crate::ambient::types::TenantId::new($tenant).expect("test fixture");
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        // Istio uses both tag versions ("1.29.2") and release branches
        // ("release-1.29") for the ztunnel sibling repo — accept either.
        assert!(!cite.version.is_empty(), "version must not be empty");
        (cite, tenant)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn istio_cite_url_uses_pinned_version() {
        let (cite, tenant) = ambient_test_ctx!(
            "pkg/hbone/server.go",
            "HBONEServer",
            "tenant-types-istio"
        );
        assert!(cite.url().contains(UPSTREAM_VERSION));
        assert_eq!(cite.repo, UPSTREAM_REPO);
        assert_eq!(tenant.as_str(), "tenant-types-istio");
    }

    #[test]
    fn ext_cite_keeps_external_repo_and_version() {
        let (cite, _t) = ambient_test_ctx!(
            ext: "istio/ztunnel",
            "release-1.29",
            "src/proxy/inbound.rs",
            "Inbound",
            "tenant-types-ext"
        );
        assert_eq!(cite.repo, "istio/ztunnel");
        assert_eq!(cite.version, "release-1.29");
    }

    #[test]
    fn tenant_id_round_trips_through_serde() {
        let (_cite, tenant) = ambient_test_ctx!(
            "pkg/hbone/server.go",
            "HBONEServer",
            "tenant-types-serde"
        );
        let json = serde_json::to_string(&tenant).unwrap();
        let back: TenantId = serde_json::from_str(&json).unwrap();
        assert_eq!(tenant, back);
    }
}
