//! Multi-cluster service mesh federation.
//!
//! Implements:
//!   • RemoteCluster registry (track peer clusters with their API endpoints)
//!   • CrossClusterService — services exported from / imported into clusters
//!   • TrustDomainFederation — cross-cluster SPIFFE trust chain config
//!   • MultiClusterRegistry — cross-cluster service discovery

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::{debug, info, warn};

// ─────────────────────────────────────────────────────────────
// Remote cluster descriptor
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteCluster {
    pub name: String,
    /// Network name (for network-aware load balancing).
    pub network: String,
    /// API server endpoint of the remote cluster.
    pub api_server_url: Option<String>,
    /// Control plane endpoint (istiod / cave-mesh remote).
    pub control_plane_url: Option<String>,
    /// Trust domain of the remote cluster.
    pub trust_domain: String,
    pub status: RemoteClusterStatus,
    pub registered_at: DateTime<Utc>,
    pub last_sync_at: Option<DateTime<Utc>>,
}

impl RemoteCluster {
    pub fn new(name: impl Into<String>, network: impl Into<String>, trust_domain: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            network: network.into(),
            api_server_url: None,
            control_plane_url: None,
            trust_domain: trust_domain.into(),
            status: RemoteClusterStatus::Unknown,
            registered_at: Utc::now(),
            last_sync_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RemoteClusterStatus {
    Unknown,
    Connected,
    Disconnected,
    Degraded,
}

// ─────────────────────────────────────────────────────────────
// Cross-cluster service
// ─────────────────────────────────────────────────────────────

/// A service exported from one cluster and available for import in others.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClusterService {
    pub name: String,
    pub namespace: String,
    pub source_cluster: String,
    pub host_fqdn: String,
    pub ports: Vec<CrossClusterPort>,
    pub endpoints: Vec<CrossClusterEndpoint>,
    pub export_to: Vec<String>,
    pub registered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClusterPort {
    pub port: u16,
    pub protocol: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossClusterEndpoint {
    pub address: String,
    pub port: u16,
    pub locality: Option<String>,
    pub network: String,
    pub weight: u32,
}

// ─────────────────────────────────────────────────────────────
// Trust domain federation
// ─────────────────────────────────────────────────────────────

/// Configuration for federating trust between clusters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustDomainFederation {
    /// Local trust domain name.
    pub local_trust_domain: String,
    /// Remote trust domain name.
    pub remote_trust_domain: String,
    /// PEM-encoded root CA bundle of the remote trust domain.
    pub remote_ca_bundle_pem: String,
    /// Whether to perform automatic bundle rotation via SPIFFE Bundle Endpoint.
    pub auto_rotate: bool,
    pub bundle_endpoint_url: Option<String>,
    pub established_at: DateTime<Utc>,
}

impl TrustDomainFederation {
    pub fn new(
        local: impl Into<String>,
        remote: impl Into<String>,
        ca_bundle_pem: impl Into<String>,
    ) -> Self {
        Self {
            local_trust_domain: local.into(),
            remote_trust_domain: remote.into(),
            remote_ca_bundle_pem: ca_bundle_pem.into(),
            auto_rotate: false,
            bundle_endpoint_url: None,
            established_at: Utc::now(),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// MultiClusterRegistry
// ─────────────────────────────────────────────────────────────

/// Central registry for multi-cluster state: remote clusters, cross-cluster
/// services, and trust federation.
#[derive(Clone)]
pub struct MultiClusterRegistry {
    clusters: Arc<RwLock<HashMap<String, RemoteCluster>>>,
    services: Arc<RwLock<HashMap<String, Vec<CrossClusterService>>>>,
    federations: Arc<RwLock<HashMap<String, TrustDomainFederation>>>,
    /// Local cluster name / identifier.
    local_cluster: Arc<RwLock<String>>,
}

impl Default for MultiClusterRegistry {
    fn default() -> Self {
        Self::new("local")
    }
}

impl MultiClusterRegistry {
    pub fn new(local_cluster: impl Into<String>) -> Self {
        Self {
            clusters: Arc::new(RwLock::new(HashMap::new())),
            services: Arc::new(RwLock::new(HashMap::new())),
            federations: Arc::new(RwLock::new(HashMap::new())),
            local_cluster: Arc::new(RwLock::new(local_cluster.into())),
        }
    }

    pub fn local_cluster(&self) -> String {
        self.local_cluster.read().unwrap().clone()
    }

    // ─── Remote cluster CRUD ─────────────────────────────────

    pub fn register_cluster(&self, cluster: RemoteCluster) {
        info!(cluster = %cluster.name, network = %cluster.network, "Remote cluster registered");
        self.clusters.write().unwrap().insert(cluster.name.clone(), cluster);
    }

    pub fn update_cluster_status(&self, cluster_name: &str, status: RemoteClusterStatus) {
        let mut map = self.clusters.write().unwrap();
        if let Some(c) = map.get_mut(cluster_name) {
            c.status = status;
            c.last_sync_at = Some(Utc::now());
        }
    }

    pub fn remove_cluster(&self, cluster_name: &str) {
        self.clusters.write().unwrap().remove(cluster_name);
        self.services.write().unwrap().remove(cluster_name);
    }

    pub fn list_clusters(&self) -> Vec<RemoteCluster> {
        self.clusters.read().unwrap().values().cloned().collect()
    }

    pub fn get_cluster(&self, name: &str) -> Option<RemoteCluster> {
        self.clusters.read().unwrap().get(name).cloned()
    }

    pub fn connected_clusters(&self) -> Vec<RemoteCluster> {
        self.clusters
            .read()
            .unwrap()
            .values()
            .filter(|c| c.status == RemoteClusterStatus::Connected)
            .cloned()
            .collect()
    }

    // ─── Cross-cluster service CRUD ──────────────────────────

    pub fn export_service(&self, svc: CrossClusterService) {
        debug!(
            service = %svc.name,
            namespace = %svc.namespace,
            cluster = %svc.source_cluster,
            "Cross-cluster service exported"
        );
        let cluster = svc.source_cluster.clone();
        let mut map = self.services.write().unwrap();
        let entry = map.entry(cluster).or_default();
        // Upsert by name/namespace
        if let Some(existing) = entry.iter_mut().find(|s| s.name == svc.name && s.namespace == svc.namespace) {
            *existing = svc;
        } else {
            entry.push(svc);
        }
    }

    pub fn remove_exported_service(&self, cluster: &str, namespace: &str, name: &str) {
        let mut map = self.services.write().unwrap();
        if let Some(svcs) = map.get_mut(cluster) {
            svcs.retain(|s| !(s.name == name && s.namespace == namespace));
        }
    }

    /// Get all services visible to this cluster (exported by all clusters,
    /// filtered by export_to rules).
    pub fn visible_services(&self) -> Vec<CrossClusterService> {
        let local = self.local_cluster();
        let map = self.services.read().unwrap();
        map.values()
            .flatten()
            .filter(|s| {
                s.export_to.is_empty()
                    || s.export_to.iter().any(|t| t == "*" || t == &local)
            })
            .cloned()
            .collect()
    }

    pub fn services_from_cluster(&self, cluster: &str) -> Vec<CrossClusterService> {
        self.services.read().unwrap().get(cluster).cloned().unwrap_or_default()
    }

    // ─── Trust federation CRUD ───────────────────────────────

    pub fn federate(&self, fed: TrustDomainFederation) {
        info!(
            local = %fed.local_trust_domain,
            remote = %fed.remote_trust_domain,
            "Trust domain federation established"
        );
        let key = format!("{}->{}", fed.local_trust_domain, fed.remote_trust_domain);
        self.federations.write().unwrap().insert(key, fed);
    }

    pub fn remove_federation(&self, local: &str, remote: &str) {
        let key = format!("{local}->{remote}");
        self.federations.write().unwrap().remove(&key);
    }

    pub fn list_federations(&self) -> Vec<TrustDomainFederation> {
        self.federations.read().unwrap().values().cloned().collect()
    }

    pub fn get_federation(&self, local: &str, remote: &str) -> Option<TrustDomainFederation> {
        let key = format!("{local}->{remote}");
        self.federations.read().unwrap().get(&key).cloned()
    }

    /// Check if a remote trust domain is federated with the local one.
    pub fn is_federated(&self, remote_trust_domain: &str) -> bool {
        let local = self.local_cluster();
        let key = format!("{local}->{remote_trust_domain}");
        self.federations.read().unwrap().contains_key(&key)
    }

    // ─── Status snapshot ─────────────────────────────────────

    pub fn federation_snapshot(&self) -> FederationSnapshot {
        let clusters = self.list_clusters();
        let services = self.visible_services();
        let federations = self.list_federations();
        FederationSnapshot {
            local_cluster: self.local_cluster(),
            total_remote_clusters: clusters.len(),
            connected_clusters: clusters
                .iter()
                .filter(|c| c.status == RemoteClusterStatus::Connected)
                .count(),
            total_cross_cluster_services: services.len(),
            total_federations: federations.len(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationSnapshot {
    pub local_cluster: String,
    pub total_remote_clusters: usize,
    pub connected_clusters: usize,
    pub total_cross_cluster_services: usize,
    pub total_federations: usize,
}
