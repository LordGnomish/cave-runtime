//! In-memory data sources backing the admin views.
//!
//! Real cave-runtime instances hand the views *connected* clients (an
//! `etcd::Client`, a `cri::Sandbox`, `kube::Api<DynamicObject>`, …). For
//! the parity scaffold we model the same shape with deterministic in-memory
//! seeded fixtures so the views can be unit-tested without a cluster.
//!
//! Multi-tenancy is enforced by filtering every collection on the request
//! context's tenant before returning anything. The fixtures intentionally
//! contain rows for *more than one* tenant so the cross-tenant tests can
//! observe that filter actually firing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::admin::types::TenantId;

// ── etcd ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtcdKv {
    pub tenant: TenantId,
    pub key: String,
    pub value: String,
    pub revision: u64,
    pub lease_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtcdLease {
    pub tenant: TenantId,
    pub lease_id: u64,
    pub ttl_seconds: u64,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EtcdEvent {
    Put { key: String, value: String, revision: u64 },
    Delete { key: String, revision: u64 },
}

// ── cri ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriSandbox {
    pub tenant: TenantId,
    pub sandbox_id: String,
    pub pod_name: String,
    pub state: &'static str, // "Ready" | "NotReady"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CriContainer {
    pub tenant: TenantId,
    pub sandbox_id: String,
    pub container_id: String,
    pub image: String,
    pub state: &'static str, // "Running" | "Exited" | "Created"
}

// ── apiserver ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct K8sResource {
    pub tenant: TenantId,
    pub kind: String,
    pub name: String,
    pub namespace: String,
}

// ── iam ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamUser {
    pub tenant: TenantId,
    pub username: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamRoleAssignment {
    pub tenant: TenantId,
    pub username: String,
    pub role: String,
}

// ── mesh ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshAuthzPolicy {
    pub tenant: TenantId,
    pub name: String,
    /// `Allow` or `Deny`.
    pub action: &'static str,
    pub principal_glob: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshFlow {
    pub tenant: TenantId,
    pub source: String,
    pub destination: String,
    pub verdict: &'static str, // "Forwarded" | "Dropped"
    pub bytes: u64,
}

// ── pg ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PgTable {
    pub tenant: TenantId,
    pub schema: String,
    pub name: String,
    pub row_count: u64,
}

// ── vault ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultSecretMeta {
    pub tenant: TenantId,
    pub path: String,
    pub version: u32,
    pub created_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultAuditEntry {
    pub tenant: TenantId,
    pub time_unix: i64,
    pub principal: String,
    pub op: &'static str, // "read-meta" | "read-value" | "write" | "delete"
    pub path: String,
}

// ── keda ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScaledObject {
    pub tenant: TenantId,
    pub name: String,
    /// Target Deployment / StatefulSet / Custom resource.
    pub target_ref: String,
    pub min_replicas: u32,
    pub max_replicas: u32,
    pub current_replicas: u32,
    pub paused: bool,
    /// Trigger types attached: `cpu`, `memory`, `kafka`, `prometheus`, `redis`, `cron`, `http`.
    pub triggers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KedaScalerEvent {
    pub tenant: TenantId,
    pub when_unix: i64,
    pub scaled_object: String,
    /// e.g. `kafka:lag=120`, `cpu:75`, `prometheus:queue_depth=900`.
    pub trigger: String,
    pub from_replicas: u32,
    pub to_replicas: u32,
    /// `Scaled` | `NoChange` | `FallbackActive`.
    pub verdict: &'static str,
}

// ── scheduler ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerNode {
    pub tenant: TenantId,
    pub name: String,
    pub ready: bool,
    pub allocatable_cpu_milli: u64,
    pub allocatable_mem_mib: u64,
    pub taints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerPolicy {
    pub tenant: TenantId,
    pub name: String,
    pub predicate: String,
    pub weight: u32,
}

// ── controller-manager ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerLease {
    pub tenant: TenantId,
    pub controller: String,
    pub leader_id: String,
    pub renewals: u64,
    pub expires_unix: i64,
}

