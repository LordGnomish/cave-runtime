// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Trusted cluster federation model.
//!
//! Teleport supports cross-cluster access via a trust relationship: a root
//! cluster vouches for users from a leaf cluster (or vice versa). This module
//! tracks the trust relationships, the root CA fingerprints of peer clusters,
//! and which roles can be mapped across the trust boundary.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// Direction of the trust relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDirection {
    /// This cluster trusts the peer's users (the peer is a leaf).
    Inbound,
    /// Users of this cluster may access the peer (the peer is a root).
    Outbound,
    /// Both directions are permitted.
    Bidirectional,
}

/// Lifecycle state of the trust relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustState {
    /// Trust relationship created but not yet activated.
    Pending,
    /// Trust is established and access is permitted.
    Active,
    /// Trust has been administratively suspended.
    Inactive,
}

/// Parameters for registering a trusted cluster.
#[derive(Debug, Clone)]
pub struct TrustedCluster {
    /// Canonical name of the remote cluster.
    pub name: String,
    /// PEM-encoded root CA certificate of the peer cluster (used to verify
    /// its user and host certificates).
    pub root_ca_pem: String,
    /// Which direction(s) of access this trust relationship covers.
    pub direction: TrustDirection,
    /// Roles in the peer cluster that should be mapped to local roles.
    pub roles_to_map: Vec<String>,
    /// Arbitrary metadata (region, env, owner, etc.).
    pub metadata: HashMap<String, String>,
}

/// A stored trusted-cluster record.
#[derive(Debug, Clone)]
pub struct TrustedClusterRecord {
    pub id: Uuid,
    pub name: String,
    pub root_ca_pem: String,
    pub direction: TrustDirection,
    pub roles_to_map: Vec<String>,
    pub metadata: HashMap<String, String>,
    pub state: TrustState,
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the trust store.
#[derive(Debug, PartialEq, Clone)]
pub enum TrustError {
    NotFound,
    DuplicateName,
    AlreadyInState,
}

impl std::fmt::Display for TrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "trusted cluster not found"),
            Self::DuplicateName => write!(f, "a trusted cluster with that name already exists"),
            Self::AlreadyInState => write!(f, "cluster is already in that state"),
        }
    }
}

impl std::error::Error for TrustError {}

// ── Trust store ───────────────────────────────────────────────────────────────

/// Thread-safe store for trusted-cluster relationships.
pub struct TrustStore {
    local_name: String,
    clusters: Arc<RwLock<HashMap<Uuid, TrustedClusterRecord>>>,
}

impl TrustStore {
    /// Create a new trust store for the named local cluster.
    pub fn new(local_cluster_name: &str) -> Self {
        Self {
            local_name: local_cluster_name.to_string(),
            clusters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Return this cluster's own name.
    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    /// Register a new trusted cluster. Returns its ID.
    pub fn register(&self, cluster: TrustedCluster) -> Result<Uuid, TrustError> {
        let mut map = self.clusters.write().unwrap();
        // Prevent duplicate names.
        if map.values().any(|c| c.name == cluster.name) {
            return Err(TrustError::DuplicateName);
        }
        let id = Uuid::new_v4();
        map.insert(
            id,
            TrustedClusterRecord {
                id,
                name: cluster.name,
                root_ca_pem: cluster.root_ca_pem,
                direction: cluster.direction,
                roles_to_map: cluster.roles_to_map,
                metadata: cluster.metadata,
                state: TrustState::Pending,
            },
        );
        Ok(id)
    }

    /// Activate a pending or inactive trust relationship.
    pub fn activate(&self, id: &Uuid) -> Result<(), TrustError> {
        let mut map = self.clusters.write().unwrap();
        let cluster = map.get_mut(id).ok_or(TrustError::NotFound)?;
        if cluster.state == TrustState::Active {
            return Err(TrustError::AlreadyInState);
        }
        cluster.state = TrustState::Active;
        Ok(())
    }

    /// Suspend an active trust relationship without removing it.
    pub fn deactivate(&self, id: &Uuid) -> Result<(), TrustError> {
        let mut map = self.clusters.write().unwrap();
        let cluster = map.get_mut(id).ok_or(TrustError::NotFound)?;
        if cluster.state == TrustState::Inactive {
            return Err(TrustError::AlreadyInState);
        }
        cluster.state = TrustState::Inactive;
        Ok(())
    }

    /// Remove a trusted cluster record entirely.
    pub fn remove(&self, id: &Uuid) -> Result<(), TrustError> {
        let mut map = self.clusters.write().unwrap();
        if map.remove(id).is_some() {
            Ok(())
        } else {
            Err(TrustError::NotFound)
        }
    }

    /// Look up a cluster by its ID.
    pub fn get(&self, id: &Uuid) -> Option<TrustedClusterRecord> {
        self.clusters.read().unwrap().get(id).cloned()
    }

    /// Look up a cluster by its canonical name.
    pub fn get_by_name(&self, name: &str) -> Option<TrustedClusterRecord> {
        self.clusters
            .read()
            .unwrap()
            .values()
            .find(|c| c.name == name)
            .cloned()
    }

    /// Return all clusters in the Active state.
    pub fn list_active(&self) -> Vec<TrustedClusterRecord> {
        self.clusters
            .read()
            .unwrap()
            .values()
            .filter(|c| c.state == TrustState::Active)
            .cloned()
            .collect()
    }

    /// Return all registered clusters regardless of state.
    pub fn list_all(&self) -> Vec<TrustedClusterRecord> {
        self.clusters.read().unwrap().values().cloned().collect()
    }

    /// Return clusters that match the given trust direction.
    pub fn list_by_direction(&self, direction: &TrustDirection) -> Vec<TrustedClusterRecord> {
        self.clusters
            .read()
            .unwrap()
            .values()
            .filter(|c| {
                &c.direction == direction
                    || c.direction == TrustDirection::Bidirectional
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_name_rejected() {
        let store = TrustStore::new("local");
        let cluster = TrustedCluster {
            name: "dup".to_string(),
            root_ca_pem: "ca".to_string(),
            direction: TrustDirection::Outbound,
            roles_to_map: vec![],
            metadata: HashMap::new(),
        };
        store.register(cluster.clone()).unwrap();
        assert_eq!(store.register(cluster).unwrap_err(), TrustError::DuplicateName);
    }

    #[test]
    fn activate_already_active_errors() {
        let store = TrustStore::new("local");
        let id = store
            .register(TrustedCluster {
                name: "peer".to_string(),
                root_ca_pem: "ca".to_string(),
                direction: TrustDirection::Inbound,
                roles_to_map: vec![],
                metadata: HashMap::new(),
            })
            .unwrap();
        store.activate(&id).unwrap();
        assert_eq!(store.activate(&id).unwrap_err(), TrustError::AlreadyInState);
    }

    #[test]
    fn list_by_direction_includes_bidirectional() {
        let store = TrustStore::new("local");
        store
            .register(TrustedCluster {
                name: "both".to_string(),
                root_ca_pem: "ca".to_string(),
                direction: TrustDirection::Bidirectional,
                roles_to_map: vec![],
                metadata: HashMap::new(),
            })
            .unwrap();
        let outbound = store.list_by_direction(&TrustDirection::Outbound);
        // Bidirectional clusters appear in both directions.
        assert_eq!(outbound.len(), 1);
    }
}
