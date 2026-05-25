// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-tenancy: namespace per tenant, resource quotas, limit ranges.

use crate::error::{ClusterError, ClusterResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Resource quota ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    /// CPU limit (e.g. "10", "10000m")
    pub cpu_limit: String,
    /// Memory limit (e.g. "20Gi", "40960Mi")
    pub memory_limit: String,
    /// CPU request
    pub cpu_request: String,
    /// Memory request
    pub memory_request: String,
    /// Max pods
    pub max_pods: Option<i32>,
    /// Max PVCs
    pub max_pvcs: Option<i32>,
    /// Max services
    pub max_services: Option<i32>,
    /// Max configmaps
    pub max_configmaps: Option<i32>,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            cpu_limit: "10".into(),
            memory_limit: "20Gi".into(),
            cpu_request: "4".into(),
            memory_request: "8Gi".into(),
            max_pods: Some(100),
            max_pvcs: Some(20),
            max_services: Some(20),
            max_configmaps: Some(50),
        }
    }
}

// ── Limit range ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitRange {
    /// Default CPU limit per container
    pub default_cpu_limit: String,
    /// Default memory limit per container
    pub default_memory_limit: String,
    /// Default CPU request per container
    pub default_cpu_request: String,
    /// Default memory request per container
    pub default_memory_request: String,
    /// Max CPU per container
    pub max_cpu: String,
    /// Max memory per container
    pub max_memory: String,
}

impl Default for LimitRange {
    fn default() -> Self {
        Self {
            default_cpu_limit: "500m".into(),
            default_memory_limit: "512Mi".into(),
            default_cpu_request: "100m".into(),
            default_memory_request: "128Mi".into(),
            max_cpu: "4".into(),
            max_memory: "8Gi".into(),
        }
    }
}

// ── Tenant ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    /// The K8s namespace for this tenant on each cluster
    pub namespace: String,
    pub quota: ResourceQuota,
    pub limits: LimitRange,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub clusters: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Tenant {
    pub fn new(id: String, name: String) -> Self {
        let namespace = name
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect();
        let mut labels = HashMap::new();
        labels.insert("cave.io/tenant".into(), id.clone());
        labels.insert("cave.io/managed-by".into(), "cave-cluster".into());
        Self {
            id,
            name,
            namespace,
            quota: ResourceQuota::default(),
            limits: LimitRange::default(),
            labels,
            annotations: HashMap::new(),
            clusters: Vec::new(),
            created_at: chrono::Utc::now(),
        }
    }

    /// Generate Kubernetes YAML manifests for this tenant.
    pub fn to_k8s_manifests(&self) -> Vec<String> {
        let mut manifests = Vec::new();

        // Namespace
        let labels_yaml: String = self
            .labels
            .iter()
            .map(|(k, v)| format!("    {k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        manifests.push(format!(
            r#"apiVersion: v1
kind: Namespace
metadata:
  name: {ns}
  labels:
{labels}"#,
            ns = self.namespace,
            labels = labels_yaml,
        ));

        // ResourceQuota
        let mut quota_hard = Vec::new();
        quota_hard.push(format!("    limits.cpu: {}", self.quota.cpu_limit));
        quota_hard.push(format!("    limits.memory: {}", self.quota.memory_limit));
        quota_hard.push(format!("    requests.cpu: {}", self.quota.cpu_request));
        quota_hard.push(format!(
            "    requests.memory: {}",
            self.quota.memory_request
        ));
        if let Some(pods) = self.quota.max_pods {
            quota_hard.push(format!("    pods: {pods}"));
        }
        if let Some(pvcs) = self.quota.max_pvcs {
            quota_hard.push(format!("    persistentvolumeclaims: {pvcs}"));
        }
        manifests.push(format!(
            r#"apiVersion: v1
kind: ResourceQuota
metadata:
  name: {tenant}-quota
  namespace: {ns}
spec:
  hard:
{hard}"#,
            tenant = self.id,
            ns = self.namespace,
            hard = quota_hard.join("\n"),
        ));

        // LimitRange
        manifests.push(format!(
            r#"apiVersion: v1
kind: LimitRange
metadata:
  name: {tenant}-limits
  namespace: {ns}
spec:
  limits:
  - type: Container
    default:
      cpu: {cpu_limit}
      memory: {mem_limit}
    defaultRequest:
      cpu: {cpu_req}
      memory: {mem_req}
    max:
      cpu: {max_cpu}
      memory: {max_mem}"#,
            tenant = self.id,
            ns = self.namespace,
            cpu_limit = self.limits.default_cpu_limit,
            mem_limit = self.limits.default_memory_limit,
            cpu_req = self.limits.default_cpu_request,
            mem_req = self.limits.default_memory_request,
            max_cpu = self.limits.max_cpu,
            max_mem = self.limits.max_memory,
        ));

        manifests
    }
}

