//! mTLS policy management.
//!
//! Implements PeerAuthentication (STRICT / PERMISSIVE / DISABLE) per namespace
//! or workload.  Provides a `validate_peer` function that the data-plane proxy
//! calls to decide whether to accept an incoming connection.

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
    /// Keyed by "namespace/policy-name"
    policies: Arc<RwLock<HashMap<String, PeerAuthentication>>>,
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
        }
    }

    // ─── CRUD ────────────────────────────────────────────────

    pub fn upsert_policy(&self, policy: PeerAuthentication) {
        let key = format!("{}/{}", policy.namespace, policy.name);
        let mut map = self.policies.write().unwrap();
        map.insert(key, policy);
    }

    pub fn remove_policy(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        let mut map = self.policies.write().unwrap();
        map.remove(&key);
    }

    pub fn list_policies(&self) -> Vec<PeerAuthentication> {
        let map = self.policies.read().unwrap();
        map.values().cloned().collect()
    }

    pub fn get_policy(&self, namespace: &str, name: &str) -> Option<PeerAuthentication> {
        let key = format!("{namespace}/{name}");
        let map = self.policies.read().unwrap();
        map.get(&key).cloned()
    }

    // ─── Policy resolution ───────────────────────────────────

    /// Determine the effective mTLS mode for a workload (namespace + labels).
    ///
    /// Priority (highest first):
    ///   1. Workload-specific policy (selector matches)
    ///   2. Namespace-wide policy (no selector / empty selector)
    ///   3. Mesh-wide default (PERMISSIVE)
    pub fn effective_mode(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> MtlsMode {
        let map = self.policies.read().unwrap();

        let mut namespace_mode: Option<MtlsMode> = None;
        let mut workload_mode: Option<MtlsMode> = None;

        for policy in map.values() {
            if policy.namespace != namespace {
                continue;
            }
            let is_namespace_wide = policy
                .selector
                .as_ref()
                .map(|s| s.is_empty())
                .unwrap_or(true);
            if is_namespace_wide {
                // Namespace-wide policy
                namespace_mode = Some(policy.mtls.mode.clone());
            } else if let Some(selector) = &policy.selector {
                // Workload-specific — selector must be a subset of workload labels
                if selector
                    .iter()
                    .all(|(k, v)| workload_labels.get(k).map(|vv| vv == v).unwrap_or(false))
                {
                    workload_mode = Some(policy.mtls.mode.clone());
                }
            }
        }

        workload_mode
            .or(namespace_mode)
            .unwrap_or(MtlsMode::Permissive)
    }

    // ─── Enforcement ─────────────────────────────────────────

    /// Validate an incoming peer connection against the effective mTLS policy.
    pub fn validate_peer(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
        ctx: &TlsContext,
    ) -> MeshResult<()> {
        let mode = self.effective_mode(namespace, workload_labels);

        debug!(
            namespace = %namespace,
            is_mtls = %ctx.is_mtls,
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
            }
            MtlsMode::Permissive => {
                // Both plaintext and mTLS are accepted
            }
            MtlsMode::Disable => {
                // mTLS disabled — plaintext only (mTLS still accepted for compatibility)
            }
        }
        Ok(())
    }

    /// Extract the SPIFFE principal from the peer certificate (passed in ctx).
    pub fn peer_principal(ctx: &TlsContext) -> Option<&str> {
        ctx.peer_principal.as_deref()
    }
}
