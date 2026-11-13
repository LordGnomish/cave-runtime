// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Node/resource inventory for privileged access management.
//!
//! Tracks the set of infrastructure nodes (SSH servers, databases, Kubernetes
//! clusters, applications) that are enrolled in the PAM plane. Modelled after
//! Teleport's node registration and heartbeat model.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// What kind of resource is being tracked.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Server,
    Database,
    Kubernetes,
    Application,
    WindowsDesktop,
}

/// Current health status of a node (updated via heartbeat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeHealth {
    /// No heartbeat received yet.
    Unknown,
    /// Node is reachable and responding normally.
    Healthy,
    /// Node failed its last health check.
    Unhealthy,
    /// Node has been administratively disabled.
    Disabled,
}

/// Parameters for enrolling a new node.
#[derive(Debug, Clone)]
pub struct EnrollNode {
    /// DNS name or short name of the node.
    pub hostname: String,
    /// What type of resource it is.
    pub kind: NodeKind,
    /// Key/value labels for policy matching (env, region, team, etc.).
    pub labels: HashMap<String, String>,
    /// Network address including port (e.g., "10.0.0.1:22").
    pub addr: String,
}

/// A stored node record.
#[derive(Debug, Clone)]
pub struct NodeRecord {
    /// Unique stable identifier assigned at enroll time.
    pub id: Uuid,
    pub hostname: String,
    pub kind: NodeKind,
    pub labels: HashMap<String, String>,
    pub addr: String,
    pub health: NodeHealth,
    pub enrolled_at: DateTime<Utc>,
    pub last_heartbeat: Option<DateTime<Utc>>,
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the node inventory.
#[derive(Debug, PartialEq, Clone)]
pub enum InventoryError {
    NodeNotFound,
    DuplicateHostname,
}

impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeNotFound => write!(f, "node not found"),
            Self::DuplicateHostname => write!(f, "a node with that hostname is already enrolled"),
        }
    }
}

impl std::error::Error for InventoryError {}

// ── Inventory ─────────────────────────────────────────────────────────────────

/// Thread-safe in-memory node inventory.
#[derive(Debug, Default)]
pub struct NodeInventory {
    nodes: Arc<RwLock<HashMap<Uuid, NodeRecord>>>,
}

impl NodeInventory {
    /// Create an empty inventory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enroll a new node. Returns its assigned UUID.
    pub fn enroll(&self, req: EnrollNode) -> Result<Uuid, InventoryError> {
        let mut nodes = self.nodes.write().unwrap();
        // Prevent duplicate hostnames of the same kind.
        let duplicate = nodes
            .values()
            .any(|n| n.hostname == req.hostname && n.kind == req.kind);
        if duplicate {
            return Err(InventoryError::DuplicateHostname);
        }
        let id = Uuid::new_v4();
        nodes.insert(
            id,
            NodeRecord {
                id,
                hostname: req.hostname,
                kind: req.kind,
                labels: req.labels,
                addr: req.addr,
                health: NodeHealth::Unknown,
                enrolled_at: Utc::now(),
                last_heartbeat: None,
            },
        );
        Ok(id)
    }

    /// Look up a node by its UUID.
    pub fn get(&self, id: &Uuid) -> Option<NodeRecord> {
        self.nodes.read().unwrap().get(id).cloned()
    }

    /// Remove a node from the inventory.
    pub fn deregister(&self, id: &Uuid) -> Result<(), InventoryError> {
        let mut nodes = self.nodes.write().unwrap();
        if nodes.remove(id).is_some() {
            Ok(())
        } else {
            Err(InventoryError::NodeNotFound)
        }
    }

    /// Update the health status of a node (called on heartbeat or probe result).
    pub fn update_health(&self, id: &Uuid, health: NodeHealth) -> Result<(), InventoryError> {
        let mut nodes = self.nodes.write().unwrap();
        let node = nodes.get_mut(id).ok_or(InventoryError::NodeNotFound)?;
        node.health = health;
        node.last_heartbeat = Some(Utc::now());
        Ok(())
    }

    /// List all nodes of a given kind.
    pub fn list_by_kind(&self, kind: &NodeKind) -> Vec<NodeRecord> {
        self.nodes
            .read()
            .unwrap()
            .values()
            .filter(|n| &n.kind == kind)
            .cloned()
            .collect()
    }

    /// List nodes whose `NodeHealth` is `Unhealthy`.
    pub fn list_unhealthy(&self) -> Vec<NodeRecord> {
        self.nodes
            .read()
            .unwrap()
            .values()
            .filter(|n| n.health == NodeHealth::Unhealthy)
            .cloned()
            .collect()
    }

    /// List all nodes.
    pub fn list_all(&self) -> Vec<NodeRecord> {
        self.nodes.read().unwrap().values().cloned().collect()
    }

    /// Filter nodes by a single label key/value.
    pub fn list_by_label(&self, key: &str, value: &str) -> Vec<NodeRecord> {
        self.nodes
            .read()
            .unwrap()
            .values()
            .filter(|n| n.labels.get(key).map(|v| v.as_str()) == Some(value))
            .cloned()
            .collect()
    }

    /// Return count of all enrolled nodes.
    pub fn count(&self) -> usize {
        self.nodes.read().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_hostname_same_kind_rejected() {
        let inv = NodeInventory::new();
        let req = EnrollNode {
            hostname: "dup".to_string(),
            kind: NodeKind::Server,
            labels: HashMap::new(),
            addr: "1.2.3.4:22".to_string(),
        };
        inv.enroll(req.clone()).unwrap();
        let err = inv.enroll(req).unwrap_err();
        assert_eq!(err, InventoryError::DuplicateHostname);
    }

    #[test]
    fn same_hostname_different_kind_allowed() {
        let inv = NodeInventory::new();
        let base = EnrollNode {
            hostname: "shared".to_string(),
            kind: NodeKind::Server,
            labels: HashMap::new(),
            addr: "1.2.3.4:22".to_string(),
        };
        inv.enroll(base).unwrap();
        let db_node = EnrollNode {
            hostname: "shared".to_string(),
            kind: NodeKind::Database,
            labels: HashMap::new(),
            addr: "1.2.3.4:5432".to_string(),
        };
        assert!(inv.enroll(db_node).is_ok());
    }

    #[test]
    fn deregister_nonexistent_errors() {
        let inv = NodeInventory::new();
        assert_eq!(
            inv.deregister(&Uuid::new_v4()).unwrap_err(),
            InventoryError::NodeNotFound
        );
    }
}
