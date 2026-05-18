// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Resource quota & limit range ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    pub cpu_limit: String,       // e.g. "10"
    pub memory_limit: String,    // e.g. "20Gi"
    pub storage_limit: String,   // e.g. "100Gi"
    pub pod_limit: u32,
    pub service_limit: u32,
    pub secret_limit: u32,
    pub configmap_limit: u32,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            cpu_limit: "10".to_string(),
            memory_limit: "20Gi".to_string(),
            storage_limit: "100Gi".to_string(),
            pod_limit: 100,
            service_limit: 20,
            secret_limit: 50,
            configmap_limit: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitRange {
    pub default_cpu_request: String,
    pub default_cpu_limit: String,
    pub default_memory_request: String,
    pub default_memory_limit: String,
    pub max_cpu: String,
    pub max_memory: String,
}

impl Default for LimitRange {
    fn default() -> Self {
        Self {
            default_cpu_request: "100m".to_string(),
            default_cpu_limit: "500m".to_string(),
            default_memory_request: "128Mi".to_string(),
            default_memory_limit: "512Mi".to_string(),
            max_cpu: "4".to_string(),
            max_memory: "8Gi".to_string(),
        }
    }
}

// ── Namespace ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamespaceStatus {
    Provisioning,
    Active,
    Terminating,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantNamespace {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub tenant_id: String,
    pub namespace: String,
    pub status: NamespaceStatus,
    pub quota: ResourceQuota,
    pub limit_range: LimitRange,
    pub labels: HashMap<String, String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl TenantNamespace {
    pub fn new(cluster_id: Uuid, tenant_id: &str, namespace: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            cluster_id,
            tenant_id: tenant_id.to_string(),
            namespace: namespace.to_string(),
            status: NamespaceStatus::Provisioning,
            quota: ResourceQuota::default(),
            limit_range: LimitRange::default(),
            labels: HashMap::new(),
            created_at: Utc::now(),
        }
    }
}

// ── Provisioner ───────────────────────────────────────────────────────────────

pub struct NamespaceProvisioner {
    namespaces: Arc<RwLock<HashMap<Uuid, TenantNamespace>>>,
}