// ── kubelet ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubeletPod {
    pub tenant: TenantId,
    pub node: String,
    pub pod_name: String,
    pub status: &'static str, // "Running" | "Pending" | "Failed"
    pub restart_count: u32,
}

// ── cloud-controller-manager ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudVolume {
    pub tenant: TenantId,
    pub id: String,
    pub region: String,
    pub size_gb: u64,
    pub attached_node: Option<String>,
}

// ── kamaji ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KamajiTcp {
    pub tenant: TenantId,
    pub name: String,
    pub k8s_version: String,
    pub ready_replicas: u32,
    pub desired_replicas: u32,
}

// ── net (Cilium) ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetEndpoint {
    pub tenant: TenantId,
    pub identity: u64,
    pub namespace: String,
    pub ip: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetPolicy {
    pub tenant: TenantId,
    pub name: String,
    pub direction: &'static str, // "Ingress" | "Egress" | "Both"
    pub selector: String,
}

// ── rdbms (postgres operator) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdbmsCluster {
    pub tenant: TenantId,
    pub name: String,
    pub version: String,
    pub replicas: u32,
    pub primary_node: String,
}

// ── docdb (mongo / yugabyte / etc.) ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocdbCollection {
    pub tenant: TenantId,
    pub database: String,
    pub collection: String,
    pub document_count: u64,
}

// ── cache (dragonfly / valkey) ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub tenant: TenantId,
    pub namespace: String,
    pub key: String,
    pub ttl_seconds: u64,
    pub size_bytes: u64,
}

// ── tenant dashboard recent activity ─────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub tenant: TenantId,
    pub when_unix: i64,
    pub kind: &'static str,
    pub summary: String,
}

// ── aggregate state ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct AdminState {
    pub etcd_kv: RwLock<Vec<EtcdKv>>,
    pub etcd_leases: RwLock<Vec<EtcdLease>>,
    pub etcd_event_log: RwLock<Vec<EtcdEvent>>,
    pub cri_sandboxes: RwLock<Vec<CriSandbox>>,
    pub cri_containers: RwLock<Vec<CriContainer>>,
    pub k8s_resources: RwLock<Vec<K8sResource>>,
    pub iam_users: RwLock<Vec<IamUser>>,
    pub iam_assignments: RwLock<Vec<IamRoleAssignment>>,
    pub mesh_authz: RwLock<Vec<MeshAuthzPolicy>>,
    pub mesh_flows: RwLock<Vec<MeshFlow>>,
    pub pg_tables: RwLock<Vec<PgTable>>,
    pub vault_secrets: RwLock<Vec<VaultSecretMeta>>,
    pub vault_audit: RwLock<Vec<VaultAuditEntry>>,
    pub keda_scaled_objects: RwLock<Vec<KedaScaledObject>>,
    pub keda_scaler_events: RwLock<Vec<KedaScalerEvent>>,
    pub recent_activity: RwLock<Vec<ActivityEntry>>,
    // 2026-05-10 batch.
    pub scheduler_nodes: RwLock<Vec<SchedulerNode>>,
    pub scheduler_policies: RwLock<Vec<SchedulerPolicy>>,
    pub controller_leases: RwLock<Vec<ControllerLease>>,
    pub kubelet_pods: RwLock<Vec<KubeletPod>>,
    pub cloud_volumes: RwLock<Vec<CloudVolume>>,
    pub kamaji_tcps: RwLock<Vec<KamajiTcp>>,
    pub net_endpoints: RwLock<Vec<NetEndpoint>>,
    pub net_policies: RwLock<Vec<NetPolicy>>,
    pub rdbms_clusters: RwLock<Vec<RdbmsCluster>>,
    pub docdb_collections: RwLock<Vec<DocdbCollection>>,
    pub cache_entries: RwLock<Vec<CacheEntry>>,
}

impl Default for AdminState {
    fn default() -> Self {
        Self::seeded()
    }
}

