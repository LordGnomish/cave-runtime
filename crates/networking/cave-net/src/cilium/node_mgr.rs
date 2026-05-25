// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodeManager — multi-node identity exchange.
//!
//! Mirrors `pkg/node/types/node.go::Node` and the in-memory NodeManager
//! that owns the per-node identity table. The agent receives node-add /
//! node-update / node-delete events from the kvstore (or the local
//! CiliumNode CRD) and exposes the resulting registry to the policy
//! engine and the BPF tunnel-IP map writer.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Node source — where the node fact came from. Mirrors `node.Source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeSource {
    Local,
    Kvstore,
    Kubernetes,
    Custom,
    Restored,
    Unspec,
}

/// One node entry. Mirrors the `pkg/node/types.Node` struct surface
/// (subset of fields the agent reads).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub cluster: String,
    pub source: NodeSource,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub identity: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NodeError {
    #[error("node {0} not found")]
    NotFound(String),
    #[error("tenant {tenant} cannot mutate node manager owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// In-memory registry. Keyed by `(cluster, name)` to support multi-cluster
/// (clustermesh) entries. Mirrors `pkg/node/manager.Manager`.
#[derive(Debug, Default)]
pub struct NodeManager {
    tenant: Option<TenantId>,
    nodes: BTreeMap<(String, String), Node>,
}

impl NodeManager {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant: Some(tenant),
            nodes: BTreeMap::new(),
        }
    }

    pub fn upsert(&mut self, node: Node) {
        self.nodes
            .insert((node.cluster.clone(), node.name.clone()), node);
    }
    pub fn delete(&mut self, cluster: &str, name: &str) -> Option<Node> {
        self.nodes.remove(&(cluster.to_string(), name.to_string()))
    }
    pub fn get(&self, cluster: &str, name: &str) -> Option<&Node> {
        self.nodes.get(&(cluster.to_string(), name.to_string()))
    }
    pub fn list(&self) -> Vec<&Node> {
        self.nodes.values().collect()
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn tenant(&self) -> Option<&TenantId> {
        self.tenant.as_ref()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/node/types/node.go", "Node");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn n(name: &str, cluster: &str, ipv4: &str) -> Node {
        Node {
            name: name.into(),
            cluster: cluster.into(),
            source: NodeSource::Kubernetes,
            ipv4: Some(ipv4.into()),
            ipv6: None,
            labels: BTreeMap::new(),
            identity: 6, // RemoteNode
        }
    }

    #[test]
    fn node_source_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/node/types/node.go", "Source.Serde", "tenant-nm-srd");
        for s in [
            NodeSource::Local,
            NodeSource::Kvstore,
            NodeSource::Kubernetes,
            NodeSource::Custom,
            NodeSource::Restored,
            NodeSource::Unspec,
        ] {
            let j = serde_json::to_string(&s).unwrap();
            let back: NodeSource = serde_json::from_str(&j).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn manager_upsert_then_get_returns_node() {
        let (_c, t) = cilium_test_ctx!("pkg/node/types/node.go", "Manager.Upsert", "tenant-nm-up");
        let mut m = NodeManager::new(t);
        m.upsert(n("node-a", "default", "10.0.0.1"));
        let got = m.get("default", "node-a").unwrap();
        assert_eq!(got.ipv4.as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn manager_delete_removes_and_returns() {
        let (_c, t) = cilium_test_ctx!("pkg/node/types/node.go", "Manager.Delete", "tenant-nm-del");
        let mut m = NodeManager::new(t);
        m.upsert(n("node-a", "default", "10.0.0.1"));
        let deleted = m.delete("default", "node-a").unwrap();
        assert_eq!(deleted.name, "node-a");
        assert!(m.is_empty());
    }

    #[test]
    fn manager_supports_multi_cluster_entries() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/node/types/node.go",
            "Manager.MultiCluster",
            "tenant-nm-mc"
        );
        let mut m = NodeManager::new(t);
        m.upsert(n("node-a", "cluster-1", "10.1.0.1"));
        m.upsert(n("node-a", "cluster-2", "10.2.0.1"));
        assert_eq!(m.len(), 2);
        assert_eq!(
            m.get("cluster-1", "node-a").unwrap().ipv4.as_deref(),
            Some("10.1.0.1")
        );
        assert_eq!(
            m.get("cluster-2", "node-a").unwrap().ipv4.as_deref(),
            Some("10.2.0.1")
        );
    }

    #[test]
    fn manager_list_returns_all_nodes() {
        let (_c, t) = cilium_test_ctx!("pkg/node/types/node.go", "Manager.List", "tenant-nm-lst");
        let mut m = NodeManager::new(t);
        m.upsert(n("a", "c", "10.0.0.1"));
        m.upsert(n("b", "c", "10.0.0.2"));
        let list = m.list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn delete_unknown_node_returns_none() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/node/types/node.go",
            "Manager.DeleteMiss",
            "tenant-nm-dm"
        );
        let mut m = NodeManager::new(t);
        assert!(m.delete("c", "missing").is_none());
    }

    #[test]
    fn upsert_overwrites_existing_entry() {
        let (_c, t) = cilium_test_ctx!(
            "pkg/node/types/node.go",
            "Manager.Overwrite",
            "tenant-nm-ow"
        );
        let mut m = NodeManager::new(t);
        m.upsert(n("a", "c", "10.0.0.1"));
        m.upsert(n("a", "c", "10.0.0.99"));
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("c", "a").unwrap().ipv4.as_deref(), Some("10.0.0.99"));
    }

    #[test]
    fn node_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/node/types/node.go", "Node.Serde", "tenant-nm-ns");
        let mut labels = BTreeMap::new();
        labels.insert("k".into(), "v".into());
        let nd = Node {
            name: "a".into(),
            cluster: "c".into(),
            source: NodeSource::Kvstore,
            ipv4: Some("1.1.1.1".into()),
            ipv6: Some("::1".into()),
            labels,
            identity: 6,
        };
        let s = serde_json::to_string(&nd).unwrap();
        let back: Node = serde_json::from_str(&s).unwrap();
        assert_eq!(nd, back);
    }

    #[test]
    fn manager_tenant_is_preserved() {
        let (_c, t) = cilium_test_ctx!("pkg/node/types/node.go", "Manager.Tenant", "tenant-nm-tn");
        let m = NodeManager::new(t.clone());
        assert_eq!(m.tenant(), Some(&t));
    }

    #[test]
    fn node_error_renders() {
        let (_c, _t) = cilium_test_ctx!("pkg/node/types/node.go", "Errors", "tenant-nm-err");
        let e = NodeError::NotFound("x".into());
        assert!(format!("{}", e).contains("x"));
        let e = NodeError::TenantDenied {
            tenant: TenantId::new("t").expect("test fixture"),
        };
        assert!(format!("{}", e).contains("t"));
    }
}
