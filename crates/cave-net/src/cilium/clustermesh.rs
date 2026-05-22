// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ClusterMesh — multi-cluster identity exchange and service announce.
//!
//! Mirrors `pkg/clustermesh/clustermesh.go` (the in-process registry that
//! holds remote-cluster identities and announced services). Each cluster
//! pushes its allocated [`identity::LocalIdentityCache`] entries and its
//! exported service set; this module tracks them and answers cross-cluster
//! identity-translation and service-discovery queries.
//!
//! Multi-tenant invariants:
//!
//! * Each `ClusterMesh` instance is owned by one tenant.
//! * Joining clusters must declare the same tenant; cross-tenant joins fail.
//! * Identity collisions across clusters get a *global* identity assigned by
//!   the mesh, recorded per (cluster, local-id) → global-id.

use crate::cilium::identity::{LabelSet, MIN_LOCAL_IDENTITY};
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MeshError {
    #[error("cluster {0} is already registered")]
    ClusterAlreadyExists(String),
    #[error("cluster {0} is not registered")]
    ClusterNotFound(String),
    #[error("tenant {tenant} cannot join a mesh owned by another tenant")]
    TenantMismatch { tenant: TenantId },
    #[error("identity {local} on cluster {cluster} not found")]
    UnknownIdentity { cluster: String, local: u32 },
    #[error("service {service} not announced by any cluster")]
    UnknownService { service: String },
}

/// One participating cluster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCluster {
    pub name: String,
    pub tenant: TenantId,
    /// `(local_id, labels)` pairs the cluster has announced.
    pub identities: Vec<(u32, LabelSet)>,
    /// `(service_name, namespace, vip)` triples.
    pub services: Vec<(String, String, String)>,
}

/// Service location in the mesh — which cluster, and which VIP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEndpoint {
    pub cluster: String,
    pub namespace: String,
    pub vip: String,
}

/// Mesh state. Mirrors the in-memory registry of upstream's
/// `clustermesh.ClusterMesh`.
#[derive(Debug)]
pub struct ClusterMesh {
    pub tenant: TenantId,
    clusters: HashMap<String, RemoteCluster>,
    /// Map (cluster, local_id) → global mesh-wide id.
    global_ids: HashMap<(String, u32), u32>,
    /// Reverse map: global_id → (cluster, local_id).
    global_owners: HashMap<u32, (String, u32)>,
    next_global: u32,
}

/// Lowest global identity issued by the mesh — sits above the local pool.
pub const MIN_GLOBAL_IDENTITY: u32 = 1 << 24;

