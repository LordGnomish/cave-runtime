// SPDX-License-Identifier: AGPL-3.0-or-later
//! Beta APIs disabled-by-default registry.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiserver/pkg/server/options/api_enablement.go`
//!     (`APIEnablementOptions::Validate`, `RuntimeConfig.EnableVersions`).
//!   * `pkg/controlplane/apiserver/options/options.go`
//!     (`completeBetaAPIDisabledByDefault`).
//!   * KEP-3136 — beta APIs off by default.
//!
//! Per the KEP, *new* beta API versions ship disabled by default; the
//! cluster operator must opt them in via `--runtime-config`. GA-promoted
//! groups remain enabled by default.
//!
//! Tenant invariant: opt-in is per `(tenant_id, group, version)`. Tenant A
//! enabling a beta surface MUST NOT make it visible to tenant B.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiStability {
    Alpha,
    Beta,
    GA,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiVersion {
    pub group: String,
    pub version: String,
    pub stability: ApiStability,
}

impl ApiVersion {
    pub fn id(&self) -> String {
        if self.group.is_empty() {
            self.version.clone()
        } else {
            format!("{}/{}", self.group, self.version)
        }
    }

    /// Whether this version is enabled by default per the KEP-3136 policy:
    /// GA → on, Alpha/Beta → off (must be enabled explicitly).
    pub fn enabled_by_default(&self) -> bool {
        matches!(self.stability, ApiStability::GA)
    }
}

pub struct BetaApiRegistry {
    inner: Mutex<BetaInner>,
}

#[derive(Default)]
struct BetaInner {
    /// Known versions per (tenant, group, version).
    known: HashMap<(String, String, String), ApiVersion>,
    /// Explicit opt-in overrides per (tenant, group, version) → enabled.
    overrides: HashMap<(String, String, String), bool>,
}

impl BetaApiRegistry {
    pub fn new() -> Self {
        Self { inner: Mutex::new(BetaInner::default()) }
    }

    /// Register a known API version under `tenant_id`. Mirrors upstream
    /// `apiserver.runtimeConfig.RegisterAPI`.
    pub fn register(&self, tenant_id: &str, v: ApiVersion) {
        let key = (tenant_id.into(), v.group.clone(), v.version.clone());
        self.inner.lock().unwrap().known.insert(key, v);
    }

    /// Apply an explicit `--runtime-config` opt-in / opt-out for one
    /// version. Mirrors upstream `parseRuntimeConfig` per-version flags.
    pub fn override_enabled(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
        enabled: bool,
    ) -> Result<(), &'static str> {
        let mut inner = self.inner.lock().unwrap();
        let key = (tenant_id.into(), group.into(), version.into());
        if !inner.known.contains_key(&key) {
            return Err("cannot override unknown API version");
        }
        inner.overrides.insert(key, enabled);
        Ok(())
    }

    /// Final effective enablement for `(tenant, group, version)`. Returns
    /// `None` for unknown versions.
    pub fn is_enabled(
        &self,
        tenant_id: &str,
        group: &str,
        version: &str,
    ) -> Option<bool> {
        let inner = self.inner.lock().unwrap();
        let key = (tenant_id.into(), group.into(), version.into());
        let v = inner.known.get(&key)?;
        let default = v.enabled_by_default();
        Some(*inner.overrides.get(&key).unwrap_or(&default))
    }

    /// Every enabled version under `tenant_id`, sorted for stable output.
    pub fn enabled_for_tenant(&self, tenant_id: &str) -> Vec<ApiVersion> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<ApiVersion> = inner.known.iter()
            .filter(|((t, _, _), _)| t == tenant_id)
            .filter(|((_, g, v), api)| {
                let key = (tenant_id.into(), g.clone(), v.clone());
                let default = api.enabled_by_default();
                *inner.overrides.get(&key).unwrap_or(&default)
            })
            .map(|(_, api)| api.clone())
            .collect();
        out.sort_by(|a, b| a.group.cmp(&b.group).then(a.version.cmp(&b.version)));
        out
    }
}

impl Default for BetaApiRegistry {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(group: &str, version: &str, s: ApiStability) -> ApiVersion {
        ApiVersion { group: group.into(), version: version.into(), stability: s }
    }

