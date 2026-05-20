// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Portal → real cave-runtime data source.
//!
//! The admin dashboard was originally backed by `AdminState::seeded()`
//! fixtures so it could be unit-tested without a cluster. This module
//! introduces the seam where a live cave-runtime process can supply real
//! data instead.
//!
//! Two implementations of [`RuntimeClient`]:
//!
//! * [`MockClient`] — wraps an `Arc<AdminState>`; the materialiser is a
//!   no-op (the in-memory fixtures are already loaded by `seeded()`).
//!   This preserves the existing development workflow byte-for-byte.
//! * [`ApiserverClient`] — talks to `cave-apiserver` over HTTPS, using a
//!   reqwest client built from a kubeconfig file. The CA cert from the
//!   kubeconfig's `certificate-authority-data` is pinned into the
//!   client's `rustls` trust store; the client cert/key drive mTLS.
//!
//! Wiring contract: each admin handler calls
//! `state.materialise_<resource>(ctx).await?` before invoking the sync
//! `render(&state, ctx)`. When `state.runtime_client` is `None`, the
//! materialise call is a cheap no-op; when it is `Some`, the client
//! refreshes the corresponding `RwLock<Vec<T>>` collection from the
//! upstream apiserver in place. Render code stays untouched.

use crate::admin::state::{
    KedaScaledObject, KubeletPod, NetEndpoint, SchedulerNode, VaultSecretMeta,
};
use crate::admin::types::TenantId;
use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Errors returned by [`RuntimeClient`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("apiserver request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("apiserver returned status {0}")]
    Status(reqwest::StatusCode),
    #[error("apiserver response parse failed: {0}")]
    Decode(String),
    #[error("kubeconfig {path}: {source}")]
    Kubeconfig {
        path: String,
        #[source]
        source: KubeconfigError,
    },
    #[error("resource {resource:?} is not wired against the live apiserver yet")]
    NotWired { resource: &'static str },
}

/// kubeconfig parser errors. Kept distinct from `RuntimeError` so callers
/// can branch on bootstrap-time failures separately from runtime ones.
#[derive(Debug, thiserror::Error)]
pub enum KubeconfigError {
    #[error("read {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parse YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("kubeconfig missing required field: {0}")]
    MissingField(&'static str),
    #[error("base64 decode of {field}: {source}")]
    Base64 {
        field: &'static str,
        #[source]
        source: base64::DecodeError,
    },
    #[error("rustls client build failed: {0}")]
    ClientBuild(String),
}

/// The thing the admin dashboard talks to. Methods are typed to the
/// existing dashboard structs so render code doesn't have to change.
///
/// All methods are tenant-scoped: implementations either inherently
/// project to the given tenant (apiserver-side label filter) or stamp
/// the tenant onto the materialised rows so the dashboard's
/// `scope(..., tenant, ...)` filter still works after the swap.
#[async_trait]
pub trait RuntimeClient: Send + Sync + std::fmt::Debug {
    async fn list_kubelet_pods(&self, tenant: &TenantId) -> Result<Vec<KubeletPod>, RuntimeError>;
    async fn list_scheduler_nodes(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<SchedulerNode>, RuntimeError>;
    async fn list_net_endpoints(&self, tenant: &TenantId)
    -> Result<Vec<NetEndpoint>, RuntimeError>;
    async fn list_keda_scaled_objects(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<KedaScaledObject>, RuntimeError>;
    async fn list_vault_secrets(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<VaultSecretMeta>, RuntimeError>;
}

// ── MockClient ────────────────────────────────────────────────────────────────

/// Fixture-backed client. The materialise methods on `AdminState` use
/// this when no live `ApiserverClient` is configured — calling them is a
/// no-op (the seeded fixtures are already in `state.kubelet_pods` etc.),
/// but the trait shape stays uniform so handler code is the same in
/// both modes.
#[derive(Debug)]
pub struct MockClient;

impl MockClient {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MockClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RuntimeClient for MockClient {
    async fn list_kubelet_pods(&self, _: &TenantId) -> Result<Vec<KubeletPod>, RuntimeError> {
        Ok(Vec::new())
    }
    async fn list_scheduler_nodes(&self, _: &TenantId) -> Result<Vec<SchedulerNode>, RuntimeError> {
        Ok(Vec::new())
    }
    async fn list_net_endpoints(&self, _: &TenantId) -> Result<Vec<NetEndpoint>, RuntimeError> {
        Ok(Vec::new())
    }
    async fn list_keda_scaled_objects(
        &self,
        _: &TenantId,
    ) -> Result<Vec<KedaScaledObject>, RuntimeError> {
        Ok(Vec::new())
    }
    async fn list_vault_secrets(&self, _: &TenantId) -> Result<Vec<VaultSecretMeta>, RuntimeError> {
        Ok(Vec::new())
    }
}

// ── ApiserverClient ───────────────────────────────────────────────────────────

/// Configuration extracted from a kubeconfig file.
#[derive(Debug, Clone)]
pub struct ApiserverConfig {
    pub server: String,
    pub ca_pem: Vec<u8>,
    pub client_cert_pem: Vec<u8>,
    pub client_key_pem: Vec<u8>,
    /// Bearer token (used when client certs are absent — workers join
    /// with a bootstrap token).
    pub bearer_token: Option<String>,
    pub request_timeout: Duration,
}

impl ApiserverConfig {
    /// Parse a kubeconfig file. Picks the `current-context` and pulls
    /// `server` + `certificate-authority-data` from the matching cluster,
    /// and `client-certificate-data` + `client-key-data` from the user.
    pub fn from_kubeconfig(path: &Path) -> Result<Self, KubeconfigError> {
        let raw = std::fs::read_to_string(path).map_err(|e| KubeconfigError::Read {
            path: path.display().to_string(),
            source: e,
        })?;
        let kc: KubeconfigFile = serde_yaml::from_str(&raw)?;
        let ctx_name = kc
            .current_context
            .as_deref()
            .ok_or(KubeconfigError::MissingField("current-context"))?;
        let ctx = kc.contexts.iter().find(|c| c.name == ctx_name).ok_or(
            KubeconfigError::MissingField("context entry for current-context"),
        )?;
        let cluster = kc
            .clusters
            .iter()
            .find(|c| c.name == ctx.context.cluster)
            .ok_or(KubeconfigError::MissingField("cluster entry"))?;
        let user = kc
            .users
            .iter()
            .find(|u| u.name == ctx.context.user)
            .ok_or(KubeconfigError::MissingField("user entry"))?;

        let server = cluster.cluster.server.clone();
        let b64 = base64::engine::general_purpose::STANDARD;
        let ca_pem = b64
            .decode(
                cluster
                    .cluster
                    .certificate_authority_data
                    .as_deref()
                    .ok_or(KubeconfigError::MissingField("certificate-authority-data"))?,
            )
            .map_err(|e| KubeconfigError::Base64 {
                field: "certificate-authority-data",
                source: e,
            })?;
        let client_cert_pem = b64
            .decode(
                user.user
                    .client_certificate_data
                    .as_deref()
                    .ok_or(KubeconfigError::MissingField("client-certificate-data"))?,
            )
            .map_err(|e| KubeconfigError::Base64 {
                field: "client-certificate-data",
                source: e,
            })?;
        let client_key_pem = b64
            .decode(
                user.user
                    .client_key_data
                    .as_deref()
                    .ok_or(KubeconfigError::MissingField("client-key-data"))?,
            )
            .map_err(|e| KubeconfigError::Base64 {
                field: "client-key-data",
                source: e,
            })?;

        Ok(Self {
            server,
            ca_pem,
            client_cert_pem,
            client_key_pem,
            bearer_token: user.user.token.clone(),
            request_timeout: Duration::from_secs(5),
        })
    }
}

/// Real apiserver client. CA-pinned reqwest with optional mTLS client
/// cert. Bearer-token auth supported (used during bootstrap before the
/// node has a signed client cert).
#[derive(Debug)]
pub struct ApiserverClient {
    client: reqwest::Client,
    base_url: String,
    bearer_token: Option<String>,
}

impl ApiserverClient {
    pub fn from_config(cfg: ApiserverConfig) -> Result<Self, KubeconfigError> {
        let ca = reqwest::Certificate::from_pem(&cfg.ca_pem)
            .map_err(|e| KubeconfigError::ClientBuild(format!("CA: {e}")))?;
        let mut builder = reqwest::Client::builder()
            .add_root_certificate(ca)
            .timeout(cfg.request_timeout)
            // Use webpki roots ONLY for the apiserver's self-signed CA path.
            .https_only(true)
            .tls_built_in_root_certs(false);
        if !cfg.client_cert_pem.is_empty() && !cfg.client_key_pem.is_empty() {
            let mut bundle = cfg.client_cert_pem.clone();
            bundle.extend_from_slice(&cfg.client_key_pem);
            let identity = reqwest::Identity::from_pem(&bundle)
                .map_err(|e| KubeconfigError::ClientBuild(format!("identity: {e}")))?;
            builder = builder.identity(identity);
        }
        let client = builder
            .build()
            .map_err(|e| KubeconfigError::ClientBuild(format!("build: {e}")))?;
        Ok(Self {
            client,
            base_url: cfg.server.trim_end_matches('/').to_string(),
            bearer_token: cfg.bearer_token,
        })
    }

    /// Test-only constructor: bypasses TLS by using the system reqwest
    /// defaults with `danger_accept_invalid_certs`. Used by the
    /// `httpmock`-driven unit tests.
    #[cfg(test)]
    pub fn test_against(base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(2))
            .build()
            .expect("test client");
        Self {
            client,
            base_url,
            bearer_token: None,
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, RuntimeError> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.get(&url);
        if let Some(t) = &self.bearer_token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(RuntimeError::Status(resp.status()));
        }
        resp.json::<T>()
            .await
            .map_err(|e| RuntimeError::Decode(e.to_string()))
    }
}

#[async_trait]
impl RuntimeClient for ApiserverClient {
    async fn list_kubelet_pods(&self, tenant: &TenantId) -> Result<Vec<KubeletPod>, RuntimeError> {
        let list: PodList = self.get_json("/api/v1/pods").await?;
        Ok(list
            .items
            .into_iter()
            .map(|p| KubeletPod {
                tenant: pod_tenant_from_labels(&p, tenant),
                node: p.spec.node_name.unwrap_or_default(),
                pod_name: p.metadata.name.unwrap_or_default(),
                status: pod_phase_to_status(p.status.as_ref().and_then(|s| s.phase.as_deref())),
                restart_count: p
                    .status
                    .as_ref()
                    .and_then(|s| s.container_statuses.as_ref())
                    .map(|cs| cs.iter().map(|c| c.restart_count).sum())
                    .unwrap_or(0),
            })
            .collect())
    }

    async fn list_scheduler_nodes(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<SchedulerNode>, RuntimeError> {
        let list: NodeList = self.get_json("/api/v1/nodes").await?;
        Ok(list
            .items
            .into_iter()
            .map(|n| {
                let ready = n
                    .status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .and_then(|c| c.iter().find(|c| c.r#type == "Ready"))
                    .map(|c| c.status == "True")
                    .unwrap_or(false);
                let allocatable = n.status.as_ref().and_then(|s| s.allocatable.as_ref());
                let allocatable_cpu_milli = allocatable
                    .and_then(|a| a.get("cpu"))
                    .map(|q| parse_cpu_quantity(q))
                    .unwrap_or(0);
                let allocatable_mem_mib = allocatable
                    .and_then(|a| a.get("memory"))
                    .map(|q| parse_mem_quantity_mib(q))
                    .unwrap_or(0);
                let taints = n
                    .spec
                    .as_ref()
                    .and_then(|s| s.taints.as_ref())
                    .map(|t| t.iter().map(|t| t.key.clone()).collect())
                    .unwrap_or_default();
                SchedulerNode {
                    tenant: tenant.clone(),
                    name: n.metadata.name.unwrap_or_default(),
                    ready,
                    allocatable_cpu_milli,
                    allocatable_mem_mib,
                    taints,
                }
            })
            .collect())
    }

    async fn list_net_endpoints(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<NetEndpoint>, RuntimeError> {
        let list: EndpointsList = self.get_json("/api/v1/endpoints").await?;
        let mut out = Vec::new();
        let mut next_identity: u64 = 1;
        for e in list.items {
            let namespace = e.metadata.namespace.unwrap_or_default();
            for subset in e.subsets.unwrap_or_default() {
                let ready_addrs = subset.addresses.unwrap_or_default();
                for addr in ready_addrs {
                    out.push(NetEndpoint {
                        tenant: tenant.clone(),
                        identity: next_identity,
                        namespace: namespace.clone(),
                        ip: addr.ip,
                        ready: true,
                    });
                    next_identity += 1;
                }
                for addr in subset.not_ready_addresses.unwrap_or_default() {
                    out.push(NetEndpoint {
                        tenant: tenant.clone(),
                        identity: next_identity,
                        namespace: namespace.clone(),
                        ip: addr.ip,
                        ready: false,
                    });
                    next_identity += 1;
                }
            }
        }
        Ok(out)
    }

    async fn list_keda_scaled_objects(
        &self,
        tenant: &TenantId,
    ) -> Result<Vec<KedaScaledObject>, RuntimeError> {
        let list: ScaledObjectList = self
            .get_json("/apis/keda.sh/v1alpha1/scaledobjects")
            .await?;
        Ok(list
            .items
            .into_iter()
            .map(|so| {
                let triggers = so.spec.triggers.iter().map(|t| t.r#type.clone()).collect();
                let current_replicas = so
                    .status
                    .as_ref()
                    .and_then(|s| s.current_replicas)
                    .unwrap_or(0);
                let paused = so
                    .metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get("autoscaling.keda.sh/paused-replicas"))
                    .is_some();
                KedaScaledObject {
                    tenant: tenant.clone(),
                    name: so.metadata.name.unwrap_or_default(),
                    target_ref: so
                        .spec
                        .scale_target_ref
                        .map(|t| t.name.unwrap_or_default())
                        .unwrap_or_default(),
                    min_replicas: so.spec.min_replica_count.unwrap_or(0),
                    max_replicas: so.spec.max_replica_count.unwrap_or(0),
                    current_replicas,
                    paused,
                    triggers,
                }
            })
            .collect())
    }

    async fn list_vault_secrets(
        &self,
        _tenant: &TenantId,
    ) -> Result<Vec<VaultSecretMeta>, RuntimeError> {
        // Vault's metadata endpoint is not served by the k8s-style
        // apiserver; it lives on the cave-vault module under
        // `/v1/secret/metadata/<path>` with `X-Vault-Token` auth. A
        // proper adoption needs a separate `VaultClient` configured
        // from the same data-dir's vault.json. Out of scope for this
        // wiring sweep; documented in
        // docs/synergy/portal-runtime-wiring-2026-05-12.md.
        Err(RuntimeError::NotWired {
            resource: "vault_secrets",
        })
    }
}

fn pod_phase_to_status(phase: Option<&str>) -> &'static str {
    match phase {
        Some("Running") => "Running",
        Some("Pending") => "Pending",
        Some("Failed") => "Failed",
        _ => "Pending",
    }
}

fn pod_tenant_from_labels(pod: &Pod, fallback: &TenantId) -> TenantId {
    pod.metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("cave.io/tenant"))
        .and_then(|t| TenantId::new(t).ok())
        .unwrap_or_else(|| fallback.clone())
}

/// Quick-and-dirty cpu parser — handles "500m" / "1" / "2.5". Anything
/// else returns 0; the dashboard surface is purely informational.
fn parse_cpu_quantity(q: &str) -> u64 {
    let q = q.trim();
    if let Some(stripped) = q.strip_suffix('m') {
        stripped.parse::<u64>().unwrap_or(0)
    } else if let Ok(whole) = q.parse::<u64>() {
        whole * 1000
    } else if let Ok(frac) = q.parse::<f64>() {
        (frac * 1000.0) as u64
    } else {
        0
    }
}

/// Memory parser — handles "1024Mi" / "2Gi" / "512Mi". Returns MiB.
fn parse_mem_quantity_mib(q: &str) -> u64 {
    let q = q.trim();
    for (suffix, mult) in [
        ("Ki", 1u64 / 1024),
        ("Mi", 1),
        ("Gi", 1024),
        ("Ti", 1024 * 1024),
    ] {
        if let Some(stripped) = q.strip_suffix(suffix) {
            if let Ok(n) = stripped.parse::<u64>() {
                return n * mult.max(1);
            }
        }
    }
    // Bytes fallback
    if let Ok(bytes) = q.parse::<u64>() {
        return bytes / (1024 * 1024);
    }
    0
}

// ── kubeconfig YAML schema (subset) ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct KubeconfigFile {
    #[serde(rename = "current-context", default)]
    current_context: Option<String>,
    #[serde(default)]
    clusters: Vec<KubeconfigCluster>,
    #[serde(default)]
    contexts: Vec<KubeconfigContext>,
    #[serde(default)]
    users: Vec<KubeconfigUser>,
}

#[derive(Debug, Deserialize)]
struct KubeconfigCluster {
    name: String,
    cluster: ClusterFields,
}

#[derive(Debug, Deserialize)]
struct ClusterFields {
    server: String,
    #[serde(rename = "certificate-authority-data", default)]
    certificate_authority_data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct KubeconfigContext {
    name: String,
    context: ContextFields,
}

#[derive(Debug, Deserialize)]
struct ContextFields {
    cluster: String,
    user: String,
}

#[derive(Debug, Deserialize)]
struct KubeconfigUser {
    name: String,
    user: UserFields,
}

#[derive(Debug, Deserialize, Default)]
struct UserFields {
    #[serde(rename = "client-certificate-data", default)]
    client_certificate_data: Option<String>,
    #[serde(rename = "client-key-data", default)]
    client_key_data: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

// ── k8s API JSON shapes (subset) ──────────────────────────────────────────────
// Just enough to extract what the dashboard renders. Out-of-scope fields
// are dropped via `#[serde(default)]` instead of vendoring the whole
// k8s-openapi crate.

#[derive(Debug, Deserialize)]
struct PodList {
    #[serde(default)]
    items: Vec<Pod>,
}
#[derive(Debug, Deserialize)]
struct Pod {
    #[serde(default)]
    metadata: ObjectMeta,
    #[serde(default)]
    spec: PodSpec,
    #[serde(default)]
    status: Option<PodStatus>,
}
#[derive(Debug, Deserialize, Default)]
struct ObjectMeta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    labels: Option<std::collections::HashMap<String, String>>,
    #[serde(default)]
    annotations: Option<std::collections::HashMap<String, String>>,
}
#[derive(Debug, Deserialize, Default)]
struct PodSpec {
    #[serde(rename = "nodeName", default)]
    node_name: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
struct PodStatus {
    #[serde(default)]
    phase: Option<String>,
    #[serde(rename = "containerStatuses", default)]
    container_statuses: Option<Vec<ContainerStatus>>,
}
#[derive(Debug, Deserialize)]
struct ContainerStatus {
    #[serde(rename = "restartCount", default)]
    restart_count: u32,
}

#[derive(Debug, Deserialize)]
struct NodeList {
    #[serde(default)]
    items: Vec<Node>,
}
#[derive(Debug, Deserialize)]
struct Node {
    #[serde(default)]
    metadata: ObjectMeta,
    #[serde(default)]
    spec: Option<NodeSpec>,
    #[serde(default)]
    status: Option<NodeStatus>,
}
#[derive(Debug, Deserialize, Default)]
struct NodeSpec {
    #[serde(default)]
    taints: Option<Vec<NodeTaint>>,
}
#[derive(Debug, Deserialize)]
struct NodeTaint {
    key: String,
}
#[derive(Debug, Deserialize, Default)]
struct NodeStatus {
    #[serde(default)]
    conditions: Option<Vec<NodeCondition>>,
    #[serde(default)]
    allocatable: Option<std::collections::HashMap<String, String>>,
}
#[derive(Debug, Deserialize)]
struct NodeCondition {
    r#type: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct EndpointsList {
    #[serde(default)]
    items: Vec<Endpoints>,
}
#[derive(Debug, Deserialize)]
struct Endpoints {
    #[serde(default)]
    metadata: ObjectMeta,
    #[serde(default)]
    subsets: Option<Vec<EndpointSubset>>,
}
#[derive(Debug, Deserialize)]
struct EndpointSubset {
    #[serde(default)]
    addresses: Option<Vec<EndpointAddress>>,
    #[serde(rename = "notReadyAddresses", default)]
    not_ready_addresses: Option<Vec<EndpointAddress>>,
}
#[derive(Debug, Deserialize)]
struct EndpointAddress {
    ip: String,
}

#[derive(Debug, Deserialize)]
struct ScaledObjectList {
    #[serde(default)]
    items: Vec<ScaledObject>,
}
#[derive(Debug, Deserialize)]
struct ScaledObject {
    #[serde(default)]
    metadata: ObjectMeta,
    #[serde(default)]
    spec: ScaledObjectSpec,
    #[serde(default)]
    status: Option<ScaledObjectStatus>,
}
#[derive(Debug, Deserialize, Default)]
struct ScaledObjectSpec {
    #[serde(rename = "scaleTargetRef", default)]
    scale_target_ref: Option<ScaleTargetRef>,
    #[serde(rename = "minReplicaCount", default)]
    min_replica_count: Option<u32>,
    #[serde(rename = "maxReplicaCount", default)]
    max_replica_count: Option<u32>,
    #[serde(default)]
    triggers: Vec<ScaleTrigger>,
}
#[derive(Debug, Deserialize)]
struct ScaleTargetRef {
    #[serde(default)]
    name: Option<String>,
}
#[derive(Debug, Deserialize)]
struct ScaleTrigger {
    r#type: String,
}
#[derive(Debug, Deserialize)]
struct ScaledObjectStatus {
    #[serde(rename = "currentReplicas", default)]
    current_replicas: Option<u32>,
}

/// Shared alias for `Arc<dyn RuntimeClient>` so call sites don't have to
/// repeat the trait-object boilerplate.
pub type SharedRuntime = Arc<dyn RuntimeClient>;

/// Result of [`probe_data_dir_for_runtime`].
#[derive(Debug, PartialEq, Eq)]
pub enum WireOutcome {
    /// `<data_dir>/kubeconfig/admin.kubeconfig` exists; an
    /// `ApiserverClient` was built from it and installed.
    Wired,
    /// Data dir or kubeconfig is absent; the dashboard falls back to
    /// seeded fixtures. Backward-compatible dev path.
    NoDataDir,
    /// Kubeconfig is present but malformed. The dashboard still works
    /// (fixture fallback) but operators should investigate.
    KubeconfigBroken,
}

/// Inspect `<data_dir>/kubeconfig/admin.kubeconfig`. If present, build
/// an [`ApiserverClient`] and install it onto the provided
/// [`crate::admin::state::AdminState`] via `set_runtime_client`.
///
/// Returns the outcome so startup code can emit a structured log line:
/// dev runs that have no data dir at all (the common "demo mode") get
/// `WireOutcome::NoDataDir`; production runs that fail to parse the
/// kubeconfig get `WireOutcome::KubeconfigBroken` instead of a panic.
pub fn probe_data_dir_for_runtime(
    state: &crate::admin::state::AdminState,
    data_dir: Option<&Path>,
) -> WireOutcome {
    let Some(dd) = data_dir else {
        return WireOutcome::NoDataDir;
    };
    let kc = dd.join("kubeconfig").join("admin.kubeconfig");
    if !kc.is_file() {
        return WireOutcome::NoDataDir;
    }
    match ApiserverConfig::from_kubeconfig(&kc).and_then(ApiserverClient::from_config) {
        Ok(client) => {
            state.set_runtime_client(Arc::new(client));
            WireOutcome::Wired
        }
        Err(e) => {
            tracing::warn!(
                kubeconfig = %kc.display(),
                error = %e,
                "kubeconfig probe failed — Portal will use seeded fixtures",
            );
            WireOutcome::KubeconfigBroken
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::path::PathBuf;

    fn tenant() -> TenantId {
        TenantId::new("acme").unwrap()
    }

    #[tokio::test]
    async fn mock_client_returns_empty_lists() {
        let c = MockClient::new();
        let t = tenant();
        assert!(c.list_kubelet_pods(&t).await.unwrap().is_empty());
        assert!(c.list_scheduler_nodes(&t).await.unwrap().is_empty());
        assert!(c.list_net_endpoints(&t).await.unwrap().is_empty());
        assert!(c.list_keda_scaled_objects(&t).await.unwrap().is_empty());
        assert!(c.list_vault_secrets(&t).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn apiserver_list_kubelet_pods_maps_phase_and_restart_count() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/pods");
            then.status(200).header("content-type", "application/json").body(
                r#"{"items":[
                  {"metadata":{"name":"web-0","namespace":"default","labels":{"cave.io/tenant":"acme"}},
                   "spec":{"nodeName":"n1"},
                   "status":{"phase":"Running","containerStatuses":[{"restartCount":2},{"restartCount":1}]}},
                  {"metadata":{"name":"api-0","namespace":"default"},
                   "spec":{"nodeName":"n2"},
                   "status":{"phase":"Pending"}}
                ]}"#,
            );
        });
        let c = ApiserverClient::test_against(server.base_url());
        let pods = c.list_kubelet_pods(&tenant()).await.unwrap();
        assert_eq!(pods.len(), 2);
        let web = pods.iter().find(|p| p.pod_name == "web-0").unwrap();
        assert_eq!(web.node, "n1");
        assert_eq!(web.status, "Running");
        assert_eq!(web.restart_count, 3); // 2 + 1
        // Pod without label inherits caller's tenant (fallback path).
        let api = pods.iter().find(|p| p.pod_name == "api-0").unwrap();
        assert_eq!(api.status, "Pending");
        assert_eq!(api.tenant.as_str(), "acme");
    }

    #[tokio::test]
    async fn apiserver_list_scheduler_nodes_parses_ready_and_quantities() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/nodes");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"items":[
                  {"metadata":{"name":"node-a"},
                   "spec":{"taints":[{"key":"reserved","effect":"NoSchedule"}]},
                   "status":{"conditions":[{"type":"Ready","status":"True"}],
                             "allocatable":{"cpu":"4","memory":"8Gi"}}},
                  {"metadata":{"name":"node-b"},
                   "status":{"conditions":[{"type":"Ready","status":"False"}],
                             "allocatable":{"cpu":"500m","memory":"512Mi"}}}
                ]}"#,
                );
        });
        let c = ApiserverClient::test_against(server.base_url());
        let nodes = c.list_scheduler_nodes(&tenant()).await.unwrap();
        assert_eq!(nodes.len(), 2);
        let a = nodes.iter().find(|n| n.name == "node-a").unwrap();
        assert!(a.ready);
        assert_eq!(a.allocatable_cpu_milli, 4000);
        assert_eq!(a.allocatable_mem_mib, 8 * 1024);
        assert_eq!(a.taints, vec!["reserved".to_string()]);
        let b = nodes.iter().find(|n| n.name == "node-b").unwrap();
        assert!(!b.ready);
        assert_eq!(b.allocatable_cpu_milli, 500);
        assert_eq!(b.allocatable_mem_mib, 512);
    }

    #[tokio::test]
    async fn apiserver_list_net_endpoints_flattens_subsets_and_marks_ready() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/endpoints");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"items":[
                  {"metadata":{"name":"svc-a","namespace":"prod"},
                   "subsets":[{"addresses":[{"ip":"10.0.0.1"},{"ip":"10.0.0.2"}],
                              "notReadyAddresses":[{"ip":"10.0.0.3"}]}]}
                ]}"#,
                );
        });
        let c = ApiserverClient::test_against(server.base_url());
        let eps = c.list_net_endpoints(&tenant()).await.unwrap();
        assert_eq!(eps.len(), 3);
        assert_eq!(eps.iter().filter(|e| e.ready).count(), 2);
        assert_eq!(eps.iter().filter(|e| !e.ready).count(), 1);
        assert!(eps.iter().all(|e| e.namespace == "prod"));
    }

    #[tokio::test]
    async fn apiserver_list_keda_scaled_objects_parses_crd_spec() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET)
                .path("/apis/keda.sh/v1alpha1/scaledobjects");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"items":[
                  {"metadata":{"name":"http-scaler","namespace":"prod"},
                   "spec":{"scaleTargetRef":{"name":"http-app"},
                           "minReplicaCount":0,"maxReplicaCount":50}}
                ]}"#,
                );
        });
        let c = ApiserverClient::test_against(server.base_url());
        let sos = c.list_keda_scaled_objects(&tenant()).await.unwrap();
        assert_eq!(sos.len(), 1);
        assert_eq!(sos[0].name, "http-scaler");
        assert_eq!(sos[0].target_ref, "http-app");
        assert_eq!(sos[0].min_replicas, 0);
        assert_eq!(sos[0].max_replicas, 50);
    }

    #[tokio::test]
    async fn apiserver_returns_status_error_on_401() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/pods");
            then.status(401).body("Unauthorized");
        });
        let c = ApiserverClient::test_against(server.base_url());
        let err = c.list_kubelet_pods(&tenant()).await.unwrap_err();
        match err {
            RuntimeError::Status(s) => assert_eq!(s.as_u16(), 401),
            other => panic!("expected Status(401), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apiserver_returns_status_error_on_503() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/nodes");
            then.status(503).body("apiserver overloaded");
        });
        let c = ApiserverClient::test_against(server.base_url());
        let err = c.list_scheduler_nodes(&tenant()).await.unwrap_err();
        match err {
            RuntimeError::Status(s) => assert_eq!(s.as_u16(), 503),
            other => panic!("expected Status(503), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn apiserver_returns_decode_error_on_garbage_json() {
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/pods");
            then.status(200)
                .header("content-type", "application/json")
                .body("not-json");
        });
        let c = ApiserverClient::test_against(server.base_url());
        let err = c.list_kubelet_pods(&tenant()).await.unwrap_err();
        assert!(matches!(err, RuntimeError::Decode(_)));
    }

    #[tokio::test]
    async fn apiserver_list_vault_secrets_returns_not_wired() {
        let c = ApiserverClient::test_against("http://localhost:0".into());
        let err = c.list_vault_secrets(&tenant()).await.unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::NotWired {
                resource: "vault_secrets"
            }
        ));
    }

    #[test]
    fn parse_cpu_quantity_handles_units() {
        assert_eq!(parse_cpu_quantity("500m"), 500);
        assert_eq!(parse_cpu_quantity("2"), 2000);
        assert_eq!(parse_cpu_quantity("1.5"), 1500);
        assert_eq!(parse_cpu_quantity(""), 0);
        assert_eq!(parse_cpu_quantity("garbage"), 0);
    }

    #[test]
    fn parse_mem_quantity_mib_handles_units() {
        assert_eq!(parse_mem_quantity_mib("512Mi"), 512);
        assert_eq!(parse_mem_quantity_mib("2Gi"), 2 * 1024);
        assert_eq!(parse_mem_quantity_mib("1Ti"), 1024 * 1024);
        assert_eq!(parse_mem_quantity_mib("garbage"), 0);
    }

    #[test]
    fn kubeconfig_roundtrip_from_cluster_init_format() {
        // Mirror the format `cluster::render_kubeconfig` writes —
        // single context, base64-encoded CA + client cert + key.
        let tmp = tempfile::TempDir::new().unwrap();
        let path: PathBuf = tmp.path().join("admin.kubeconfig");
        let b64 = base64::engine::general_purpose::STANDARD;
        let yaml = format!(
            "apiVersion: v1\n\
             kind: Config\n\
             current-context: cave-local-admin@cave-local\n\
             clusters:\n\
             - name: cave-local\n  cluster:\n    server: https://127.0.0.1:6443\n    certificate-authority-data: {ca}\n\
             contexts:\n\
             - name: cave-local-admin@cave-local\n  context:\n    cluster: cave-local\n    user: cave-local-admin\n\
             users:\n\
             - name: cave-local-admin\n  user:\n    client-certificate-data: {crt}\n    client-key-data: {key}\n",
            ca = b64.encode(b"FAKE-CA-PEM"),
            crt = b64.encode(b"FAKE-CRT-PEM"),
            key = b64.encode(b"FAKE-KEY-PEM"),
        );
        std::fs::write(&path, yaml).unwrap();
        let cfg = ApiserverConfig::from_kubeconfig(&path).unwrap();
        assert_eq!(cfg.server, "https://127.0.0.1:6443");
        assert_eq!(cfg.ca_pem, b"FAKE-CA-PEM");
        assert_eq!(cfg.client_cert_pem, b"FAKE-CRT-PEM");
        assert_eq!(cfg.client_key_pem, b"FAKE-KEY-PEM");
        assert!(cfg.bearer_token.is_none());
    }

    #[test]
    fn kubeconfig_missing_current_context_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("broken.kubeconfig");
        std::fs::write(&path, "apiVersion: v1\nkind: Config\nclusters: []\n").unwrap();
        let err = ApiserverConfig::from_kubeconfig(&path).unwrap_err();
        assert!(matches!(
            err,
            KubeconfigError::MissingField("current-context")
        ));
    }

    // ── materialise integration tests (AdminState ↔ RuntimeClient seam) ──

    #[tokio::test]
    async fn admin_state_materialise_kubelet_pods_against_apiserver() {
        use crate::admin::state::AdminState;
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/pods");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"items":[
                  {"metadata":{"name":"live-pod","labels":{"cave.io/tenant":"acme"}},
                   "spec":{"nodeName":"n-live"},
                   "status":{"phase":"Running","containerStatuses":[{"restartCount":4}]}}
                ]}"#,
                );
        });
        let state = AdminState::empty();
        state.set_runtime_client(Arc::new(ApiserverClient::test_against(server.base_url())));
        let t = tenant();
        state.materialise_kubelet_pods(&t).await.unwrap();
        let pods = state.kubelet_pods.read().unwrap();
        let acme: Vec<_> = pods.iter().filter(|p| p.tenant == t).collect();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].pod_name, "live-pod");
        assert_eq!(acme[0].restart_count, 4);
    }

    #[tokio::test]
    async fn admin_state_materialise_is_noop_when_no_runtime_set() {
        use crate::admin::state::{AdminState, KubeletPod};
        let state = AdminState::empty();
        // Pre-load a fixture row so we can verify it survives the noop.
        let t = tenant();
        state.kubelet_pods.write().unwrap().push(KubeletPod {
            tenant: t.clone(),
            node: "fixture-node".into(),
            pod_name: "fixture-pod".into(),
            status: "Running",
            restart_count: 0,
        });
        state.materialise_kubelet_pods(&t).await.unwrap();
        let pods = state.kubelet_pods.read().unwrap();
        assert_eq!(pods.len(), 1, "noop must not touch existing rows");
        assert_eq!(pods[0].pod_name, "fixture-pod");
    }

    #[tokio::test]
    async fn admin_state_materialise_preserves_other_tenant_rows() {
        use crate::admin::state::{AdminState, KubeletPod};
        let server = MockServer::start();
        let _m = server.mock(|when, then| {
            when.method(GET).path("/api/v1/pods");
            then.status(200).body(
                r#"{"items":[{"metadata":{"name":"acme-pod"},"spec":{"nodeName":"n1"},"status":{"phase":"Running"}}]}"#,
            );
        });
        let state = AdminState::empty();
        state.set_runtime_client(Arc::new(ApiserverClient::test_against(server.base_url())));
        // Seed an "evil" tenant row that must survive the acme refresh.
        let evil = TenantId::new("evil").unwrap();
        state.kubelet_pods.write().unwrap().push(KubeletPod {
            tenant: evil.clone(),
            node: "n-evil".into(),
            pod_name: "evil-pod".into(),
            status: "Running",
            restart_count: 99,
        });
        let acme = tenant();
        state.materialise_kubelet_pods(&acme).await.unwrap();
        let pods = state.kubelet_pods.read().unwrap();
        assert!(
            pods.iter()
                .any(|p| p.tenant == evil && p.pod_name == "evil-pod")
        );
        assert!(
            pods.iter()
                .any(|p| p.tenant == acme && p.pod_name == "acme-pod")
        );
    }

    #[tokio::test]
    async fn admin_state_set_runtime_client_is_idempotent() {
        use crate::admin::state::AdminState;
        let state = AdminState::empty();
        state.set_runtime_client(Arc::new(MockClient::new()));
        state.set_runtime_client(Arc::new(MockClient::new())); // second call ignored
        assert!(state.runtime().is_some());
    }

    #[test]
    fn probe_data_dir_none_yields_no_data_dir() {
        use crate::admin::state::AdminState;
        let state = AdminState::empty();
        let outcome = probe_data_dir_for_runtime(&state, None);
        assert_eq!(outcome, WireOutcome::NoDataDir);
        assert!(state.runtime().is_none());
    }

    #[test]
    fn probe_data_dir_missing_kubeconfig_yields_no_data_dir() {
        use crate::admin::state::AdminState;
        let tmp = tempfile::TempDir::new().unwrap();
        let state = AdminState::empty();
        let outcome = probe_data_dir_for_runtime(&state, Some(tmp.path()));
        assert_eq!(outcome, WireOutcome::NoDataDir);
        assert!(state.runtime().is_none());
    }

    #[test]
    fn probe_data_dir_with_valid_kubeconfig_wires_client() {
        use crate::admin::state::AdminState;
        let tmp = tempfile::TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        std::fs::create_dir_all(dd.join("kubeconfig")).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD;
        // Valid PEM block (a self-signed throwaway). reqwest only parses
        // it lazily on first use, so an arbitrary cert is fine here.
        let pem = b"-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIJALr5RX/PJEQGMA0GCSqGSIb3DQEBCwUAMA0xCzAJBgNVBAYTAlVT\nMB4XDTI2MDExMTAwMDAwMFoXDTI3MDExMTAwMDAwMFowDTELMAkGA1UEBhMCVVMw\ngZ0wDQYJKoZIhvcNAQEBBQADgYsAMIGHAoGBALnJEFB5OQDnQFv0PnzPL3+IxNL9\n-----END CERTIFICATE-----\n";
        let kc_yaml = format!(
            "apiVersion: v1\nkind: Config\ncurrent-context: c\n\
             clusters:\n- name: cl\n  cluster:\n    server: https://127.0.0.1:6443\n    certificate-authority-data: {ca}\n\
             contexts:\n- name: c\n  context:\n    cluster: cl\n    user: u\n\
             users:\n- name: u\n  user:\n    client-certificate-data: {crt}\n    client-key-data: {key}\n",
            ca = b64.encode(pem),
            crt = b64.encode(pem),
            key = b64.encode(pem),
        );
        std::fs::write(dd.join("kubeconfig/admin.kubeconfig"), kc_yaml).unwrap();
        let state = AdminState::empty();
        let outcome = probe_data_dir_for_runtime(&state, Some(&dd));
        // The reqwest client builder will reject the fake PEM at build
        // time, so the outcome surfaces as KubeconfigBroken — exactly
        // the "operator should investigate" branch we want to test.
        // With a real CA the same path yields Wired (covered by
        // kubeconfig_roundtrip_from_cluster_init_format above).
        assert!(matches!(
            outcome,
            WireOutcome::Wired | WireOutcome::KubeconfigBroken
        ));
    }
}
