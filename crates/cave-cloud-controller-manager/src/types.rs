//! Shared types for cave-cloud-controller-manager.
//!
//! `TenantId` is re-exported from `cave_kernel::ns` (sweep-002 F2-G adoption,
//! 2026-05-01) — the local copy used to mirror conventions in
//! `cave-controller-manager`, but the kernel newtype now provides a single
//! DNS-1123-validated surface across the platform. `Cite` stays local.

use serde::{Deserialize, Serialize};
use std::fmt;

pub use cave_kernel::ns::TenantId;

/// Pinned upstream Kubernetes release this scaffold tracks. The matching
/// out-of-tree provider versions live in `providers::*::PROVIDER_VERSION`.
pub const UPSTREAM_VERSION: &str = "v1.36.0";

/// Citation pointing at the upstream source a piece of local code is a parity
/// port of. `repo` defaults to kubernetes/kubernetes; out-of-tree providers
/// override it (`hetznercloud/hcloud-cloud-controller-manager`, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cite {
    pub repo: &'static str,
    pub path: &'static str,
    pub symbol: &'static str,
    pub version: &'static str,
}

impl Cite {
    pub const fn k8s(path: &'static str, symbol: &'static str) -> Self {
        Self { repo: "kubernetes/kubernetes", path, symbol, version: UPSTREAM_VERSION }
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

/// Names of the out-of-tree providers this scaffold ships. Mirrors the
/// `provider-id` URI scheme (`hcloud://`, `azure://`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderName {
    Hetzner,
    Azure,
}

impl ProviderName {
    /// Provider-id URI scheme for `<scheme>://<id>` node identifiers.
    pub const fn provider_id_scheme(self) -> &'static str {
        match self {
            ProviderName::Hetzner => "hcloud",
            ProviderName::Azure => "azure",
        }
    }
}

impl fmt::Display for ProviderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ProviderName::Hetzner => "hetzner",
            ProviderName::Azure => "azure",
        })
    }
}

/// Outcome of a single reconciliation pass — flatter than upstream's
/// `controllerruntime.Result` but enough to drive a state machine in tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reconcile {
    NoOp,
    Annotate(u32),
    Untaint(u32),
    AllocateIp(u32),
    Update(u32),
    Delete(u32),
    Requeue,
}

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("invalid cloud config for {provider}: {reason}")]
    InvalidConfig { provider: ProviderName, reason: String },
    #[error("tenant {tenant} not authorized for {kind}/{name}")]
    TenantDenied { tenant: TenantId, kind: &'static str, name: String },
    #[error("provider {provider} returned upstream error: {reason}")]
    Upstream { provider: ProviderName, reason: String },
    #[error("not yet implemented: {0}")]
    Unimplemented(&'static str),
}

/// Bundles `Cite` and `TenantId`, sanity-checks both, and returns them so the
/// test can assert against either one. Same shape as the macro in
/// `cave-controller-manager`, but accepts an extra `repo` token for the
/// out-of-tree providers.
#[macro_export]
macro_rules! test_ctx {
    // k8s upstream
    ($path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::types::Cite::k8s($path, $symbol);
        let tenant = $crate::types::TenantId::new($tenant).expect("test fixture: tenant id must be DNS-1123");
        assert_eq!(cite.version, $crate::types::UPSTREAM_VERSION);
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        (cite, tenant)
    }};
    // out-of-tree provider
    (ext: $repo:expr, $version:expr, $path:expr, $symbol:expr, $tenant:expr) => {{
        let cite = $crate::types::Cite::ext($repo, $path, $symbol, $version);
        let tenant = $crate::types::TenantId::new($tenant).expect("test fixture: tenant id must be DNS-1123");
        assert!(!tenant.as_str().is_empty(), "tenant_id must not be empty");
        assert!(cite.version.starts_with('v'), "version must look like a tag");
        (cite, tenant)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k8s_cite_resolves_to_pinned_url() {
        let (cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "Interface",
            "tenant-types-k8s"
        );
        assert!(cite.url().contains(UPSTREAM_VERSION));
        assert_eq!(cite.repo, "kubernetes/kubernetes");
        assert_eq!(tenant.as_str(), "tenant-types-k8s");
    }

    #[test]
    fn ext_cite_keeps_provider_repo_and_version() {
        let (cite, _tenant) = test_ctx!(
            ext: "hetznercloud/hcloud-cloud-controller-manager",
            "v1.30.1",
            "hcloud/instances.go",
            "InstanceMetadata",
            "tenant-types-ext"
        );
        assert_eq!(cite.repo, "hetznercloud/hcloud-cloud-controller-manager");
        assert_eq!(cite.version, "v1.30.1");
    }

    #[test]
    fn provider_name_uri_schemes_match_upstream_provider_id() {
        let (_cite, _tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/cloud.go",
            "ProviderID",
            "tenant-types-providerid"
        );
        assert_eq!(ProviderName::Hetzner.provider_id_scheme(), "hcloud");
        assert_eq!(ProviderName::Azure.provider_id_scheme(), "azure");
    }
}
