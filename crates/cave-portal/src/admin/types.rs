// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared types for the admin-view batch.
//!
//! Same conventions as the other deeper batches (cave-mesh ambient,
//! cave-net cilium, …): `TenantId` for multi-tenant scoping, `Cite`
//! pointing at the upstream Backstage symbol, and a `portal_test_ctx!`
//! macro that asserts both for shape.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Pinned upstream Backstage release this batch tracks.
pub const UPSTREAM_VERSION: &str = "v1.50.3";

/// Upstream repo (without the leading `https://github.com/`).
pub const UPSTREAM_REPO: &str = "backstage/backstage";

/// Multi-tenant identifier — re-exported from `cave_kernel::ns` (sweep-002
/// F2-G adoption, 2026-05-01). Every admin view is scoped to a tenant; routes
/// take `tenant_id` as a query param and refuse to render data the request
/// principal does not own.
pub use cave_kernel::ns::TenantId;

/// Citation pointing at the upstream Backstage symbol a view ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cite {
    pub repo: &'static str,
    pub path: &'static str,
    pub symbol: &'static str,
    pub version: &'static str,
}

impl Cite {
    pub const fn backstage(path: &'static str, symbol: &'static str) -> Self {
        Self { repo: UPSTREAM_REPO, path, symbol, version: UPSTREAM_VERSION }
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

/// Bundles a `Cite` and a `TenantId` for a test, and asserts both for shape.
#[macro_export]
macro_rules! portal_test_ctx {
    ($path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::admin::types::Cite::backstage($path, $symbol);
        let tenant = $crate::admin::types::TenantId::new($tenant).expect("test fixture");
        assert_eq!(cite.version, $crate::admin::types::UPSTREAM_VERSION);
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        (cite, tenant)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cite_url_uses_pinned_version_and_repo() {
        let (cite, tenant) = portal_test_ctx!(
            "plugins/permission-react/src/index.ts",
            "PermissionApi",
            "tenant-types-cite"
        );
        assert!(cite.url().contains(UPSTREAM_VERSION));
        assert!(cite.url().contains(UPSTREAM_REPO));
        assert_eq!(tenant.as_str(), "tenant-types-cite");
    }

    #[test]
    fn cite_display_includes_all_fields() {
        let (cite, _t) = portal_test_ctx!(
            "packages/core-components/src/layout/Page/Page.tsx",
            "Page",
            "tenant-types-display"
        );
        let s = format!("{cite}");
        assert!(s.contains("backstage/backstage"));
        assert!(s.contains("Page.tsx"));
        assert!(s.contains("Page"));
        assert!(s.contains(UPSTREAM_VERSION));
    }

    #[test]
    fn tenant_id_round_trips_through_serde() {
        let (_cite, tenant) = portal_test_ctx!(
            "plugins/permission-react/src/index.ts",
            "PermissionApi",
            "tenant-types-serde"
        );
        let json = serde_json::to_string(&tenant).unwrap();
        let back: TenantId = serde_json::from_str(&json).unwrap();
        assert_eq!(tenant, back);
    }
}
