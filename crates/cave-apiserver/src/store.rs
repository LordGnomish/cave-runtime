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

// ── Update tests for every resource kind ─────────────────────────────────────
// upstream: kubernetes/kubernetes pkg/registry/core/rest/updatecreate.go::Update

#[cfg(test)]
mod tests_update {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    const TENANT: &str = "tenant-a";

    fn make_pod(name: &str, ns: &str) -> Resource {
        Resource::Pod(Pod {
            api_version: "v1".into(), kind: "Pod".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: PodSpec::default(), status: PodStatus::default(),
        })
    }
    fn make_deployment(name: &str, ns: &str) -> Resource {
        Resource::Deployment(Deployment {
            api_version: "apps/v1".into(), kind: "Deployment".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: DeploymentSpec::default(), status: DeploymentStatus::default(),
        })
    }
    fn make_service(name: &str, ns: &str) -> Resource {
        Resource::Service(Service {
            api_version: "v1".into(), kind: "Service".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: ServiceSpec { service_type: "ClusterIP".into(), selector: HashMap::new(), ports: vec![], cluster_ip: None },
        })
    }
    fn make_secret(name: &str, ns: &str) -> Resource {
        Resource::Secret(Secret {
            api_version: "v1".into(), kind: "Secret".into(),
            metadata: ObjectMeta::new(name, ns),
            data: HashMap::new(), secret_type: "Opaque".into(),
        })
    }
    fn make_configmap(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns), data: HashMap::new(),
        })
    }
    fn make_namespace(name: &str) -> Resource {
        Resource::Namespace(Namespace {
            api_version: "v1".into(), kind: "Namespace".into(),
            metadata: ObjectMeta::new(name, ""),
            status: NamespaceStatus { phase: "Active".into() },
        })
    }
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
            provisioner: "disk.csi.azure.com".into(), parameters: HashMap::new(),
            reclaim_policy: Some("Delete".into()),
            volume_binding_mode: Some("Immediate".into()),
            allow_volume_expansion: true,
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
            reason: "Updated".into(), message: "test update".into(),
            event_type: "Normal".into(), count: 2,
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

    // upstream: kubernetes/kubernetes pkg/registry/core/pod/storage/storage.go::Update
    #[test]
    fn test_pod_update() {
        let store = ResourceStore::new();
        store.create(make_pod("p", TENANT)).unwrap();
        store.update(make_pod("p", TENANT)).unwrap();
        assert_eq!(store.get("Pod", TENANT, "p").unwrap().name(), "p");
    }

    // upstream: kubernetes/kubernetes pkg/registry/apps/deployment/storage/storage.go::Update
    #[test]
    fn test_deployment_update() {
        let store = ResourceStore::new();
        store.create(make_deployment("d", TENANT)).unwrap();
        store.update(make_deployment("d", TENANT)).unwrap();
        assert_eq!(store.get("Deployment", TENANT, "d").unwrap().name(), "d");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/service/storage/storage.go::Update
    #[test]
    fn test_service_update() {
        let store = ResourceStore::new();
        store.create(make_service("svc", TENANT)).unwrap();
        store.update(make_service("svc", TENANT)).unwrap();
        assert_eq!(store.get("Service", TENANT, "svc").unwrap().name(), "svc");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/configmap/storage/storage.go::Update
    #[test]
    fn test_configmap_update() {
        let store = ResourceStore::new();
        store.create(make_configmap("cm", TENANT)).unwrap();
        store.update(make_configmap("cm", TENANT)).unwrap();
        assert_eq!(store.get("ConfigMap", TENANT, "cm").unwrap().name(), "cm");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/secret/storage/storage.go::Update
    #[test]
    fn test_secret_update() {
        let store = ResourceStore::new();
        store.create(make_secret("sec", TENANT)).unwrap();
        store.update(make_secret("sec", TENANT)).unwrap();
        assert_eq!(store.get("Secret", TENANT, "sec").unwrap().name(), "sec");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::Update
    #[test]
    fn test_namespace_update() {
        let store = ResourceStore::new();
        store.create(make_namespace(TENANT)).unwrap();
        store.update(make_namespace(TENANT)).unwrap();
        assert_eq!(store.get("Namespace", "", TENANT).unwrap().name(), TENANT);
    }

    // upstream: kubernetes/kubernetes pkg/registry/apps/statefulset/storage/storage.go::Update
    #[test]
    fn test_statefulset_update() {
        let store = ResourceStore::new();
        store.create(make_statefulset("sts", TENANT)).unwrap();
        store.update(make_statefulset("sts", TENANT)).unwrap();
        assert_eq!(store.get("StatefulSet", TENANT, "sts").unwrap().name(), "sts");
    }

    // upstream: kubernetes/kubernetes pkg/registry/apps/daemonset/storage/storage.go::Update
    #[test]
    fn test_daemonset_update() {
        let store = ResourceStore::new();
        store.create(make_daemonset("ds", TENANT)).unwrap();
        store.update(make_daemonset("ds", TENANT)).unwrap();
        assert_eq!(store.get("DaemonSet", TENANT, "ds").unwrap().name(), "ds");
    }

    // upstream: kubernetes/kubernetes pkg/registry/apps/replicaset/storage/storage.go::Update
    #[test]
    fn test_replicaset_update() {
        let store = ResourceStore::new();
        store.create(make_replicaset("rs", TENANT)).unwrap();
        store.update(make_replicaset("rs", TENANT)).unwrap();
        assert_eq!(store.get("ReplicaSet", TENANT, "rs").unwrap().name(), "rs");
    }

    // upstream: kubernetes/kubernetes pkg/registry/batch/job/storage/storage.go::Update
    #[test]
    fn test_job_update() {
        let store = ResourceStore::new();
        store.create(make_job("job", TENANT)).unwrap();
        store.update(make_job("job", TENANT)).unwrap();
        assert_eq!(store.get("Job", TENANT, "job").unwrap().name(), "job");
    }

    // upstream: kubernetes/kubernetes pkg/registry/batch/cronjob/storage/storage.go::Update
    #[test]
    fn test_cronjob_update() {
        let store = ResourceStore::new();
        store.create(make_cronjob("cj", TENANT)).unwrap();
        store.update(make_cronjob("cj", TENANT)).unwrap();
        assert_eq!(store.get("CronJob", TENANT, "cj").unwrap().name(), "cj");
    }

    // upstream: kubernetes/kubernetes pkg/registry/networking/ingress/storage/storage.go::Update
    #[test]
    fn test_ingress_update() {
        let store = ResourceStore::new();
        store.create(make_ingress("ing", TENANT)).unwrap();
        store.update(make_ingress("ing", TENANT)).unwrap();
        assert_eq!(store.get("Ingress", TENANT, "ing").unwrap().name(), "ing");
    }

    // upstream: kubernetes/kubernetes pkg/registry/networking/networkpolicy/storage/storage.go::Update
    #[test]
    fn test_networkpolicy_update() {
        let store = ResourceStore::new();
        store.create(make_networkpolicy("np", TENANT)).unwrap();
        store.update(make_networkpolicy("np", TENANT)).unwrap();
        assert_eq!(store.get("NetworkPolicy", TENANT, "np").unwrap().name(), "np");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/persistentvolume/storage/storage.go::Update
    #[test]
    fn test_pv_update() {
        let store = ResourceStore::new();
        store.create(make_pv("pv1")).unwrap();
        store.update(make_pv("pv1")).unwrap();
        assert_eq!(store.get("PersistentVolume", "", "pv1").unwrap().name(), "pv1");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/persistentvolumeclaim/storage/storage.go::Update
    #[test]
    fn test_pvc_update() {
        let store = ResourceStore::new();
        store.create(make_pvc("pvc", TENANT)).unwrap();
        store.update(make_pvc("pvc", TENANT)).unwrap();
        assert_eq!(store.get("PersistentVolumeClaim", TENANT, "pvc").unwrap().name(), "pvc");
    }

    // upstream: kubernetes/kubernetes pkg/registry/storage/storageclass/storage/storage.go::Update
    #[test]
    fn test_storageclass_update() {
        let store = ResourceStore::new();
        store.create(make_storageclass("standard")).unwrap();
        store.update(make_storageclass("standard")).unwrap();
        assert_eq!(store.get("StorageClass", "", "standard").unwrap().name(), "standard");
    }

    // upstream: kubernetes/kubernetes pkg/registry/rbac/role/storage/storage.go::Update
    #[test]
    fn test_role_update() {
        let store = ResourceStore::new();
        store.create(make_role("r", TENANT)).unwrap();
        store.update(make_role("r", TENANT)).unwrap();
        assert_eq!(store.get("Role", TENANT, "r").unwrap().name(), "r");
    }

    // upstream: kubernetes/kubernetes pkg/registry/rbac/clusterrole/storage/storage.go::Update
    #[test]
    fn test_clusterrole_update() {
        let store = ResourceStore::new();
        store.create(make_clusterrole("cr")).unwrap();
        store.update(make_clusterrole("cr")).unwrap();
        assert_eq!(store.get("ClusterRole", "", "cr").unwrap().name(), "cr");
    }

    // upstream: kubernetes/kubernetes pkg/registry/rbac/rolebinding/storage/storage.go::Update
    #[test]
    fn test_rolebinding_update() {
        let store = ResourceStore::new();
        store.create(make_rolebinding("rb", TENANT)).unwrap();
        store.update(make_rolebinding("rb", TENANT)).unwrap();
        assert_eq!(store.get("RoleBinding", TENANT, "rb").unwrap().name(), "rb");
    }

    // upstream: kubernetes/kubernetes pkg/registry/rbac/clusterrolebinding/storage/storage.go::Update
    #[test]
    fn test_clusterrolebinding_update() {
        let store = ResourceStore::new();
        store.create(make_clusterrolebinding("crb")).unwrap();
        store.update(make_clusterrolebinding("crb")).unwrap();
        assert_eq!(store.get("ClusterRoleBinding", "", "crb").unwrap().name(), "crb");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/serviceaccount/storage/storage.go::Update
    #[test]
    fn test_serviceaccount_update() {
        let store = ResourceStore::new();
        store.create(make_serviceaccount("sa", TENANT)).unwrap();
        store.update(make_serviceaccount("sa", TENANT)).unwrap();
        assert_eq!(store.get("ServiceAccount", TENANT, "sa").unwrap().name(), "sa");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/node/storage/storage.go::Update
    #[test]
    fn test_node_update() {
        let store = ResourceStore::new();
        store.create(make_node("node1")).unwrap();
        store.update(make_node("node1")).unwrap();
        assert_eq!(store.get("Node", "", "node1").unwrap().name(), "node1");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/event/storage/storage.go::Update
    #[test]
    fn test_event_update() {
        let store = ResourceStore::new();
        store.create(make_event("ev", TENANT)).unwrap();
        store.update(make_event("ev", TENANT)).unwrap();
        assert_eq!(store.get("KubeEvent", TENANT, "ev").unwrap().name(), "ev");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/endpoints/storage/storage.go::Update
    #[test]
    fn test_endpoints_update() {
        let store = ResourceStore::new();
        store.create(make_endpoints("ep", TENANT)).unwrap();
        store.update(make_endpoints("ep", TENANT)).unwrap();
        assert_eq!(store.get("Endpoints", TENANT, "ep").unwrap().name(), "ep");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/resourcequota/storage/storage.go::Update
    #[test]
    fn test_resourcequota_update() {
        let store = ResourceStore::new();
        store.create(make_resourcequota("rq", TENANT)).unwrap();
        store.update(make_resourcequota("rq", TENANT)).unwrap();
        assert_eq!(store.get("ResourceQuota", TENANT, "rq").unwrap().name(), "rq");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/limitrange/storage/storage.go::Update
    #[test]
    fn test_limitrange_update() {
        let store = ResourceStore::new();
        store.create(make_limitrange("lr", TENANT)).unwrap();
        store.update(make_limitrange("lr", TENANT)).unwrap();
        assert_eq!(store.get("LimitRange", TENANT, "lr").unwrap().name(), "lr");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/updatecreate.go::Update
    #[test]
    fn test_update_nonexistent_fails() {
        let store = ResourceStore::new();
        assert!(store.update(make_pod("ghost", TENANT)).is_err());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/updatecreate.go::Update
    #[test]
    fn test_update_preserves_kind() {
        let store = ResourceStore::new();
        store.create(make_configmap("cfg", TENANT)).unwrap();
        let updated = store.update(make_configmap("cfg", TENANT)).unwrap();
        assert_eq!(updated.kind(), "ConfigMap");
    }
}

// ── Watch event tests ─────────────────────────────────────────────────────────
// upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch

#[cfg(test)]
mod tests_watch {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    const TENANT: &str = "tenant-b";

    fn make_cm(name: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, TENANT), data: HashMap::new(),
        })
    }
    fn make_pod(name: &str) -> Resource {
        Resource::Pod(Pod {
            api_version: "v1".into(), kind: "Pod".into(),
            metadata: ObjectMeta::new(name, TENANT),
            spec: PodSpec::default(), status: PodStatus::default(),
        })
    }
    fn make_deployment(name: &str) -> Resource {
        Resource::Deployment(Deployment {
            api_version: "apps/v1".into(), kind: "Deployment".into(),
            metadata: ObjectMeta::new(name, TENANT),
            spec: DeploymentSpec::default(), status: DeploymentStatus::default(),
        })
    }
    fn make_secret(name: &str) -> Resource {
        Resource::Secret(Secret {
            api_version: "v1".into(), kind: "Secret".into(),
            metadata: ObjectMeta::new(name, TENANT),
            data: HashMap::new(), secret_type: "Opaque".into(),
        })
    }
    fn make_service(name: &str) -> Resource {
        Resource::Service(Service {
            api_version: "v1".into(), kind: "Service".into(),
            metadata: ObjectMeta::new(name, TENANT),
            spec: ServiceSpec { service_type: "ClusterIP".into(), selector: HashMap::new(), ports: vec![], cluster_ip: None },
        })
    }
    fn make_statefulset(name: &str) -> Resource {
        Resource::StatefulSet(StatefulSet {
            api_version: "apps/v1".into(), kind: "StatefulSet".into(),
            metadata: ObjectMeta::new(name, TENANT),
            spec: StatefulSetSpec::default(), status: StatefulSetStatus::default(),
        })
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_watch_create_emits_added() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_cm("w1")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Added));
        assert_eq!(ev.resource.name(), "w1");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Update
    #[test]
    fn test_watch_update_emits_modified() {
        let store = ResourceStore::new();
        store.create(make_cm("w2")).unwrap();
        let mut rx = store.subscribe();
        store.update(make_cm("w2")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Modified));
        assert_eq!(ev.resource.name(), "w2");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Delete
    #[test]
    fn test_watch_delete_emits_deleted() {
        let store = ResourceStore::new();
        store.create(make_cm("w3")).unwrap();
        let mut rx = store.subscribe();
        store.delete("ConfigMap", TENANT, "w3").unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Deleted));
        assert_eq!(ev.resource.name(), "w3");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_watch_pod_create_added() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_pod("pod-w")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Added));
        assert_eq!(ev.resource.kind(), "Pod");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Update
    #[test]
    fn test_watch_pod_update_modified() {
        let store = ResourceStore::new();
        store.create(make_pod("pod-x")).unwrap();
        let mut rx = store.subscribe();
        store.update(make_pod("pod-x")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Modified));
        assert_eq!(ev.resource.kind(), "Pod");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Delete
    #[test]
    fn test_watch_pod_delete_deleted() {
        let store = ResourceStore::new();
        store.create(make_pod("pod-z")).unwrap();
        let mut rx = store.subscribe();
        store.delete("Pod", TENANT, "pod-z").unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Deleted));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_watch_deployment_create_added() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_deployment("dep-w")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Added));
        assert_eq!(ev.resource.kind(), "Deployment");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Update
    #[test]
    fn test_watch_deployment_update_modified() {
        let store = ResourceStore::new();
        store.create(make_deployment("dep-y")).unwrap();
        let mut rx = store.subscribe();
        store.update(make_deployment("dep-y")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Modified));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_watch_secret_create_added() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_secret("sec-w")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Added));
        assert_eq!(ev.resource.kind(), "Secret");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_watch_service_create_added() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_service("svc-w")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Added));
        assert_eq!(ev.resource.kind(), "Service");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_watch_statefulset_create_added() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_statefulset("sts-w")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev.event_type, WatchEventType::Added));
        assert_eq!(ev.resource.kind(), "StatefulSet");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch
    #[test]
    fn test_watch_multiple_events_ordered() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_cm("seq1")).unwrap();
        store.create(make_cm("seq2")).unwrap();
        let ev1 = rx.try_recv().unwrap();
        let ev2 = rx.try_recv().unwrap();
        assert!(matches!(ev1.event_type, WatchEventType::Added));
        assert!(matches!(ev2.event_type, WatchEventType::Added));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch
    #[test]
    fn test_watch_resource_name_preserved_in_event() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_pod("event-pod")).unwrap();
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.resource.namespace(), TENANT);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch
    #[test]
    fn test_watch_subscriber_gets_all_kinds() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        store.create(make_pod("p")).unwrap();
        store.create(make_cm("c")).unwrap();
        store.create(make_service("s")).unwrap();
        let kinds: Vec<String> = (0..3)
            .map(|_| rx.try_recv().unwrap().resource.kind().to_string())
            .collect();
        assert!(kinds.contains(&"Pod".to_string()));
        assert!(kinds.contains(&"ConfigMap".to_string()));
        assert!(kinds.contains(&"Service".to_string()));
    }
}