impl AdminState {
    /// Empty state — used by integration tests that want to push their
    /// own fixtures.
    pub fn empty() -> Self {
        Self {
            etcd_kv: RwLock::new(Vec::new()),
            etcd_leases: RwLock::new(Vec::new()),
            etcd_event_log: RwLock::new(Vec::new()),
            cri_sandboxes: RwLock::new(Vec::new()),
            cri_containers: RwLock::new(Vec::new()),
            k8s_resources: RwLock::new(Vec::new()),
            iam_users: RwLock::new(Vec::new()),
            iam_assignments: RwLock::new(Vec::new()),
            mesh_authz: RwLock::new(Vec::new()),
            mesh_flows: RwLock::new(Vec::new()),
            pg_tables: RwLock::new(Vec::new()),
            vault_secrets: RwLock::new(Vec::new()),
            vault_audit: RwLock::new(Vec::new()),
            keda_scaled_objects: RwLock::new(Vec::new()),
            keda_scaler_events: RwLock::new(Vec::new()),
            recent_activity: RwLock::new(Vec::new()),
            scheduler_nodes: RwLock::new(Vec::new()),
            scheduler_policies: RwLock::new(Vec::new()),
            controller_leases: RwLock::new(Vec::new()),
            kubelet_pods: RwLock::new(Vec::new()),
            cloud_volumes: RwLock::new(Vec::new()),
            kamaji_tcps: RwLock::new(Vec::new()),
            net_endpoints: RwLock::new(Vec::new()),
            net_policies: RwLock::new(Vec::new()),
            rdbms_clusters: RwLock::new(Vec::new()),
            docdb_collections: RwLock::new(Vec::new()),
            cache_entries: RwLock::new(Vec::new()),
        }
    }

