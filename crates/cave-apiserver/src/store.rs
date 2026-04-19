//! Resource store — namespaced, versioned storage for K8s resources.

use crate::error::{ApiError, ApiResult};
use crate::resources::Resource;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::broadcast;

type ResourceKey = (String, String, String); // (kind, namespace, name)

/// K8s-compatible resource store with watch support.
pub struct ResourceStore {
    resources: DashMap<ResourceKey, Resource>,
    revision: AtomicU64,
    watch_tx: broadcast::Sender<WatchEvent>,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub event_type: WatchEventType,
    pub resource: Resource,
}

#[derive(Debug, Clone)]
pub enum WatchEventType {
    Added,
    Modified,
    Deleted,
}

impl ResourceStore {
    pub fn new() -> Self {
        let (watch_tx, _) = broadcast::channel(4096);
        Self {
            resources: DashMap::new(),
            revision: AtomicU64::new(1),
            watch_tx,
        }
    }

    #[allow(dead_code)]
    fn next_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn create(&self, resource: Resource) -> ApiResult<Resource> {
        let key = (resource.kind().to_string(), resource.namespace().to_string(), resource.name().to_string());
        if self.resources.contains_key(&key) {
            return Err(ApiError::AlreadyExists { kind: key.0, name: key.2 });
        }
        self.resources.insert(key, resource.clone());
        let _ = self.watch_tx.send(WatchEvent { event_type: WatchEventType::Added, resource: resource.clone() });
        Ok(resource)
    }

    pub fn get(&self, kind: &str, namespace: &str, name: &str) -> ApiResult<Resource> {
        let key = (kind.to_string(), namespace.to_string(), name.to_string());
        self.resources.get(&key)
            .map(|r| r.value().clone())
            .ok_or(ApiError::NotFound { kind: kind.to_string(), name: name.to_string() })
    }

