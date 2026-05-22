// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared types for the Cilium-parity batch.
//!
//! Same conventions as the other deeper batches in the workspace:
//! `TenantId` for multi-tenant scoping, `Cite` pointing at the upstream
//! symbol, and a `cilium_test_ctx!` macro that asserts both for shape.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Pinned upstream Cilium release this batch tracks.
pub const UPSTREAM_VERSION: &str = "v1.19.3";

/// Upstream repo (without the leading `https://github.com/`).
pub const UPSTREAM_REPO: &str = "cilium/cilium";

/// Multi-tenant identifier — re-exported from `cave_kernel::ns` (sweep-002
/// F2-G adoption, 2026-05-01). Every Cilium-side object — identity, L7 rule,
/// flow log, ClusterMesh announcement — is scoped to a tenant.
pub use cave_kernel::ns::TenantId;

/// Citation pointing at the upstream Cilium symbol a piece of local code is
/// a parity port of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cite {
    pub repo: &'static str,
    pub path: &'static str,
    pub symbol: &'static str,
    pub version: &'static str,
}

impl Cite {
    pub const fn cilium(path: &'static str, symbol: &'static str) -> Self {
        Self {
            repo: UPSTREAM_REPO,
            path,
            symbol,
            version: UPSTREAM_VERSION,
        }
    }
    pub fn url(&self) -> String {
        format!(
            "https://github.com/{}/blob/{}/{}",
            self.repo, self.version, self.path
        )
    }
}

impl fmt::Display for Cite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}::{}::{} @ {}",
            self.repo, self.path, self.symbol, self.version
        )
    }
}

/// Bundles a `Cite` and a `TenantId` for a test, asserting:
/// * the cite version matches [`UPSTREAM_VERSION`]
/// * the tenant id is non-empty
///
/// Returns `(Cite, TenantId)` so the test can assert against either one.
#[macro_export]
macro_rules! cilium_test_ctx {
    ($path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::cilium::types::Cite::cilium($path, $symbol);
        let tenant = $crate::cilium::types::TenantId::new($tenant).expect("test fixture");
        assert_eq!(cite.version, $crate::cilium::types::UPSTREAM_VERSION);
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        (cite, tenant)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cite_url_uses_pinned_version_and_repo() {
        let (cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "LocalIdentityCache",
            "tenant-types-cite"
        );
        assert!(cite.url().contains(UPSTREAM_VERSION));
        assert!(cite.url().contains(UPSTREAM_REPO));
        assert_eq!(tenant.as_str(), "tenant-types-cite");
    }

    #[test]
    fn tenant_id_round_trips_through_serde() {
        let (_cite, tenant) =
            cilium_test_ctx!("pkg/identity/identity.go", "Identity", "tenant-types-serde");
        let json = serde_json::to_string(&tenant).unwrap();
        let back: TenantId = serde_json::from_str(&json).unwrap();
        assert_eq!(tenant, back);
    }

    #[test]
    fn cite_display_includes_repo_path_symbol_and_version() {
        let (cite, _t) =
            cilium_test_ctx!("pkg/policy/api/l7.go", "PortRule", "tenant-types-display");
        let s = format!("{}", cite);
        assert!(s.contains("cilium/cilium"));
        assert!(s.contains("pkg/policy/api/l7.go"));
        assert!(s.contains("PortRule"));
        assert!(s.contains(UPSTREAM_VERSION));
    }
}
