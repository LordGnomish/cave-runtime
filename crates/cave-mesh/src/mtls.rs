// SPDX-License-Identifier: AGPL-3.0-or-later
//! mTLS policy management — PeerAuthentication enforcement.
//!
//! Priority (highest first):
//!   1. Workload-specific policy (selector matches)
//!   2. Namespace-wide policy (no / empty selector)
//!   3. Mesh-wide default (PERMISSIVE)
//!
//! Supports:
//!   • Per-port mTLS mode overrides
//!   • Auto-mTLS (STRICT everywhere when auto_mtls_enabled)
//!   • SPIFFE identity extraction

use crate::{
    error::{MeshError, MeshResult},
    models::{MtlsMode, PeerAuthentication, TlsContext},
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;

/// Manages PeerAuthentication policies and enforces mTLS rules.
#[derive(Debug, Clone)]
pub struct MtlsManager {
    policies: Arc<RwLock<HashMap<String, PeerAuthentication>>>,
    /// When true, STRICT mTLS is enforced mesh-wide unless overridden.
    auto_mtls_enabled: Arc<RwLock<bool>>,
}

impl Default for MtlsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MtlsManager {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            auto_mtls_enabled: Arc::new(RwLock::new(false)),
        }
    }

    /// Enable / disable automatic mesh-wide STRICT mTLS.
    pub fn set_auto_mtls(&self, enabled: bool) {
        *self.auto_mtls_enabled.write().unwrap() = enabled;
    }

    pub fn auto_mtls_enabled(&self) -> bool {
        *self.auto_mtls_enabled.read().unwrap()
    }

    // ─── CRUD ────────────────────────────────────────────────

    pub fn upsert_policy(&self, policy: PeerAuthentication) {
        let key = format!("{}/{}", policy.namespace, policy.name);
        self.policies.write().unwrap().insert(key, policy);
    }

    pub fn remove_policy(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        self.policies.write().unwrap().remove(&key);
    }

    pub fn list_policies(&self) -> Vec<PeerAuthentication> {
        self.policies.read().unwrap().values().cloned().collect()
    }

    pub fn get_policy(&self, namespace: &str, name: &str) -> Option<PeerAuthentication> {
        let key = format!("{namespace}/{name}");
        self.policies.read().unwrap().get(&key).cloned()
    }

    // ─── Policy resolution ───────────────────────────────────

    /// Determine the effective mTLS mode for a workload at an optional port.
    pub fn effective_mode(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
        port: Option<u16>,
    ) -> MtlsMode {
        let map = self.policies.read().unwrap();

        let mut namespace_mode: Option<MtlsMode> = None;
        let mut workload_mode: Option<MtlsMode> = None;

        for policy in map.values() {
            if policy.namespace != namespace {
                continue;
            }
            let is_namespace_wide =
                policy.selector.as_ref().map(|s| s.is_empty()).unwrap_or(true);

            if is_namespace_wide {
                // Check port-level override first
                let effective = if let Some(p) = port {
                    policy
                        .port_level_mtls
                        .get(&p)
                        .map(|c| c.mode.clone())
                        .unwrap_or_else(|| policy.mtls.mode.clone())
                } else {
                    policy.mtls.mode.clone()
                };
                namespace_mode = Some(effective);
            } else if let Some(selector) = &policy.selector {
                let matches = selector.iter().all(|(k, v)| {
                    workload_labels.get(k).map(|vv| vv == v).unwrap_or(false)
                });
                if matches {
                    let effective = if let Some(p) = port {
                        policy
                            .port_level_mtls
                            .get(&p)
                            .map(|c| c.mode.clone())
                            .unwrap_or_else(|| policy.mtls.mode.clone())
                    } else {
                        policy.mtls.mode.clone()
                    };
                    workload_mode = Some(effective);
                }
            }
        }

        let resolved = workload_mode.or(namespace_mode);

        // Auto-mTLS: if no explicit policy, enforce STRICT mesh-wide
        if *self.auto_mtls_enabled.read().unwrap() {
            return resolved.unwrap_or(MtlsMode::Strict);
        }

        // Resolve Unset → Permissive (mesh default)
        match resolved {
            Some(MtlsMode::Unset) | None => MtlsMode::Permissive,
            Some(mode) => mode,
        }
    }

    // ─── Enforcement ─────────────────────────────────────────

    pub fn validate_peer(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
        ctx: &TlsContext,
        port: Option<u16>,
    ) -> MeshResult<()> {
        let mode = self.effective_mode(namespace, workload_labels, port);

        debug!(
            namespace = %namespace,
            is_mtls = ctx.is_mtls,
            mode = ?mode,
            "mTLS validation"
        );

        match mode {
            MtlsMode::Strict => {
                if !ctx.is_mtls {
                    return Err(MeshError::MtlsRejected(
                        "STRICT mode requires mTLS — plaintext rejected".to_string(),
                    ));
                }
                // Validate SPIFFE SAN if present
                if ctx.is_mtls && ctx.peer_cert_san.is_empty() && ctx.peer_principal.is_none() {
                    return Err(MeshError::MtlsRejected(
                        "STRICT mode: mTLS peer certificate has no SPIFFE identity".to_string(),
                    ));
                }
            }
            MtlsMode::Permissive | MtlsMode::Unset => {}
            MtlsMode::Disable => {}
        }
        Ok(())
    }

    /// Extract SPIFFE principal from the peer TLS context.
    pub fn peer_principal(ctx: &TlsContext) -> Option<&str> {
        ctx.peer_principal.as_deref().or_else(|| {
            ctx.peer_cert_san.iter().find(|s| s.starts_with("spiffe://")).map(|s| s.as_str())
        })
    }
}