// ── Tenant store ──────────────────────────────────────────────────────────────

pub struct TenantStore {
    tenants: DashMap<String, Tenant>,
}

impl TenantStore {
    pub fn new() -> Self {
        Self {
            tenants: DashMap::new(),
        }
    }

    pub fn create(&self, id: String, name: String) -> ClusterResult<Tenant> {
        if self.tenants.contains_key(&id) {
            return Err(ClusterError::TenantAlreadyExists(id));
        }
        let tenant = Tenant::new(id.clone(), name);
        let result = tenant.clone();
        self.tenants.insert(id, tenant);
        Ok(result)
    }

    pub fn get(&self, id: &str) -> ClusterResult<Tenant> {
        self.tenants
            .get(id)
            .map(|t| t.clone())
            .ok_or_else(|| ClusterError::TenantNotFound(id.to_string()))
    }

    pub fn list(&self) -> Vec<Tenant> {
        self.tenants.iter().map(|e| e.value().clone()).collect()
    }

    pub fn delete(&self, id: &str) -> ClusterResult<()> {
        self.tenants
            .remove(id)
            .ok_or_else(|| ClusterError::TenantNotFound(id.to_string()))?;
        Ok(())
    }

    pub fn update_quota(&self, id: &str, quota: ResourceQuota) -> ClusterResult<Tenant> {
        let mut tenant = self
            .tenants
            .get_mut(id)
            .ok_or_else(|| ClusterError::TenantNotFound(id.to_string()))?;
        tenant.quota = quota;
        Ok(tenant.clone())
    }

    pub fn update_limits(&self, id: &str, limits: LimitRange) -> ClusterResult<Tenant> {
        let mut tenant = self
            .tenants
            .get_mut(id)
            .ok_or_else(|| ClusterError::TenantNotFound(id.to_string()))?;
        tenant.limits = limits;
        Ok(tenant.clone())
    }

    pub fn attach_to_cluster(&self, tenant_id: &str, cluster_name: &str) -> ClusterResult<()> {
        let mut tenant = self
            .tenants
            .get_mut(tenant_id)
            .ok_or_else(|| ClusterError::TenantNotFound(tenant_id.to_string()))?;
        if !tenant.clusters.contains(&cluster_name.to_string()) {
            tenant.clusters.push(cluster_name.to_string());
        }
        Ok(())
    }

    pub fn detach_from_cluster(&self, tenant_id: &str, cluster_name: &str) -> ClusterResult<()> {
        let mut tenant = self
            .tenants
            .get_mut(tenant_id)
            .ok_or_else(|| ClusterError::TenantNotFound(tenant_id.to_string()))?;
        tenant.clusters.retain(|c| c != cluster_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> TenantStore {
        TenantStore::new()
    }

    #[test]
    fn create_tenant() {
        let s = store();
        let t = s.create("team-a".into(), "Team Alpha".into()).unwrap();
        assert_eq!(t.namespace, "team-alpha");
        assert!(t.labels.contains_key("cave.io/tenant"));
    }

    #[test]
    fn duplicate_tenant_fails() {
        let s = store();
        s.create("t1".into(), "Team One".into()).unwrap();
        assert!(matches!(
            s.create("t1".into(), "Team One Dup".into()),
            Err(ClusterError::TenantAlreadyExists(_))
        ));
    }

    #[test]
    fn manifests_include_namespace_quota_limits() {
        let s = store();
        let t = s.create("eng".into(), "Engineering".into()).unwrap();
        let manifests = t.to_k8s_manifests();
        assert_eq!(manifests.len(), 3);
        assert!(manifests[0].contains("Namespace"));
        assert!(manifests[1].contains("ResourceQuota"));
        assert!(manifests[2].contains("LimitRange"));
    }

    #[test]
    fn attach_and_detach_cluster() {
        let s = store();
        s.create("t1".into(), "T1".into()).unwrap();
        s.attach_to_cluster("t1", "prod").unwrap();
        assert!(s.get("t1").unwrap().clusters.contains(&"prod".to_string()));
        s.detach_from_cluster("t1", "prod").unwrap();
        assert!(s.get("t1").unwrap().clusters.is_empty());
    }
}