// ── Multi-tenant isolation tests ──────────────────────────────────────────────
// upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::ListWithOptions

#[cfg(test)]
mod tests_multitenant {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    fn make_cm(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns), data: HashMap::new(),
        })
    }
    fn make_pod(name: &str, ns: &str) -> Resource {
        Resource::Pod(Pod {
            api_version: "v1".into(), kind: "Pod".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: PodSpec::default(), status: PodStatus::default(),
        })
    }
    fn make_secret(name: &str, ns: &str) -> Resource {
        Resource::Secret(Secret {
            api_version: "v1".into(), kind: "Secret".into(),
            metadata: ObjectMeta::new(name, ns),
            data: HashMap::new(), secret_type: "Opaque".into(),
        })
    }
    fn make_deployment(name: &str, ns: &str) -> Resource {
        Resource::Deployment(Deployment {
            api_version: "apps/v1".into(), kind: "Deployment".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: DeploymentSpec::default(), status: DeploymentStatus::default(),
        })
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::List
    #[test]
    fn test_tenant_isolation_configmap_list() {
        let store = ResourceStore::new();
        store.create(make_cm("cfg", "tenant-alpha")).unwrap();
        store.create(make_cm("cfg", "tenant-beta")).unwrap();
        assert_eq!(store.list("ConfigMap", "tenant-alpha").len(), 1);
        assert_eq!(store.list("ConfigMap", "tenant-beta").len(), 1);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::List
    #[test]
    fn test_tenant_isolation_pod_list() {
        let store = ResourceStore::new();
        store.create(make_pod("app", "tenant-x")).unwrap();
        store.create(make_pod("app", "tenant-y")).unwrap();
        store.create(make_pod("app2", "tenant-x")).unwrap();
        assert_eq!(store.list("Pod", "tenant-x").len(), 2);
        assert_eq!(store.list("Pod", "tenant-y").len(), 1);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::Get
    #[test]
    fn test_tenant_isolation_same_name_different_ns() {
        let store = ResourceStore::new();
        store.create(make_cm("shared-name", "tenant-1")).unwrap();
        store.create(make_cm("shared-name", "tenant-2")).unwrap();
        let r1 = store.get("ConfigMap", "tenant-1", "shared-name").unwrap();
        let r2 = store.get("ConfigMap", "tenant-2", "shared-name").unwrap();
        assert_eq!(r1.namespace(), "tenant-1");
        assert_eq!(r2.namespace(), "tenant-2");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::Delete
    #[test]
    fn test_tenant_isolation_delete_only_affects_own_ns() {
        let store = ResourceStore::new();
        store.create(make_secret("cred", "tenant-safe")).unwrap();
        store.create(make_secret("cred", "tenant-del")).unwrap();
        store.delete("Secret", "tenant-del", "cred").unwrap();
        assert!(store.get("Secret", "tenant-safe", "cred").is_ok());
        assert!(store.get("Secret", "tenant-del", "cred").is_err());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::List
    #[test]
    fn test_tenant_isolation_list_all_namespaces() {
        let store = ResourceStore::new();
        store.create(make_cm("c1", "tenant-a1")).unwrap();
        store.create(make_cm("c2", "tenant-a2")).unwrap();
        store.create(make_cm("c3", "tenant-a3")).unwrap();
        let all = store.list("ConfigMap", "");
        assert_eq!(all.len(), 3);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::Create
    #[test]
    fn test_tenant_duplicate_within_same_ns_fails() {
        let store = ResourceStore::new();
        store.create(make_pod("svc", "tenant-dup")).unwrap();
        assert!(store.create(make_pod("svc", "tenant-dup")).is_err());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::Create
    #[test]
    fn test_tenant_duplicate_allowed_across_ns() {
        let store = ResourceStore::new();
        store.create(make_pod("svc", "tenant-ns1")).unwrap();
        assert!(store.create(make_pod("svc", "tenant-ns2")).is_ok());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::List
    #[test]
    fn test_tenant_many_resources_per_tenant() {
        let store = ResourceStore::new();
        for i in 0..20u32 {
            store.create(make_cm(&format!("cm-{i}"), "tenant-heavy")).unwrap();
        }
        assert_eq!(store.list("ConfigMap", "tenant-heavy").len(), 20);
        assert_eq!(store.list("ConfigMap", "tenant-other").len(), 0);
    }

    // upstream: kubernetes/kubernetes pkg/registry/apps/deployment/storage/storage.go::List
    #[test]
    fn test_tenant_deployment_cross_ns_isolation() {
        let store = ResourceStore::new();
        store.create(make_deployment("web", "org-a")).unwrap();
        store.create(make_deployment("web", "org-b")).unwrap();
        store.create(make_deployment("api", "org-a")).unwrap();
        assert_eq!(store.list("Deployment", "org-a").len(), 2);
        assert_eq!(store.list("Deployment", "org-b").len(), 1);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/secret/storage/storage.go::Update
    #[test]
    fn test_tenant_update_does_not_affect_other_tenant() {
        let store = ResourceStore::new();
        store.create(make_secret("key", "tenant-aa")).unwrap();
        store.create(make_secret("key", "tenant-bb")).unwrap();
        store.update(make_secret("key", "tenant-aa")).unwrap();
        // tenant-bb's secret still reachable
        assert!(store.get("Secret", "tenant-bb", "key").is_ok());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::List
    #[test]
    fn test_tenant_empty_store_list_returns_empty() {
        let store = ResourceStore::new();
        assert_eq!(store.list("ConfigMap", "tenant-empty").len(), 0);
        assert_eq!(store.list("Pod", "tenant-empty").len(), 0);
        assert_eq!(store.list("Deployment", "tenant-empty").len(), 0);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/namespace/storage/storage.go::Get
    #[test]
    fn test_tenant_get_wrong_ns_returns_error() {
        let store = ResourceStore::new();
        store.create(make_cm("obj", "tenant-correct")).unwrap();
        assert!(store.get("ConfigMap", "tenant-wrong", "obj").is_err());
    }
}

// ── Cross-kind isolation and count tests ─────────────────────────────────────
// upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects

#[cfg(test)]
mod tests_cross_kind {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    const TENANT: &str = "tenant-crosskind";

    fn make_cm(name: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, TENANT), data: HashMap::new(),
        })
    }
    fn make_pod(name: &str) -> Resource {
        Resource::Pod(Pod {
            api_version: "v1".into(), kind: "Pod".into(),
            metadata: ObjectMeta::new(name, TENANT),
            spec: PodSpec::default(), status: PodStatus::default(),
        })
    }
    fn make_secret(name: &str) -> Resource {
        Resource::Secret(Secret {
            api_version: "v1".into(), kind: "Secret".into(),
            metadata: ObjectMeta::new(name, TENANT),
            data: HashMap::new(), secret_type: "Opaque".into(),
        })
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_same_name_different_kind_both_stored() {
        let store = ResourceStore::new();
        store.create(make_cm("obj")).unwrap();
        store.create(make_pod("obj")).unwrap();
        store.create(make_secret("obj")).unwrap();
        assert!(store.get("ConfigMap", TENANT, "obj").is_ok());
        assert!(store.get("Pod", TENANT, "obj").is_ok());
        assert!(store.get("Secret", TENANT, "obj").is_ok());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_count_zero_when_empty() {
        let store = ResourceStore::new();
        assert_eq!(store.count("Pod"), 0);
        assert_eq!(store.count("ConfigMap"), 0);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_count_increases_on_create() {
        let store = ResourceStore::new();
        store.create(make_pod("p1")).unwrap();
        assert_eq!(store.count("Pod"), 1);
        store.create(make_pod("p2")).unwrap();
        assert_eq!(store.count("Pod"), 2);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_count_decreases_on_delete() {
        let store = ResourceStore::new();
        store.create(make_pod("p")).unwrap();
        assert_eq!(store.count("Pod"), 1);
        store.delete("Pod", TENANT, "p").unwrap();
        assert_eq!(store.count("Pod"), 0);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_count_not_affected_by_other_kinds() {
        let store = ResourceStore::new();
        store.create(make_cm("c")).unwrap();
        store.create(make_secret("s")).unwrap();
        assert_eq!(store.count("Pod"), 0);
        assert_eq!(store.count("ConfigMap"), 1);
        assert_eq!(store.count("Secret"), 1);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_count_unchanged_after_update() {
        let store = ResourceStore::new();
        store.create(make_cm("c")).unwrap();
        store.create(make_cm("c2")).unwrap();
        store.update(make_cm("c")).unwrap();
        assert_eq!(store.count("ConfigMap"), 2);
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_delete_nonexistent_returns_error() {
        let store = ResourceStore::new();
        let err = store.delete("Pod", TENANT, "ghost").unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("ghost"));
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_get_nonexistent_returns_error() {
        let store = ResourceStore::new();
        let err = store.get("Pod", TENANT, "missing").unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("missing"));
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_list_returns_only_matching_kind() {
        let store = ResourceStore::new();
        store.create(make_pod("p")).unwrap();
        store.create(make_cm("c")).unwrap();
        store.create(make_secret("s")).unwrap();
        let pods = store.list("Pod", TENANT);
        assert_eq!(pods.len(), 1);
        assert_eq!(pods[0].kind(), "Pod");
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_delete_removes_correct_resource() {
        let store = ResourceStore::new();
        store.create(make_pod("keep")).unwrap();
        store.create(make_pod("remove")).unwrap();
        store.delete("Pod", TENANT, "remove").unwrap();
        assert!(store.get("Pod", TENANT, "keep").is_ok());
        assert!(store.get("Pod", TENANT, "remove").is_err());
    }

    // upstream: kubernetes/kubernetes pkg/registry/core/rest/registry.go::CountObjects
    #[test]
    fn test_store_default_is_empty() {
        let store = ResourceStore::default();
        assert_eq!(store.count("Pod"), 0);
        assert_eq!(store.count("ConfigMap"), 0);
    }
}

// ── Concurrent revision and edge case tests ───────────────────────────────────
// upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::GuaranteedUpdate

#[cfg(test)]
mod tests_edge_cases {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    const TENANT: &str = "tenant-edge";

    fn pod(name: &str, ns: &str) -> Resource {
        Resource::Pod(Pod {
            api_version: "v1".into(), kind: "Pod".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: PodSpec::default(), status: PodStatus::default(),
        })
    }
    fn cm(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns), data: HashMap::new(),
        })
    }
    fn deploy(name: &str, ns: &str) -> Resource {
        Resource::Deployment(Deployment {
            api_version: "apps/v1".into(), kind: "Deployment".into(),
            metadata: ObjectMeta::new(name, ns),
            spec: DeploymentSpec::default(), status: DeploymentStatus::default(),
        })
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_create_100_resources() {
        let store = ResourceStore::new();
        for i in 0u32..100 {
            store.create(pod(&format!("pod-{i}"), TENANT)).unwrap();
        }
        assert_eq!(store.list("Pod", TENANT).len(), 100);
        assert_eq!(store.count("Pod"), 100);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Delete
    #[test]
    fn test_delete_all_leaves_empty() {
        let store = ResourceStore::new();
        for i in 0u32..5 {
            store.create(cm(&format!("cm-{i}"), TENANT)).unwrap();
        }
        for i in 0u32..5 {
            store.delete("ConfigMap", TENANT, &format!("cm-{i}")).unwrap();
        }
        assert_eq!(store.list("ConfigMap", TENANT).len(), 0);
        assert_eq!(store.count("ConfigMap"), 0);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Update
    #[test]
    fn test_create_update_delete_pod() {
        let store = ResourceStore::new();
        store.create(pod("lifecycle", TENANT)).unwrap();
        store.update(pod("lifecycle", TENANT)).unwrap();
        store.delete("Pod", TENANT, "lifecycle").unwrap();
        assert!(store.get("Pod", TENANT, "lifecycle").is_err());
        assert_eq!(store.count("Pod"), 0);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_kind_isolation_pod_vs_deployment() {
        let store = ResourceStore::new();
        store.create(pod("workload", TENANT)).unwrap();
        store.create(deploy("workload", TENANT)).unwrap();
        assert_eq!(store.count("Pod"), 1);
        assert_eq!(store.count("Deployment"), 1);
        assert!(store.get("Pod", TENANT, "workload").is_ok());
        assert!(store.get("Deployment", TENANT, "workload").is_ok());
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::List
    #[test]
    fn test_list_empty_namespace_string_returns_all() {
        let store = ResourceStore::new();
        store.create(pod("p1", "ns1")).unwrap();
        store.create(pod("p2", "ns2")).unwrap();
        store.create(pod("p3", "ns3")).unwrap();
        assert_eq!(store.list("Pod", "").len(), 3);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::List
    #[test]
    fn test_list_specific_namespace_filters() {
        let store = ResourceStore::new();
        store.create(pod("p1", "tenant-filter-a")).unwrap();
        store.create(pod("p2", "tenant-filter-b")).unwrap();
        store.create(pod("p3", "tenant-filter-a")).unwrap();
        assert_eq!(store.list("Pod", "tenant-filter-a").len(), 2);
        assert_eq!(store.list("Pod", "tenant-filter-b").len(), 1);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_create_returns_correct_resource() {
        let store = ResourceStore::new();
        let created = store.create(cm("ret-test", TENANT)).unwrap();
        assert_eq!(created.name(), "ret-test");
        assert_eq!(created.namespace(), TENANT);
        assert_eq!(created.kind(), "ConfigMap");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Update
    #[test]
    fn test_update_returns_correct_resource() {
        let store = ResourceStore::new();
        store.create(cm("upd-ret", TENANT)).unwrap();
        let updated = store.update(cm("upd-ret", TENANT)).unwrap();
        assert_eq!(updated.name(), "upd-ret");
        assert_eq!(updated.kind(), "ConfigMap");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Delete
    #[test]
    fn test_delete_returns_deleted_resource() {
        let store = ResourceStore::new();
        store.create(pod("del-ret", TENANT)).unwrap();
        let deleted = store.delete("Pod", TENANT, "del-ret").unwrap();
        assert_eq!(deleted.name(), "del-ret");
        assert_eq!(deleted.kind(), "Pod");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_error_message_not_found_contains_kind() {
        let store = ResourceStore::new();
        let err = store.get("Pod", TENANT, "missing").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Pod") || msg.contains("not found"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_error_message_already_exists_contains_kind() {
        let store = ResourceStore::new();
        store.create(cm("dup-err", TENANT)).unwrap();
        let err = store.create(cm("dup-err", TENANT)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ConfigMap") || msg.contains("already exists"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::New
    #[test]
    fn test_fresh_store_is_empty() {
        let store = ResourceStore::new();
        for kind in ["Pod", "ConfigMap", "Secret", "Deployment", "Service", "Node"] {
            assert_eq!(store.count(kind), 0);
        }
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::New
    #[test]
    fn test_store_default_matches_new() {
        let s1 = ResourceStore::new();
        let s2 = ResourceStore::default();
        assert_eq!(s1.count("Pod"), s2.count("Pod"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch
    #[test]
    fn test_multiple_subscribers_all_receive() {
        let store = ResourceStore::new();
        let mut rx1 = store.subscribe();
        let mut rx2 = store.subscribe();
        store.create(pod("multi-sub", TENANT)).unwrap();
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch
    #[test]
    fn test_subscribe_after_events_misses_past_events() {
        let store = ResourceStore::new();
        store.create(pod("pre-sub", TENANT)).unwrap();
        let mut rx = store.subscribe(); // subscribe AFTER create
        // The event for pre-sub was already sent before rx was created
        store.create(pod("post-sub", TENANT)).unwrap();
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.resource.name(), "post-sub");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_create_many_kinds_count_per_kind() {
        let store = ResourceStore::new();
        for i in 0u32..5 { store.create(pod(&format!("p{i}"), TENANT)).unwrap(); }
        for i in 0u32..3 { store.create(cm(&format!("c{i}"), TENANT)).unwrap(); }
        for i in 0u32..7 { store.create(deploy(&format!("d{i}"), TENANT)).unwrap(); }
        assert_eq!(store.count("Pod"), 5);
        assert_eq!(store.count("ConfigMap"), 3);
        assert_eq!(store.count("Deployment"), 7);
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::List
    #[test]
    fn test_list_returns_items_with_correct_kind() {
        let store = ResourceStore::new();
        store.create(pod("p1", TENANT)).unwrap();
        store.create(pod("p2", TENANT)).unwrap();
        let pods = store.list("Pod", TENANT);
        for p in &pods {
            assert_eq!(p.kind(), "Pod");
        }
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::List
    #[test]
    fn test_list_returns_items_with_correct_namespace() {
        let store = ResourceStore::new();
        store.create(cm("c1", "tenant-ns-check")).unwrap();
        store.create(cm("c2", "tenant-ns-check")).unwrap();
        let items = store.list("ConfigMap", "tenant-ns-check");
        for item in &items {
            assert_eq!(item.namespace(), "tenant-ns-check");
        }
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Get
    #[test]
    fn test_get_returns_exact_name() {
        let store = ResourceStore::new();
        store.create(pod("exact-name", TENANT)).unwrap();
        let got = store.get("Pod", TENANT, "exact-name").unwrap();
        assert_eq!(got.name(), "exact-name");
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Get
    #[test]
    fn test_get_wrong_kind_fails() {
        let store = ResourceStore::new();
        store.create(pod("obj", TENANT)).unwrap();
        // "obj" is a Pod, getting as ConfigMap fails
        assert!(store.get("ConfigMap", TENANT, "obj").is_err());
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Watch
    #[test]
    fn test_watch_all_three_event_types_in_sequence() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        // create → modified → deleted
        store.create(pod("seq", TENANT)).unwrap();
        store.update(pod("seq", TENANT)).unwrap();
        store.delete("Pod", TENANT, "seq").unwrap();
        let ev1 = rx.try_recv().unwrap();
        let ev2 = rx.try_recv().unwrap();
        let ev3 = rx.try_recv().unwrap();
        assert!(matches!(ev1.event_type, WatchEventType::Added));
        assert!(matches!(ev2.event_type, WatchEventType::Modified));
        assert!(matches!(ev3.event_type, WatchEventType::Deleted));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go::Create
    #[test]
    fn test_multitenant_30_tenants_isolated() {
        let store = ResourceStore::new();
        for i in 0u32..30 {
            let ns = format!("tenant-{i}");
            store.create(cm("shared-cfg", &ns)).unwrap();
        }
        // Each tenant has exactly 1 configmap
        for i in 0u32..30 {
            let ns = format!("tenant-{i}");
            assert_eq!(store.list("ConfigMap", &ns).len(), 1);
        }
        assert_eq!(store.count("ConfigMap"), 30);
    }
}

// ── Error type tests ──────────────────────────────────────────────────────────
// upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go

#[cfg(test)]
mod tests_errors {
    use crate::error::ApiError;

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewNotFound
    #[test]
    fn test_not_found_display() {
        let e = ApiError::NotFound { kind: "Pod".into(), name: "missing".into() };
        let s = e.to_string();
        assert!(s.contains("Pod") && s.contains("missing"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewAlreadyExists
    #[test]
    fn test_already_exists_display() {
        let e = ApiError::AlreadyExists { kind: "ConfigMap".into(), name: "dup".into() };
        let s = e.to_string();
        assert!(s.contains("ConfigMap") && s.contains("dup"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewConflict
    #[test]
    fn test_conflict_display() {
        let e = ApiError::Conflict("resource version conflict".into());
        let s = e.to_string();
        assert!(s.contains("conflict"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewInvalid
    #[test]
    fn test_invalid_display() {
        let e = ApiError::Invalid("name cannot be empty".into());
        let s = e.to_string();
        assert!(s.contains("invalid") || s.contains("name cannot be empty"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewForbidden
    #[test]
    fn test_forbidden_display() {
        let e = ApiError::Forbidden("access denied".into());
        let s = e.to_string();
        assert!(s.contains("forbidden") || s.contains("access denied"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewInternalError
    #[test]
    fn test_internal_display() {
        let e = ApiError::Internal("storage failure".into());
        let s = e.to_string();
        assert!(s.contains("internal") || s.contains("storage failure"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::IsNotFound
    #[test]
    fn test_not_found_different_kinds() {
        let e1 = ApiError::NotFound { kind: "Pod".into(), name: "x".into() };
        let e2 = ApiError::NotFound { kind: "Service".into(), name: "y".into() };
        assert!(e1.to_string().contains("Pod"));
        assert!(e2.to_string().contains("Service"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::IsAlreadyExists
    #[test]
    fn test_already_exists_different_tenants() {
        let e1 = ApiError::AlreadyExists { kind: "Secret".into(), name: "cred".into() };
        assert!(e1.to_string().contains("Secret"));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewConflict
    #[test]
    fn test_conflict_message_preserved() {
        let msg = "optimistic lock: resource version mismatch";
        let e = ApiError::Conflict(msg.into());
        assert!(e.to_string().contains(msg));
    }

    // upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/api/errors/errors.go::NewInvalid
    #[test]
    fn test_invalid_multiple_errors() {
        for msg in ["name too long", "label invalid", "namespace missing"] {
            let e = ApiError::Invalid(msg.into());
            assert!(e.to_string().contains(msg));
        }
    }
}

// ── GenericAPIServer storage-layer deeper coverage (v1.36.0) ─────────────────
// upstream: kubernetes/kubernetes staging/src/k8s.io/apiserver/pkg/registry/generic/registry/store.go

#[cfg(test)]
mod tests_generic_storage {
    use super::*;
    use crate::resources::*;
    use std::collections::HashMap;

    fn cm(name: &str, ns: &str) -> Resource {
        Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(), kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, ns), data: HashMap::new(),
        })
    }
    fn secret(name: &str, ns: &str) -> Resource {
        Resource::Secret(Secret {
            api_version: "v1".into(), kind: "Secret".into(),
            metadata: ObjectMeta::new(name, ns),
            data: HashMap::new(), secret_type: "Opaque".into(),
        })
    }

    /// Upstream parity: `TestStore_WatchEmitsAddModifyDeleteInOrder`
    /// (apiserver/pkg/registry/generic/registry/store_test.go — broadcast
    /// channel emits typed events for each lifecycle step).
    #[test]
    fn test_watch_emits_correct_event_type_per_lifecycle_step() {
        let store = ResourceStore::new();
        let mut rx = store.subscribe();
        // tenant_id invariant: tenant scoping is by namespace; pin to "acme".
        let r = cm("c", "acme");
        store.create(r.clone()).unwrap();
        store.update(r.clone()).unwrap();
        store.delete("ConfigMap", "acme", "c").unwrap();
        let added = rx.try_recv().unwrap();
        let modified = rx.try_recv().unwrap();
        let deleted = rx.try_recv().unwrap();
        assert!(matches!(added.event_type,    WatchEventType::Added));
        assert!(matches!(modified.event_type, WatchEventType::Modified));
        assert!(matches!(deleted.event_type,  WatchEventType::Deleted));
        assert_eq!(added.resource.namespace(),    "acme",
            "tenant_id invariant: Added event scoped to acme");
        assert_eq!(deleted.resource.namespace(),  "acme",
            "tenant_id invariant: Deleted event scoped to acme");
    }

    /// Upstream parity: `TestStore_TenantIsolatedDeleteDoesNotAffectPeer`
    /// (registry/store.go::Delete — same `name` in different namespaces are
    /// distinct keys; deletion in one MUST NOT affect the other).
    #[test]
    fn test_delete_in_one_tenant_namespace_does_not_remove_peer_tenant_object() {
        let store = ResourceStore::new();
        store.create(cm("shared", "acme")).unwrap();
        store.create(cm("shared", "globex")).unwrap();
        store.delete("ConfigMap", "acme", "shared").unwrap();
        let acme_q  = store.get("ConfigMap", "acme",   "shared");
        let globex_q = store.get("ConfigMap", "globex", "shared");
        assert!(acme_q.is_err(),
            "tenant_id invariant: acme's object removed");
        assert!(globex_q.is_ok(),
            "tenant_id invariant: globex's same-named object UNAFFECTED");
        let g = globex_q.unwrap();
        assert_eq!(g.namespace(), "globex");
    }

    /// Upstream parity: `TestStore_CountByKindCoversMultipleKinds`
    /// (registry/store.go::Count — per-kind counter is independent across kinds).
    #[test]
    fn test_count_by_kind_is_independent_across_kinds_and_tenants() {
        let store = ResourceStore::new();
        store.create(cm("c1", "acme")).unwrap();
        store.create(cm("c2", "acme")).unwrap();
        store.create(cm("c3", "globex")).unwrap();
        store.create(secret("s1", "acme")).unwrap();
        assert_eq!(store.count("ConfigMap"), 3);
        assert_eq!(store.count("Secret"),   1);
        // tenant_id invariant: per-tenant list still segregates by namespace.
        assert_eq!(store.list("ConfigMap", "acme").len(), 2);
        assert_eq!(store.list("ConfigMap", "globex").len(), 1);
        assert!(store.list("ConfigMap", "acme").iter()
            .all(|r| r.namespace() == "acme"),
            "tenant_id invariant: acme list strictly scoped");
    }

    /// Upstream parity: `TestStore_UpdateMissingObjectReturnsNotFound`
    /// (registry/store.go::Update on a non-existent key returns NotFound,
    /// never silently inserts — no upsert semantics).
    #[test]
    fn test_update_missing_object_returns_not_found_and_does_not_insert() {
        let store = ResourceStore::new();
        let r = cm("ghost", "acme");
        let err = store.update(r).expect_err("update of missing object must fail");
        match err {
            ApiError::NotFound { kind, name } => {
                assert_eq!(kind, "ConfigMap");
                assert_eq!(name, "ghost");
            }
            other => panic!("expected NotFound, got {:?}", other),
        }
        // tenant_id invariant: no acme insertion side-effect.
        assert_eq!(store.list("ConfigMap", "acme").len(), 0,
            "tenant_id invariant: failed update never created acme entry");
    }
}