    /// Seeded state — every collection has rows for `acme` *and* a foreign
    /// `evil` tenant so cross-tenant tests can verify the filter.
    pub fn seeded() -> Self {
        let s = Self::empty();
        let acme = TenantId::new("acme").expect("test fixture");
        let evil = TenantId::new("evil").expect("test fixture");
        s.etcd_kv.write().unwrap().extend([
            EtcdKv { tenant: acme.clone(), key: "/cfg/feature_x".into(), value: "on".into(), revision: 7, lease_id: None },
            EtcdKv { tenant: acme.clone(), key: "/state/leader".into(), value: "node-a".into(), revision: 8, lease_id: Some(1001) },
            EtcdKv { tenant: evil.clone(), key: "/cfg/feature_y".into(), value: "secret".into(), revision: 1, lease_id: None },
        ]);
        s.etcd_leases.write().unwrap().extend([
            EtcdLease { tenant: acme.clone(), lease_id: 1001, ttl_seconds: 30, keys: vec!["/state/leader".into()] },
            EtcdLease { tenant: evil.clone(), lease_id: 9999, ttl_seconds: 60, keys: vec!["/secret/x".into()] },
        ]);
        s.etcd_event_log.write().unwrap().extend([
            EtcdEvent::Put { key: "/cfg/feature_x".into(), value: "on".into(), revision: 7 },
            EtcdEvent::Put { key: "/state/leader".into(), value: "node-a".into(), revision: 8 },
        ]);
        s.cri_sandboxes.write().unwrap().extend([
            CriSandbox { tenant: acme.clone(), sandbox_id: "sb-1".into(), pod_name: "web-0".into(), state: "Ready" },
            CriSandbox { tenant: acme.clone(), sandbox_id: "sb-2".into(), pod_name: "api-0".into(), state: "Ready" },
            CriSandbox { tenant: evil.clone(), sandbox_id: "sb-evil".into(), pod_name: "x-0".into(), state: "Ready" },
        ]);
        s.cri_containers.write().unwrap().extend([
            CriContainer { tenant: acme.clone(), sandbox_id: "sb-1".into(), container_id: "c-1".into(), image: "nginx:1.27".into(), state: "Running" },
            CriContainer { tenant: acme.clone(), sandbox_id: "sb-2".into(), container_id: "c-2".into(), image: "api:v3".into(), state: "Running" },
        ]);
        s.k8s_resources.write().unwrap().extend([
            K8sResource { tenant: acme.clone(), kind: "Deployment".into(), name: "web".into(), namespace: "default".into() },
            K8sResource { tenant: acme.clone(), kind: "Service".into(), name: "web".into(), namespace: "default".into() },
            K8sResource { tenant: evil.clone(), kind: "Deployment".into(), name: "evil-web".into(), namespace: "default".into() },
        ]);
        s.iam_users.write().unwrap().extend([
            IamUser { tenant: acme.clone(), username: "alice".into(), email: "alice@acme".into() },
            IamUser { tenant: acme.clone(), username: "bob".into(), email: "bob@acme".into() },
            IamUser { tenant: evil.clone(), username: "mallory".into(), email: "m@evil".into() },
        ]);
        s.iam_assignments.write().unwrap().extend([
            IamRoleAssignment { tenant: acme.clone(), username: "alice".into(), role: "admin".into() },
            IamRoleAssignment { tenant: acme.clone(), username: "bob".into(), role: "viewer".into() },
        ]);
        s.mesh_authz.write().unwrap().extend([
            MeshAuthzPolicy { tenant: acme.clone(), name: "allow-web".into(), action: "Allow", principal_glob: "spiffe://*/ns/acme/sa/*".into() },
            MeshAuthzPolicy { tenant: evil.clone(), name: "evil-allow".into(), action: "Allow", principal_glob: "*".into() },
        ]);
        s.mesh_flows.write().unwrap().extend([
            MeshFlow { tenant: acme.clone(), source: "web".into(), destination: "api".into(), verdict: "Forwarded", bytes: 100 },
            MeshFlow { tenant: acme.clone(), source: "web".into(), destination: "api".into(), verdict: "Dropped", bytes: 0 },
        ]);
        s.pg_tables.write().unwrap().extend([
            PgTable { tenant: acme.clone(), schema: "public".into(), name: "users".into(), row_count: 1234 },
            PgTable { tenant: acme.clone(), schema: "public".into(), name: "orders".into(), row_count: 99 },
            PgTable { tenant: evil.clone(), schema: "public".into(), name: "secret".into(), row_count: 0 },
        ]);
        s.vault_secrets.write().unwrap().extend([
            VaultSecretMeta { tenant: acme.clone(), path: "kv/db".into(), version: 3, created_unix: 1_000_000 },
            VaultSecretMeta { tenant: acme.clone(), path: "kv/api".into(), version: 1, created_unix: 1_000_500 },
            VaultSecretMeta { tenant: evil.clone(), path: "kv/secret".into(), version: 7, created_unix: 999_999 },
        ]);
        s.vault_audit.write().unwrap().extend([
            VaultAuditEntry { tenant: acme.clone(), time_unix: 1_000_001, principal: "alice".into(), op: "read-meta", path: "kv/db".into() },
            VaultAuditEntry { tenant: acme.clone(), time_unix: 1_000_010, principal: "bob".into(), op: "read-meta", path: "kv/api".into() },
        ]);
        s.keda_scaled_objects.write().unwrap().extend([
            KedaScaledObject {
                tenant: acme.clone(),
                name: "ingest-worker".into(),
                target_ref: "Deployment/ingest-worker".into(),
                min_replicas: 1,
                max_replicas: 50,
                current_replicas: 8,
                paused: false,
                triggers: vec!["kafka".into(), "prometheus".into()],
            },
            KedaScaledObject {
                tenant: acme.clone(),
                name: "report-runner".into(),
                target_ref: "Deployment/report-runner".into(),
                min_replicas: 0,
                max_replicas: 10,
                current_replicas: 0,
                paused: true,
                triggers: vec!["cron".into()],
            },
            KedaScaledObject {
                tenant: evil.clone(),
                name: "evil-worker".into(),
                target_ref: "Deployment/evil-worker".into(),
                min_replicas: 1,
                max_replicas: 5,
                current_replicas: 1,
                paused: false,
                triggers: vec!["cpu".into()],
            },
        ]);
        s.keda_scaler_events.write().unwrap().extend([
            KedaScalerEvent {
                tenant: acme.clone(),
                when_unix: 1_000_400,
                scaled_object: "ingest-worker".into(),
                trigger: "kafka:lag=2400".into(),
                from_replicas: 4,
                to_replicas: 8,
                verdict: "Scaled",
            },
            KedaScalerEvent {
                tenant: acme.clone(),
                when_unix: 1_000_450,
                scaled_object: "ingest-worker".into(),
                trigger: "kafka:lag=2300".into(),
                from_replicas: 8,
                to_replicas: 8,
                verdict: "NoChange",
            },
            KedaScalerEvent {
                tenant: evil.clone(),
                when_unix: 1_000_460,
                scaled_object: "evil-worker".into(),
                trigger: "cpu:99".into(),
                from_replicas: 1,
                to_replicas: 5,
                verdict: "Scaled",
            },
        ]);
        s.recent_activity.write().unwrap().extend([
            ActivityEntry { tenant: acme.clone(), when_unix: 1_000_100, kind: "deploy", summary: "deployed web v17".into() },
            ActivityEntry { tenant: acme.clone(), when_unix: 1_000_200, kind: "policy", summary: "updated AuthorizationPolicy allow-web".into() },
            ActivityEntry { tenant: evil.clone(), when_unix: 1_000_300, kind: "deploy", summary: "deployed evil-web v1".into() },
        ]);
        s.scheduler_nodes.write().unwrap().extend([
            SchedulerNode { tenant: acme.clone(), name: "node-a".into(), ready: true, allocatable_cpu_milli: 8000, allocatable_mem_mib: 16384, taints: vec![] },
            SchedulerNode { tenant: acme.clone(), name: "node-b".into(), ready: false, allocatable_cpu_milli: 4000, allocatable_mem_mib: 8192, taints: vec!["NoSchedule=cordoned".into()] },
            SchedulerNode { tenant: evil.clone(), name: "evil-node".into(), ready: true, allocatable_cpu_milli: 1000, allocatable_mem_mib: 2048, taints: vec![] },
        ]);
        s.scheduler_policies.write().unwrap().extend([
            SchedulerPolicy { tenant: acme.clone(), name: "least-utilised".into(), predicate: "cpu<70".into(), weight: 5 },
            SchedulerPolicy { tenant: evil.clone(), name: "evil-pin".into(), predicate: "host==evil-node".into(), weight: 10 },
        ]);
        s.controller_leases.write().unwrap().extend([
            ControllerLease { tenant: acme.clone(), controller: "deployment".into(), leader_id: "ctl-1".into(), renewals: 17, expires_unix: 1_001_000 },
            ControllerLease { tenant: acme.clone(), controller: "replicaset".into(), leader_id: "ctl-1".into(), renewals: 18, expires_unix: 1_001_010 },
            ControllerLease { tenant: evil.clone(), controller: "evil-loop".into(), leader_id: "evil-1".into(), renewals: 1, expires_unix: 1_001_020 },
        ]);
        s.kubelet_pods.write().unwrap().extend([
            KubeletPod { tenant: acme.clone(), node: "node-a".into(), pod_name: "web-0".into(), status: "Running", restart_count: 0 },
            KubeletPod { tenant: acme.clone(), node: "node-a".into(), pod_name: "api-0".into(), status: "Running", restart_count: 1 },
            KubeletPod { tenant: acme.clone(), node: "node-b".into(), pod_name: "worker-0".into(), status: "Pending", restart_count: 0 },
            KubeletPod { tenant: evil.clone(), node: "evil-node".into(), pod_name: "x-0".into(), status: "Running", restart_count: 99 },
        ]);
        s.cloud_volumes.write().unwrap().extend([
            CloudVolume { tenant: acme.clone(), id: "vol-1".into(), region: "eu-central-1".into(), size_gb: 50, attached_node: Some("node-a".into()) },
            CloudVolume { tenant: acme.clone(), id: "vol-2".into(), region: "eu-central-1".into(), size_gb: 100, attached_node: None },
            CloudVolume { tenant: evil.clone(), id: "evil-vol".into(), region: "us-east-1".into(), size_gb: 1024, attached_node: None },
        ]);
        s.kamaji_tcps.write().unwrap().extend([
            KamajiTcp { tenant: acme.clone(), name: "tcp-prod".into(), k8s_version: "1.31.2".into(), ready_replicas: 3, desired_replicas: 3 },
            KamajiTcp { tenant: acme.clone(), name: "tcp-staging".into(), k8s_version: "1.31.0".into(), ready_replicas: 2, desired_replicas: 3 },
            KamajiTcp { tenant: evil.clone(), name: "evil-tcp".into(), k8s_version: "1.27.0".into(), ready_replicas: 1, desired_replicas: 1 },
        ]);
        s.net_endpoints.write().unwrap().extend([
            NetEndpoint { tenant: acme.clone(), identity: 1001, namespace: "default".into(), ip: "10.0.0.5".into(), ready: true },
            NetEndpoint { tenant: acme.clone(), identity: 1002, namespace: "default".into(), ip: "10.0.0.6".into(), ready: true },
            NetEndpoint { tenant: evil.clone(), identity: 9001, namespace: "default".into(), ip: "10.0.99.99".into(), ready: true },
        ]);
        s.net_policies.write().unwrap().extend([
            NetPolicy { tenant: acme.clone(), name: "allow-web".into(), direction: "Ingress", selector: "app=web".into() },
            NetPolicy { tenant: evil.clone(), name: "evil-allow-all".into(), direction: "Both", selector: "*".into() },
        ]);
        s.rdbms_clusters.write().unwrap().extend([
            RdbmsCluster { tenant: acme.clone(), name: "pg-prod".into(), version: "16.2".into(), replicas: 3, primary_node: "node-a".into() },
            RdbmsCluster { tenant: evil.clone(), name: "evil-pg".into(), version: "13.0".into(), replicas: 1, primary_node: "evil-node".into() },
        ]);
        s.docdb_collections.write().unwrap().extend([
            DocdbCollection { tenant: acme.clone(), database: "orders".into(), collection: "items".into(), document_count: 10_000 },
            DocdbCollection { tenant: acme.clone(), database: "orders".into(), collection: "ledger".into(), document_count: 250 },
            DocdbCollection { tenant: evil.clone(), database: "secrets".into(), collection: "tokens".into(), document_count: 5 },
        ]);
        s.cache_entries.write().unwrap().extend([
            CacheEntry { tenant: acme.clone(), namespace: "session".into(), key: "u-1".into(), ttl_seconds: 3600, size_bytes: 256 },
            CacheEntry { tenant: acme, namespace: "session".into(), key: "u-2".into(), ttl_seconds: 1800, size_bytes: 256 },
            CacheEntry { tenant: evil, namespace: "session".into(), key: "evil-1".into(), ttl_seconds: 60, size_bytes: 999 },
        ]);
        s
    }
}

/// Filter helper used by every view: returns only rows belonging to `tenant`.
pub fn scope<'a, T, F>(rows: &'a [T], tenant: &TenantId, f: F) -> Vec<&'a T>
where
    F: Fn(&T) -> &TenantId,
{
    rows.iter().filter(|r| f(r) == tenant).collect()
}

/// Tally helper used by tenant_dashboard.
pub fn tally_by_kind(rows: &[ActivityEntry], tenant: &TenantId) -> BTreeMap<&'static str, u64> {
    let mut out: BTreeMap<&'static str, u64> = BTreeMap::new();
    for r in rows.iter().filter(|r| &r.tenant == tenant) {
        *out.entry(r.kind).or_insert(0) += 1;
    }
    out
}