impl NamespaceProvisioner {
    pub fn new() -> Self {
        Self { namespaces: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Persist the namespace and transition it to `Active`.
    pub async fn provision(&self, mut ns: TenantNamespace) -> Result<TenantNamespace, String> {
        {
            let guard = self.namespaces.read().await;
            let duplicate = guard.values().any(|existing| {
                existing.cluster_id == ns.cluster_id && existing.namespace == ns.namespace
            });
            if duplicate {
                return Err(format!(
                    "namespace '{}' already exists in cluster {}",
                    ns.namespace, ns.cluster_id
                ));
            }
        }

        ns.status = NamespaceStatus::Active;
        tracing::info!(ns_id = %ns.id, namespace = %ns.namespace, "tenant namespace provisioned");
        let mut guard = self.namespaces.write().await;
        guard.insert(ns.id, ns.clone());
        Ok(ns)
    }

    pub async fn get(&self, id: Uuid) -> Option<TenantNamespace> {
        let guard = self.namespaces.read().await;
        guard.get(&id).cloned()
    }

    pub async fn get_by_name(&self, cluster_id: Uuid, namespace: &str) -> Option<TenantNamespace> {
        let guard = self.namespaces.read().await;
        guard
            .values()
            .find(|ns| ns.cluster_id == cluster_id && ns.namespace == namespace)
            .cloned()
    }

    pub async fn list_for_tenant(&self, tenant_id: &str) -> Vec<TenantNamespace> {
        let guard = self.namespaces.read().await;
        guard.values().filter(|ns| ns.tenant_id == tenant_id).cloned().collect()
    }

    pub async fn update_quota(&self, id: Uuid, quota: ResourceQuota) -> Result<(), String> {
        let mut guard = self.namespaces.write().await;
        let ns = guard.get_mut(&id).ok_or_else(|| format!("namespace {id} not found"))?;
        ns.quota = quota;
        Ok(())
    }

    pub async fn terminate(&self, id: Uuid) -> Result<(), String> {
        let mut guard = self.namespaces.write().await;
        let ns = guard.get_mut(&id).ok_or_else(|| format!("namespace {id} not found"))?;
        if ns.status == NamespaceStatus::Terminated {
            return Err(format!("namespace {id} is already terminated"));
        }
        ns.status = NamespaceStatus::Terminated;
        Ok(())
    }

    /// Check whether `requested` units of `resource` would exceed the current quota.
    ///
    /// Supported resources: `"pods"`, `"services"`, `"secrets"`, `"configmaps"`.
    /// Returns `true` if the limit would be exceeded.
    pub async fn is_quota_exceeded(
        &self,
        id: Uuid,
        resource: &str,
        requested: f64,
    ) -> Result<bool, String> {
        let guard = self.namespaces.read().await;
        let ns = guard.get(&id).ok_or_else(|| format!("namespace {id} not found"))?;
        let limit = match resource {
            "pods" => ns.quota.pod_limit as f64,
            "services" => ns.quota.service_limit as f64,
            "secrets" => ns.quota.secret_limit as f64,
            "configmaps" => ns.quota.configmap_limit as f64,
            other => return Err(format!("unknown resource: {other}")),
        };
        Ok(requested > limit)
    }
}

impl Default for NamespaceProvisioner {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(cluster_id: Uuid) -> TenantNamespace {
        TenantNamespace::new(cluster_id, "tenant-xyz", "prod-ns")
    }

    #[tokio::test]
    async fn test_provision_namespace_active() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();
        assert_eq!(provisioned.status, NamespaceStatus::Active);
    }

    #[tokio::test]
    async fn test_provision_duplicate_fails() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        prov.provision(ns(cluster_id)).await.unwrap();
        let err = prov.provision(ns(cluster_id)).await.unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[tokio::test]
    async fn test_list_for_tenant() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();

        let mut ns1 = ns(cluster_id);
        ns1.namespace = "ns-1".to_string();
        let ns2 = TenantNamespace::new(cluster_id, "tenant-xyz", "ns-2");
        let ns_other = TenantNamespace::new(cluster_id, "other-tenant", "ns-3");

        prov.provision(ns1).await.unwrap();
        prov.provision(ns2).await.unwrap();
        prov.provision(ns_other).await.unwrap();

        let result = prov.list_for_tenant("tenant-xyz").await;
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn test_update_quota() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();

        let new_quota = ResourceQuota { pod_limit: 500, ..Default::default() };
        prov.update_quota(provisioned.id, new_quota).await.unwrap();

        let updated = prov.get(provisioned.id).await.unwrap();
        assert_eq!(updated.quota.pod_limit, 500);
    }

    #[tokio::test]
    async fn test_terminate_namespace() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();
        prov.terminate(provisioned.id).await.unwrap();

        let updated = prov.get(provisioned.id).await.unwrap();
        assert_eq!(updated.status, NamespaceStatus::Terminated);
    }

    #[tokio::test]
    async fn test_terminate_already_terminated_error() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();
        prov.terminate(provisioned.id).await.unwrap();
        let err = prov.terminate(provisioned.id).await.unwrap_err();
        assert!(err.contains("already terminated"));
    }

    #[tokio::test]
    async fn test_quota_not_exceeded() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();
        // default pod_limit = 100; requesting 50 → not exceeded
        let exceeded = prov.is_quota_exceeded(provisioned.id, "pods", 50.0).await.unwrap();
        assert!(!exceeded);
    }

    #[tokio::test]
    async fn test_quota_exceeded() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();
        // default pod_limit = 100; requesting 200 → exceeded
        let exceeded = prov.is_quota_exceeded(provisioned.id, "pods", 200.0).await.unwrap();
        assert!(exceeded);
    }

    #[tokio::test]
    async fn test_get_by_name() {
        let prov = NamespaceProvisioner::new();
        let cluster_id = Uuid::new_v4();
        let provisioned = prov.provision(ns(cluster_id)).await.unwrap();
        let found = prov.get_by_name(cluster_id, "prod-ns").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, provisioned.id);
    }
}