impl ClusterMesh {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            clusters: HashMap::new(),
            global_ids: HashMap::new(),
            global_owners: HashMap::new(),
            next_global: MIN_GLOBAL_IDENTITY,
        }
    }

    /// Join a cluster. Refuses cross-tenant joins and double-registration.
    pub fn join(&mut self, cluster: RemoteCluster) -> Result<(), MeshError> {
        if cluster.tenant != self.tenant {
            return Err(MeshError::TenantMismatch {
                tenant: cluster.tenant,
            });
        }
        if self.clusters.contains_key(&cluster.name) {
            return Err(MeshError::ClusterAlreadyExists(cluster.name));
        }
        // Allocate global IDs for every announced identity.
        for (local, _labels) in &cluster.identities {
            if *local < MIN_LOCAL_IDENTITY {
                continue; // reserved IDs share a global namespace already.
            }
            let key = (cluster.name.clone(), *local);
            if !self.global_ids.contains_key(&key) {
                let g = self.next_global;
                self.next_global += 1;
                self.global_ids.insert(key.clone(), g);
                self.global_owners.insert(g, key);
            }
        }
        self.clusters.insert(cluster.name.clone(), cluster);
        Ok(())
    }

    /// Leave: drops every identity and service the cluster announced.
    pub fn leave(&mut self, cluster: &str) -> Result<(), MeshError> {
        let removed = self
            .clusters
            .remove(cluster)
            .ok_or_else(|| MeshError::ClusterNotFound(cluster.to_string()))?;
        for (local, _) in &removed.identities {
            let key = (removed.name.clone(), *local);
            if let Some(g) = self.global_ids.remove(&key) {
                self.global_owners.remove(&g);
            }
        }
        Ok(())
    }

    /// Translate a local cluster identity into the mesh-global one.
    pub fn translate(&self, cluster: &str, local: u32) -> Result<u32, MeshError> {
        if local < MIN_LOCAL_IDENTITY {
            return Ok(local); // reserved IDs are already global.
        }
        self.global_ids
            .get(&(cluster.to_string(), local))
            .copied()
            .ok_or_else(|| MeshError::UnknownIdentity {
                cluster: cluster.into(),
                local,
            })
    }

    /// Reverse-lookup: which cluster owns this global id?
    pub fn owner_of(&self, global: u32) -> Option<(&str, u32)> {
        self.global_owners
            .get(&global)
            .map(|(c, l)| (c.as_str(), *l))
    }

    /// Return every endpoint announcing the named service across the mesh.
    pub fn lookup_service(&self, name: &str) -> Result<Vec<ServiceEndpoint>, MeshError> {
        let mut out = Vec::new();
        for c in self.clusters.values() {
            for (svc, ns, vip) in &c.services {
                if svc == name {
                    out.push(ServiceEndpoint {
                        cluster: c.name.clone(),
                        namespace: ns.clone(),
                        vip: vip.clone(),
                    });
                }
            }
        }
        if out.is_empty() {
            Err(MeshError::UnknownService {
                service: name.into(),
            })
        } else {
            // Stable ordering: by cluster name.
            out.sort_by(|a, b| a.cluster.cmp(&b.cluster));
            Ok(out)
        }
    }

    pub fn cluster_count(&self) -> usize {
        self.clusters.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/clustermesh/clustermesh.go", "ClusterMesh");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn cluster(name: &str, tenant: &str, ids: &[(u32, &[(&str, &str)])]) -> RemoteCluster {
        RemoteCluster {
            name: name.into(),
            tenant: TenantId::new(tenant).expect("test fixture"),
            identities: ids
                .iter()
                .map(|(id, labels)| {
                    (
                        *id,
                        LabelSet::from_iter(labels.iter().map(|(k, v)| (*k, *v))),
                    )
                })
                .collect(),
            services: vec![],
        }
    }

    #[test]
    fn join_assigns_global_ids_above_local_pool() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/clustermesh.go",
            "ClusterMesh.AddCluster",
            "acme"
        );
        let mut mesh = ClusterMesh::new(tenant);
        mesh.join(cluster("us-east", "acme", &[(256, &[("app", "web")])]))
            .unwrap();
        let g = mesh.translate("us-east", 256).unwrap();
        assert!(g >= MIN_GLOBAL_IDENTITY);
        assert_eq!(mesh.cluster_count(), 1);
    }

    #[test]
    fn distinct_clusters_get_distinct_global_ids_for_same_local() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/clustermesh.go",
            "ClusterMesh.AllocateGlobal",
            "acme"
        );
        let mut mesh = ClusterMesh::new(tenant);
        mesh.join(cluster("us-east", "acme", &[(256, &[("app", "web")])]))
            .unwrap();
        mesh.join(cluster("eu-west", "acme", &[(256, &[("app", "web")])]))
            .unwrap();
        assert_ne!(
            mesh.translate("us-east", 256).unwrap(),
            mesh.translate("eu-west", 256).unwrap()
        );
    }

    #[test]
    fn cross_tenant_join_is_refused() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/clustermesh.go",
            "ClusterMesh.tenantCheck",
            "acme"
        );
        let mut mesh = ClusterMesh::new(tenant);
        let err = mesh
            .join(cluster("evil-cluster", "evil", &[(256, &[("a", "b")])]))
            .unwrap_err();
        assert!(matches!(err, MeshError::TenantMismatch { .. }));
    }

    #[test]
    fn double_join_returns_already_exists() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/clustermesh.go",
            "ClusterMesh.AddCluster",
            "acme"
        );
        let mut mesh = ClusterMesh::new(tenant);
        mesh.join(cluster("us-east", "acme", &[(256, &[("a", "b")])]))
            .unwrap();
        let err = mesh
            .join(cluster("us-east", "acme", &[(256, &[("a", "b")])]))
            .unwrap_err();
        assert!(matches!(err, MeshError::ClusterAlreadyExists(_)));
    }

    #[test]
    fn leave_removes_global_ids() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/clustermesh.go",
            "ClusterMesh.RemoveCluster",
            "acme"
        );
        let mut mesh = ClusterMesh::new(tenant);
        mesh.join(cluster("us-east", "acme", &[(256, &[("a", "b")])]))
            .unwrap();
        let g = mesh.translate("us-east", 256).unwrap();
        mesh.leave("us-east").unwrap();
        assert!(mesh.translate("us-east", 256).is_err());
        assert!(mesh.owner_of(g).is_none());
    }

    #[test]
    fn reserved_ids_translate_to_themselves() {
        let (_cite, tenant) =
            cilium_test_ctx!("pkg/identity/numericidentity.go", "GetReservedID", "acme");
        let mesh = ClusterMesh::new(tenant);
        // No cluster joined; reserved IDs are mesh-global by definition.
        assert_eq!(mesh.translate("anywhere", 1).unwrap(), 1);
        assert_eq!(mesh.translate("anywhere", 2).unwrap(), 2);
    }

    #[test]
    fn announced_services_are_discoverable_across_clusters() {
        let (_cite, tenant) =
            cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "ServiceMerger", "acme");
        let mut mesh = ClusterMesh::new(tenant);
        let mut us = cluster("us-east", "acme", &[(256, &[("a", "b")])]);
        us.services
            .push(("web".into(), "default".into(), "10.0.0.1".into()));
        let mut eu = cluster("eu-west", "acme", &[(256, &[("a", "b")])]);
        eu.services
            .push(("web".into(), "default".into(), "10.1.0.1".into()));
        mesh.join(us).unwrap();
        mesh.join(eu).unwrap();
        let endpoints = mesh.lookup_service("web").unwrap();
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].cluster, "eu-west");
        assert_eq!(endpoints[1].cluster, "us-east");
    }

    #[test]
    fn unknown_service_returns_error() {
        let (_cite, tenant) =
            cilium_test_ctx!("pkg/clustermesh/clustermesh.go", "ServiceLookup", "acme");
        let mesh = ClusterMesh::new(tenant);
        assert!(matches!(
            mesh.lookup_service("ghost").unwrap_err(),
            MeshError::UnknownService { .. }
        ));
    }

    #[test]
    fn owner_of_round_trips_with_translate() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/clustermesh/clustermesh.go",
            "ClusterMesh.OwnerOf",
            "acme"
        );
        let mut mesh = ClusterMesh::new(tenant);
        mesh.join(cluster("us-east", "acme", &[(256, &[("a", "b")])]))
            .unwrap();
        let g = mesh.translate("us-east", 256).unwrap();
        let (cluster, local) = mesh.owner_of(g).unwrap();
        assert_eq!(cluster, "us-east");
        assert_eq!(local, 256);
    }
}