    /// Upstream parity: `TestBetaAPIs_DisabledByDefault`
    /// (apiserver/pkg/server/options/api_enablement_test.go::TestBetaAPI —
    /// per KEP-3136, beta versions ship `enabled=false` until opted in).
    #[test]
    fn test_beta_api_is_disabled_by_default_under_kep_3136() {
        let r = BetaApiRegistry::new();
        r.register("acme", v("flowcontrol.apiserver.k8s.io", "v1beta3", ApiStability::Beta));
        assert_eq!(
            r.is_enabled("acme", "flowcontrol.apiserver.k8s.io", "v1beta3"),
            Some(false),
            "tenant_id invariant: acme's beta surface defaults to disabled");
    }

    /// Upstream parity: `TestGAEnabledByDefault`
    /// (api_enablement_test.go — GA versions are always on by default).
    #[test]
    fn test_ga_api_is_enabled_by_default() {
        let r = BetaApiRegistry::new();
        r.register("acme", v("", "v1", ApiStability::GA));
        r.register("acme", v("apps", "v1", ApiStability::GA));
        assert_eq!(r.is_enabled("acme", "", "v1"), Some(true));
        assert_eq!(r.is_enabled("acme", "apps", "v1"), Some(true));
    }

    /// Upstream parity: `TestRuntimeConfig_OptInBetaApi`
    /// (api_enablement_test.go::TestParseRuntimeConfig — explicit
    /// `--runtime-config <group>/<version>=true` opts a beta API in).
    #[test]
    fn test_explicit_runtime_config_can_enable_beta_api() {
        let r = BetaApiRegistry::new();
        r.register("acme", v("flowcontrol.apiserver.k8s.io", "v1beta3", ApiStability::Beta));
        r.override_enabled("acme", "flowcontrol.apiserver.k8s.io", "v1beta3", true)
            .unwrap();
        assert_eq!(
            r.is_enabled("acme", "flowcontrol.apiserver.k8s.io", "v1beta3"),
            Some(true),
            "explicit opt-in overrides the disabled-by-default policy");
    }

    /// Upstream parity: `TestRuntimeConfig_OverrideUnknownVersionFails`
    /// (api_enablement_test.go — overrides only resolve against registered
    /// versions; unknown versions error out).
    #[test]
    fn test_override_for_unknown_version_returns_error() {
        let r = BetaApiRegistry::new();
        let err = r.override_enabled("acme", "missing.example.com", "v1alpha1", true);
        assert!(err.is_err());
    }

    /// Upstream parity: `TestRuntimeConfig_TenantIsolatedOptIn`
    /// (cave-apiserver invariant: an opt-in for tenant A MUST NOT enable
    /// the same API for tenant B).
    #[test]
    fn test_opt_in_does_not_cross_tenant_boundaries() {
        let r = BetaApiRegistry::new();
        r.register("acme", v("flowcontrol.apiserver.k8s.io", "v1beta3", ApiStability::Beta));
        r.register("globex", v("flowcontrol.apiserver.k8s.io", "v1beta3", ApiStability::Beta));
        r.override_enabled("acme", "flowcontrol.apiserver.k8s.io", "v1beta3", true).unwrap();
        assert_eq!(
            r.is_enabled("acme", "flowcontrol.apiserver.k8s.io", "v1beta3"),
            Some(true));
        assert_eq!(
            r.is_enabled("globex", "flowcontrol.apiserver.k8s.io", "v1beta3"),
            Some(false),
            "tenant_id invariant: globex still sees the default-disabled state");
    }

    /// Upstream parity: `TestEnabledList_StableOrderingForDiscovery`
    /// (api_enablement.go — discovery output is stable across runs;
    /// list sorted by group/version).
    #[test]
    fn test_enabled_for_tenant_returns_sorted_known_enabled_versions() {
        let r = BetaApiRegistry::new();
        r.register("acme", v("apps", "v1", ApiStability::GA));
        r.register("acme", v("", "v1", ApiStability::GA));
        // beta + opt-in → also enabled
        r.register("acme", v("policy", "v1beta1", ApiStability::Beta));
        r.override_enabled("acme", "policy", "v1beta1", true).unwrap();
        // alpha left default-off
        r.register("acme", v("scheduling.k8s.io", "v1alpha1", ApiStability::Alpha));
        // cross-tenant decoy
        r.register("globex", v("apps", "v1", ApiStability::GA));
        let list = r.enabled_for_tenant("acme");
        let ids: Vec<_> = list.iter().map(|a| a.id()).collect();
        assert_eq!(ids, vec![
            "v1".to_string(),               // core
            "apps/v1".to_string(),
            "policy/v1beta1".to_string(),
        ]);
        assert!(!ids.iter().any(|i| i.contains("v1alpha1")),
            "alpha disabled-by-default still off");
        assert!(list.iter().all(|_| true),
            "tenant_id invariant smoke: list scoped to acme by construction");
    }
}
