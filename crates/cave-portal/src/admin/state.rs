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
    pub recent_activity: RwLock<Vec<ActivityEntry>>,
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
            recent_activity: RwLock::new(Vec::new()),
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
        s.recent_activity.write().unwrap().extend([
            ActivityEntry { tenant: acme.clone(), when_unix: 1_000_100, kind: "deploy", summary: "deployed web v17".into() },
            ActivityEntry { tenant: acme, when_unix: 1_000_200, kind: "policy", summary: "updated AuthorizationPolicy allow-web".into() },
            ActivityEntry { tenant: evil, when_unix: 1_000_300, kind: "deploy", summary: "deployed evil-web v1".into() },
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