    pub fn list(&self, kind: &str, namespace: &str) -> Vec<Resource> {
        self.resources.iter()
            .filter(|r| r.key().0 == kind && (namespace.is_empty() || r.key().1 == namespace))
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn update(&self, resource: Resource) -> ApiResult<Resource> {
        let key = (resource.kind().to_string(), resource.namespace().to_string(), resource.name().to_string());
        if !self.resources.contains_key(&key) {
            return Err(ApiError::NotFound { kind: key.0, name: key.2 });
        }
        self.resources.insert(key, resource.clone());
        let _ = self.watch_tx.send(WatchEvent { event_type: WatchEventType::Modified, resource: resource.clone() });
        Ok(resource)
    }

    pub fn delete(&self, kind: &str, namespace: &str, name: &str) -> ApiResult<Resource> {
        let key = (kind.to_string(), namespace.to_string(), name.to_string());
        self.resources.remove(&key)
            .map(|(_, r)| {
                let _ = self.watch_tx.send(WatchEvent { event_type: WatchEventType::Deleted, resource: r.clone() });
                r
            })
            .ok_or(ApiError::NotFound { kind: kind.to_string(), name: name.to_string() })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.watch_tx.subscribe()
    }

    pub fn count(&self, kind: &str) -> usize {
        self.resources.iter().filter(|r| r.key().0 == kind).count()
    }
}

impl Default for ResourceStore {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    fn make_configmap(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns),
            data: HashMap::new(),
        })
    }

    #[test]
    fn test_create_and_get() {
        let store = ResourceStore::new();
        let cm = make_configmap("myconfig", "default");
        store.create(cm).unwrap();
        let got = store.get("ConfigMap", "default", "myconfig").unwrap();
        assert_eq!(got.name(), "myconfig");
    }

    #[test]
    fn test_create_duplicate_fails() {
        let store = ResourceStore::new();
        store.create(make_configmap("dup", "default")).unwrap();
        assert!(store.create(make_configmap("dup", "default")).is_err());
    }

    #[test]
    fn test_list_by_namespace() {
        let store = ResourceStore::new();
        store.create(make_configmap("a", "ns1")).unwrap();
        store.create(make_configmap("b", "ns1")).unwrap();
        store.create(make_configmap("c", "ns2")).unwrap();
        assert_eq!(store.list("ConfigMap", "ns1").len(), 2);
        assert_eq!(store.list("ConfigMap", "ns2").len(), 1);
    }

    #[test]
    fn test_delete() {
        let store = ResourceStore::new();
        store.create(make_configmap("del", "default")).unwrap();
        store.delete("ConfigMap", "default", "del").unwrap();
        assert!(store.get("ConfigMap", "default", "del").is_err());
    }

    #[test]
    fn test_watch() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_configmap("w", "default")).unwrap();
        let event = rx.try_recv().unwrap();
        assert!(matches!(event.event_type, WatchEventType::Added));
    }

    // ── helpers for new resource types ────────────────────────────────────

    fn make_statefulset(name: &str, ns: &str) -> Resource {
        Resource::StatefulSet(StatefulSet {
            api_version: "apps/v1".into(), kind: "StatefulSet".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: StatefulSetSpec::default(), status: StatefulSetStatus::default(),
        })
    }
    fn make_daemonset(name: &str, ns: &str) -> Resource {
        Resource::DaemonSet(DaemonSet {
            api_version: "apps/v1".into(), kind: "DaemonSet".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: DaemonSetSpec::default(), status: DaemonSetStatus::default(),
        })
    }
    fn make_replicaset(name: &str, ns: &str) -> Resource {
        Resource::ReplicaSet(ReplicaSet {
            api_version: "apps/v1".into(), kind: "ReplicaSet".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: ReplicaSetSpec::default(), status: ReplicaSetStatus::default(),
        })
    }
    fn make_job(name: &str, ns: &str) -> Resource {
        Resource::Job(Job {
            api_version: "batch/v1".into(), kind: "Job".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: JobSpec::default(), status: JobStatus::default(),
        })
    }
    fn make_cronjob(name: &str, ns: &str) -> Resource {
        Resource::CronJob(CronJob {
            api_version: "batch/v1".into(), kind: "CronJob".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: CronJobSpec::default(), status: CronJobStatus::default(),
        })
    }
    fn make_ingress(name: &str, ns: &str) -> Resource {
        Resource::Ingress(Ingress {
            api_version: "networking.k8s.io/v1".into(), kind: "Ingress".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: IngressSpec::default(), status: IngressStatus::default(),
        })
    }
    fn make_networkpolicy(name: &str, ns: &str) -> Resource {
        Resource::NetworkPolicy(NetworkPolicy {
            api_version: "networking.k8s.io/v1".into(), kind: "NetworkPolicy".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: NetworkPolicySpec::default(),
        })
    }
    fn make_pv(name: &str) -> Resource {
        Resource::PersistentVolume(PersistentVolume {
            api_version: "v1".into(), kind: "PersistentVolume".into(),
            metadata: ObjectMeta::new(name, ""),
            spec: PersistentVolumeSpec::default(), status: PersistentVolumeStatus::default(),
        })
    }
    fn make_pvc(name: &str, ns: &str) -> Resource {
        Resource::PersistentVolumeClaim(PersistentVolumeClaim {
            api_version: "v1".into(), kind: "PersistentVolumeClaim".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: PersistentVolumeClaimSpec::default(), status: PersistentVolumeClaimStatus::default(),
        })
    }
    fn make_storageclass(name: &str) -> Resource {
        Resource::StorageClass(StorageClass {
            api_version: "storage.k8s.io/v1".into(), kind: "StorageClass".into(),
            metadata: ObjectMeta::new(name, ""),
            provisioner: "kubernetes.io/no-provisioner".into(),
            parameters: HashMap::new(),
            reclaim_policy: Some("Retain".into()),
            volume_binding_mode: Some("WaitForFirstConsumer".into()),
            allow_volume_expansion: false,
        })
    }
    fn role_ref() -> RoleRef {
        RoleRef { api_group: "rbac.authorization.k8s.io".into(), kind: "Role".into(), name: "r".into() }
    }
    fn make_role(name: &str, ns: &str) -> Resource {
        Resource::Role(Role {
            api_version: "rbac.authorization.k8s.io/v1".into(), kind: "Role".into(),
            metadata: ObjectMeta::new(name, ns), rules: vec![],
        })
    }
    fn make_clusterrole(name: &str) -> Resource {
        Resource::ClusterRole(ClusterRole {
            api_version: "rbac.authorization.k8s.io/v1".into(), kind: "ClusterRole".into(),
            metadata: ObjectMeta::new(name, ""), rules: vec![], aggregation_rule: None,
        })
    }
    fn make_rolebinding(name: &str, ns: &str) -> Resource {
        Resource::RoleBinding(RoleBinding {
            api_version: "rbac.authorization.k8s.io/v1".into(), kind: "RoleBinding".into(),
            metadata: ObjectMeta::new(name, ns), subjects: vec![], role_ref: role_ref(),
        })
    }
    fn make_clusterrolebinding(name: &str) -> Resource {
        Resource::ClusterRoleBinding(ClusterRoleBinding {
            api_version: "rbac.authorization.k8s.io/v1".into(), kind: "ClusterRoleBinding".into(),
            metadata: ObjectMeta::new(name, ""), subjects: vec![], role_ref: role_ref(),
        })
    }
    fn make_serviceaccount(name: &str, ns: &str) -> Resource {
        Resource::ServiceAccount(ServiceAccount {
            api_version: "v1".into(), kind: "ServiceAccount".into(),
            metadata: ObjectMeta::new(name, ns),
            secrets: vec![], image_pull_secrets: vec![],
            automount_service_account_token: None,
        })
    }
    fn make_node(name: &str) -> Resource {
        Resource::Node(Node {
            api_version: "v1".into(), kind: "Node".into(),
            metadata: ObjectMeta::new(name, ""),
            spec: NodeSpec::default(), status: NodeStatus::default(),
        })
    }
    fn make_event(name: &str, ns: &str) -> Resource {
        Resource::KubeEvent(KubeEvent {
            api_version: "v1".into(), kind: "Event".into(),
            metadata: ObjectMeta::new(name, ns),
            involved_object: ObjectReference::default(),
            reason: "TestReason".into(), message: "test".into(),
            event_type: "Normal".into(), count: 1,
            first_timestamp: None, last_timestamp: None,
            source: EventSource::default(),
        })
    }
    fn make_endpoints(name: &str, ns: &str) -> Resource {
        Resource::Endpoints(Endpoints {
            api_version: "v1".into(), kind: "Endpoints".into(),
            metadata: ObjectMeta::new(name, ns), subsets: vec![],
        })
    }
    fn make_resourcequota(name: &str, ns: &str) -> Resource {
        Resource::ResourceQuota(ResourceQuota {
            api_version: "v1".into(), kind: "ResourceQuota".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: ResourceQuotaSpec::default(), status: ResourceQuotaStatus::default(),
        })
    }
    fn make_limitrange(name: &str, ns: &str) -> Resource {
        Resource::LimitRange(LimitRange {
            api_version: "v1".into(), kind: "LimitRange".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: LimitRangeSpec::default(),
        })
    }

    // ── StatefulSet tests ─────────────────────────────────────────────────

    #[test]
    fn test_statefulset_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_statefulset("sts1", "default")).unwrap();
        assert_eq!(store.get("StatefulSet", "default", "sts1").unwrap().name(), "sts1");
    }
    #[test]
    fn test_statefulset_list() {
        let store = ResourceStore::new();
        store.create(make_statefulset("a", "default")).unwrap();
        store.create(make_statefulset("b", "default")).unwrap();
        assert_eq!(store.list("StatefulSet", "default").len(), 2);
    }
    #[test]
    fn test_statefulset_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_statefulset("s", "default")).unwrap();
        store.delete("StatefulSet", "default", "s").unwrap();
        assert!(store.get("StatefulSet", "default", "s").is_err());
    }

    // ── DaemonSet tests ───────────────────────────────────────────────────

    #[test]
    fn test_daemonset_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_daemonset("ds1", "default")).unwrap();
        assert_eq!(store.get("DaemonSet", "default", "ds1").unwrap().name(), "ds1");
    }
    #[test]
    fn test_daemonset_list() {
        let store = ResourceStore::new();
        store.create(make_daemonset("a", "kube-system")).unwrap();
        store.create(make_daemonset("b", "kube-system")).unwrap();
        assert_eq!(store.list("DaemonSet", "kube-system").len(), 2);
    }
    #[test]
    fn test_daemonset_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_daemonset("d", "default")).unwrap();
        store.delete("DaemonSet", "default", "d").unwrap();
        assert!(store.get("DaemonSet", "default", "d").is_err());
    }

    // ── ReplicaSet tests ──────────────────────────────────────────────────

    #[test]
    fn test_replicaset_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_replicaset("rs1", "default")).unwrap();
        assert_eq!(store.get("ReplicaSet", "default", "rs1").unwrap().name(), "rs1");
    }
    #[test]
    fn test_replicaset_list() {
        let store = ResourceStore::new();
        store.create(make_replicaset("a", "ns1")).unwrap();
        store.create(make_replicaset("b", "ns2")).unwrap();
        assert_eq!(store.list("ReplicaSet", "ns1").len(), 1);
    }
    #[test]
    fn test_replicaset_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_replicaset("r", "default")).unwrap();
        store.delete("ReplicaSet", "default", "r").unwrap();
        assert!(store.get("ReplicaSet", "default", "r").is_err());
    }

    // ── Job tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_job_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_job("job1", "default")).unwrap();
        assert_eq!(store.get("Job", "default", "job1").unwrap().name(), "job1");
    }
    #[test]
    fn test_job_list() {
        let store = ResourceStore::new();
        store.create(make_job("j1", "default")).unwrap();
        store.create(make_job("j2", "default")).unwrap();
        assert_eq!(store.list("Job", "default").len(), 2);
    }
    #[test]
    fn test_job_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_job("j", "default")).unwrap();
        store.delete("Job", "default", "j").unwrap();
        assert!(store.get("Job", "default", "j").is_err());
    }

    // ── CronJob tests ─────────────────────────────────────────────────────

    #[test]
    fn test_cronjob_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_cronjob("cj1", "default")).unwrap();
        assert_eq!(store.get("CronJob", "default", "cj1").unwrap().name(), "cj1");
    }
    #[test]
    fn test_cronjob_list() {
        let store = ResourceStore::new();
        store.create(make_cronjob("c1", "default")).unwrap();
        store.create(make_cronjob("c2", "default")).unwrap();
        assert_eq!(store.list("CronJob", "default").len(), 2);
    }
    #[test]
    fn test_cronjob_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_cronjob("c", "default")).unwrap();
        store.delete("CronJob", "default", "c").unwrap();
        assert!(store.get("CronJob", "default", "c").is_err());
    }

    // ── Ingress tests ─────────────────────────────────────────────────────

    #[test]
    fn test_ingress_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_ingress("ing1", "default")).unwrap();
        assert_eq!(store.get("Ingress", "default", "ing1").unwrap().name(), "ing1");
    }
    #[test]
    fn test_ingress_list() {
        let store = ResourceStore::new();
        store.create(make_ingress("i1", "prod")).unwrap();
        store.create(make_ingress("i2", "prod")).unwrap();
        assert_eq!(store.list("Ingress", "prod").len(), 2);
    }
    #[test]
    fn test_ingress_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_ingress("i", "default")).unwrap();
        store.delete("Ingress", "default", "i").unwrap();
        assert!(store.get("Ingress", "default", "i").is_err());
    }

    // ── NetworkPolicy tests ───────────────────────────────────────────────

    #[test]
    fn test_networkpolicy_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_networkpolicy("np1", "default")).unwrap();
        assert_eq!(store.get("NetworkPolicy", "default", "np1").unwrap().name(), "np1");
    }
    #[test]
    fn test_networkpolicy_list() {
        let store = ResourceStore::new();
        store.create(make_networkpolicy("n1", "default")).unwrap();
        store.create(make_networkpolicy("n2", "other")).unwrap();
        assert_eq!(store.list("NetworkPolicy", "default").len(), 1);
    }
    #[test]
    fn test_networkpolicy_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_networkpolicy("n", "default")).unwrap();
        store.delete("NetworkPolicy", "default", "n").unwrap();
        assert!(store.get("NetworkPolicy", "default", "n").is_err());
    }

    // ── PersistentVolume tests ────────────────────────────────────────────

    #[test]
    fn test_pv_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_pv("pv1")).unwrap();
        assert_eq!(store.get("PersistentVolume", "", "pv1").unwrap().name(), "pv1");
    }
    #[test]
    fn test_pv_list() {
        let store = ResourceStore::new();
        store.create(make_pv("p1")).unwrap();
        store.create(make_pv("p2")).unwrap();
        assert_eq!(store.list("PersistentVolume", "").len(), 2);
    }
    #[test]
    fn test_pv_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_pv("p")).unwrap();
        store.delete("PersistentVolume", "", "p").unwrap();
        assert!(store.get("PersistentVolume", "", "p").is_err());
    }

    // ── PVC tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_pvc_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_pvc("pvc1", "default")).unwrap();
        assert_eq!(store.get("PersistentVolumeClaim", "default", "pvc1").unwrap().name(), "pvc1");
    }
    #[test]
    fn test_pvc_list() {
        let store = ResourceStore::new();
        store.create(make_pvc("v1", "default")).unwrap();
        store.create(make_pvc("v2", "default")).unwrap();
        assert_eq!(store.list("PersistentVolumeClaim", "default").len(), 2);
    }
    #[test]
    fn test_pvc_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_pvc("v", "default")).unwrap();
        store.delete("PersistentVolumeClaim", "default", "v").unwrap();
        assert!(store.get("PersistentVolumeClaim", "default", "v").is_err());
    }

    // ── StorageClass tests ────────────────────────────────────────────────

    #[test]
    fn test_storageclass_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_storageclass("standard")).unwrap();
        assert_eq!(store.get("StorageClass", "", "standard").unwrap().name(), "standard");
    }
    #[test]
    fn test_storageclass_list() {
        let store = ResourceStore::new();
        store.create(make_storageclass("fast")).unwrap();
        store.create(make_storageclass("slow")).unwrap();
        assert_eq!(store.list("StorageClass", "").len(), 2);
    }
    #[test]
    fn test_storageclass_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_storageclass("sc")).unwrap();
        store.delete("StorageClass", "", "sc").unwrap();
        assert!(store.get("StorageClass", "", "sc").is_err());
    }

    // ── Role tests ────────────────────────────────────────────────────────

    #[test]
    fn test_role_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_role("r1", "default")).unwrap();
        assert_eq!(store.get("Role", "default", "r1").unwrap().name(), "r1");
    }
    #[test]
    fn test_role_list() {
        let store = ResourceStore::new();
        store.create(make_role("r1", "default")).unwrap();
        store.create(make_role("r2", "other")).unwrap();
        assert_eq!(store.list("Role", "default").len(), 1);
    }
    #[test]
    fn test_role_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_role("r", "default")).unwrap();
        store.delete("Role", "default", "r").unwrap();
        assert!(store.get("Role", "default", "r").is_err());
    }

    // ── ClusterRole tests ─────────────────────────────────────────────────

    #[test]
    fn test_clusterrole_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_clusterrole("cr1")).unwrap();
        assert_eq!(store.get("ClusterRole", "", "cr1").unwrap().name(), "cr1");
    }
    #[test]
    fn test_clusterrole_list() {
        let store = ResourceStore::new();
        store.create(make_clusterrole("cr1")).unwrap();
        store.create(make_clusterrole("cr2")).unwrap();
        assert_eq!(store.list("ClusterRole", "").len(), 2);
    }
    #[test]
    fn test_clusterrole_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_clusterrole("cr")).unwrap();
        store.delete("ClusterRole", "", "cr").unwrap();
        assert!(store.get("ClusterRole", "", "cr").is_err());
    }

    // ── RoleBinding tests ─────────────────────────────────────────────────

    #[test]
    fn test_rolebinding_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_rolebinding("rb1", "default")).unwrap();
        assert_eq!(store.get("RoleBinding", "default", "rb1").unwrap().name(), "rb1");
    }
    #[test]
    fn test_rolebinding_list() {
        let store = ResourceStore::new();
        store.create(make_rolebinding("rb1", "default")).unwrap();
        store.create(make_rolebinding("rb2", "default")).unwrap();
        assert_eq!(store.list("RoleBinding", "default").len(), 2);
    }
    #[test]
    fn test_rolebinding_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_rolebinding("rb", "default")).unwrap();
        store.delete("RoleBinding", "default", "rb").unwrap();
        assert!(store.get("RoleBinding", "default", "rb").is_err());
    }

    // ── ClusterRoleBinding tests ──────────────────────────────────────────

    #[test]
    fn test_clusterrolebinding_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_clusterrolebinding("crb1")).unwrap();
        assert_eq!(store.get("ClusterRoleBinding", "", "crb1").unwrap().name(), "crb1");
    }
    #[test]
    fn test_clusterrolebinding_list() {
        let store = ResourceStore::new();
        store.create(make_clusterrolebinding("crb1")).unwrap();
        store.create(make_clusterrolebinding("crb2")).unwrap();
        assert_eq!(store.list("ClusterRoleBinding", "").len(), 2);
    }
    #[test]
    fn test_clusterrolebinding_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_clusterrolebinding("crb")).unwrap();
        store.delete("ClusterRoleBinding", "", "crb").unwrap();
        assert!(store.get("ClusterRoleBinding", "", "crb").is_err());
    }

    // ── ServiceAccount tests ──────────────────────────────────────────────

    #[test]
    fn test_serviceaccount_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_serviceaccount("default", "default")).unwrap();
        assert_eq!(store.get("ServiceAccount", "default", "default").unwrap().name(), "default");
    }
    #[test]
    fn test_serviceaccount_list() {
        let store = ResourceStore::new();
        store.create(make_serviceaccount("sa1", "ns1")).unwrap();
        store.create(make_serviceaccount("sa2", "ns1")).unwrap();
        assert_eq!(store.list("ServiceAccount", "ns1").len(), 2);
    }
    #[test]
    fn test_serviceaccount_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_serviceaccount("sa", "default")).unwrap();
        store.delete("ServiceAccount", "default", "sa").unwrap();
        assert!(store.get("ServiceAccount", "default", "sa").is_err());
    }

    // ── Node tests ────────────────────────────────────────────────────────

    #[test]
    fn test_node_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_node("node1")).unwrap();
        assert_eq!(store.get("Node", "", "node1").unwrap().name(), "node1");
    }
    #[test]
    fn test_node_list() {
        let store = ResourceStore::new();
        store.create(make_node("n1")).unwrap();
        store.create(make_node("n2")).unwrap();
        store.create(make_node("n3")).unwrap();
        assert_eq!(store.list("Node", "").len(), 3);
    }
    #[test]
    fn test_node_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_node("n")).unwrap();
        store.delete("Node", "", "n").unwrap();
        assert!(store.get("Node", "", "n").is_err());
    }

    // ── Event tests ───────────────────────────────────────────────────────

    #[test]
    fn test_event_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_event("ev1", "default")).unwrap();
        assert_eq!(store.get("KubeEvent", "default", "ev1").unwrap().name(), "ev1");
    }
    #[test]
    fn test_event_list() {
        let store = ResourceStore::new();
        store.create(make_event("e1", "default")).unwrap();
        store.create(make_event("e2", "default")).unwrap();
        assert_eq!(store.list("KubeEvent", "default").len(), 2);
    }
    #[test]
    fn test_event_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_event("e", "default")).unwrap();
        store.delete("KubeEvent", "default", "e").unwrap();
        assert!(store.get("KubeEvent", "default", "e").is_err());
    }

    // ── Endpoints tests ───────────────────────────────────────────────────

    #[test]
    fn test_endpoints_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_endpoints("ep1", "default")).unwrap();
        assert_eq!(store.get("Endpoints", "default", "ep1").unwrap().name(), "ep1");
    }
    #[test]
    fn test_endpoints_list() {
        let store = ResourceStore::new();
        store.create(make_endpoints("e1", "default")).unwrap();
        store.create(make_endpoints("e2", "default")).unwrap();
        assert_eq!(store.list("Endpoints", "default").len(), 2);
    }
    #[test]
    fn test_endpoints_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_endpoints("e", "default")).unwrap();
        store.delete("Endpoints", "default", "e").unwrap();
        assert!(store.get("Endpoints", "default", "e").is_err());
    }

    // ── ResourceQuota tests ───────────────────────────────────────────────

    #[test]
    fn test_resourcequota_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_resourcequota("rq1", "default")).unwrap();
        assert_eq!(store.get("ResourceQuota", "default", "rq1").unwrap().name(), "rq1");
    }
    #[test]
    fn test_resourcequota_list() {
        let store = ResourceStore::new();
        store.create(make_resourcequota("rq1", "ns1")).unwrap();
        store.create(make_resourcequota("rq2", "ns2")).unwrap();
        assert_eq!(store.list("ResourceQuota", "ns1").len(), 1);
    }
    #[test]
    fn test_resourcequota_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_resourcequota("rq", "default")).unwrap();
        store.delete("ResourceQuota", "default", "rq").unwrap();
        assert!(store.get("ResourceQuota", "default", "rq").is_err());
    }

    // ── LimitRange tests ──────────────────────────────────────────────────

    #[test]
    fn test_limitrange_create_and_get() {
        let store = ResourceStore::new();
        store.create(make_limitrange("lr1", "default")).unwrap();
        assert_eq!(store.get("LimitRange", "default", "lr1").unwrap().name(), "lr1");
    }
    #[test]
    fn test_limitrange_list() {
        let store = ResourceStore::new();
        store.create(make_limitrange("l1", "default")).unwrap();
        store.create(make_limitrange("l2", "default")).unwrap();
        assert_eq!(store.list("LimitRange", "default").len(), 2);
    }
    #[test]
    fn test_limitrange_delete_and_not_found() {
        let store = ResourceStore::new();
        store.create(make_limitrange("l", "default")).unwrap();
        store.delete("LimitRange", "default", "l").unwrap();
        assert!(store.get("LimitRange", "default", "l").is_err());
    }
}
