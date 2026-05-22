// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
pub mod api;
pub mod auth;
pub mod core;
pub mod engines;
pub mod error;
pub mod response;
pub mod storage;
pub mod token;

// Earlier-generation top-level modules — the newer subdir versions
// (`core::*`, `engines::*`) are the active path. These four expose unit
// tests that previously didn't run; they're kept compileable but are not
// the canonical surface.
//
// Excluded (intentionally left orphan):
//   * `transit`, `pki`, `database`  — re-exported under `engines::` already
//     (would collide as top-level mods)
//   * `routes`                      — references removed `SharedVaultStore` /
//                                     `VaultStore` / `TransitStore` symbols
//   * `kv`, `audit`, `models`       — zero unit tests, no activation value
pub mod lease;
pub mod policy;
pub mod shamir;

use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::auth::{approle, cert, kubernetes, ldap, oidc, userpass};
use crate::core::{AuditLogger, LeaseStore, PolicyStore, StorageBackend, WrapStore};
use crate::engines::{aws, cubbyhole, database, identity, kv1, kv2, pki, ssh, totp, transit};
use crate::token::TokenStore;

pub struct VaultState {
    // Core
    pub storage: Arc<RwLock<StorageBackend>>,
    pub seal_state: Arc<RwLock<core::seal::SealState>>,
    pub token_store: Arc<RwLock<TokenStore>>,
    pub policy_store: Arc<RwLock<PolicyStore>>,
    pub lease_store: Arc<RwLock<LeaseStore>>,
    pub audit_logger: Arc<AuditLogger>,
    pub mount_table: Arc<RwLock<MountTable>>,
    pub auth_table: Arc<RwLock<AuthTable>>,
    pub wrap_store: Arc<RwLock<WrapStore>>,
    // Engines
    pub kv1_store: Arc<RwLock<kv1::Kv1Store>>,
    pub kv2_store: Arc<RwLock<kv2::Kv2Store>>,
    pub transit_store: Arc<RwLock<transit::TransitStore>>,
    pub pki_store: Arc<RwLock<pki::PkiStore>>,
    pub database_store: Arc<RwLock<database::DatabaseStore>>,
    pub aws_store: Arc<RwLock<aws::AwsStore>>,
    pub ssh_store: Arc<RwLock<ssh::SshStore>>,
    pub totp_store: Arc<RwLock<totp::TotpStore>>,
    pub cubbyhole_store: Arc<RwLock<cubbyhole::CubbyholeStore>>,
    pub identity_store: Arc<RwLock<identity::IdentityStore>>,
    // Auth method state
    pub approle_store: Arc<RwLock<approle::ApproleStore>>,
    pub userpass_store: Arc<RwLock<userpass::UserpassStore>>,
    pub kubernetes_store: Arc<RwLock<kubernetes::KubernetesStore>>,
    pub ldap_store: Arc<RwLock<ldap::LdapStore>>,
    pub oidc_store: Arc<RwLock<oidc::OidcStore>>,
    pub cert_store: Arc<RwLock<cert::CertStore>>,
    pub namespace_store: Arc<RwLock<NamespaceStore>>,
}

#[derive(Default)]
pub struct MountTable {
    pub mounts: HashMap<String, MountEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MountEntry {
    pub path: String,
    pub mount_type: String,
    pub description: String,
    pub config: MountConfig,
    pub local: bool,
    pub seal_wrap: bool,
    pub uuid: String,
    pub accessor: String,
    /// Multi-tenant scoping: which namespace this mount belongs to. Empty = root.
    /// See openbao `helper/namespace/namespace.go:40` (Namespace) and
    /// `vault/mount.go:506` (MountEntry.Namespace).
    #[serde(default)]
    pub namespace_id: String,
}

impl MountTable {
    /// Insert a mount entry. Mirrors `vault/mount.go:1705` (persistMounts) write path.
    pub fn register(&mut self, entry: MountEntry) {
        self.mounts.insert(entry.path.clone(), entry);
    }

    /// Look up a mount entry by exact path. Mirrors openbao
    /// `vault/mount.go:320` (MountTable.findByPath).
    pub fn lookup(&self, path: &str) -> Option<&MountEntry> {
        self.mounts.get(path)
    }

    /// Walk every mount whose path is a prefix of the given request path,
    /// returning the longest match. Mirrors openbao
    /// `vault/mount.go:344` (MountTable.find) — predicate-based linear scan.
    pub fn longest_prefix(&self, req_path: &str) -> Option<&MountEntry> {
        let mut best: Option<&MountEntry> = None;
        for entry in self.mounts.values() {
            if req_path.starts_with(&entry.path) {
                let len = entry.path.len();
                if best.map_or(true, |b| b.path.len() < len) {
                    best = Some(entry);
                }
            }
        }
        best
    }

    /// Remove a mount entry by exact path. Mirrors openbao
    /// `vault/mount.go:302` (MountTable.remove).
    pub fn unregister(&mut self, path: &str) -> Option<MountEntry> {
        self.mounts.remove(path)
    }

    /// Sorted list of mount paths. Mirrors openbao
    /// `vault/mount.go:361` (MountTable.sortEntriesByPath).
    pub fn list(&self) -> Vec<String> {
        let mut paths: Vec<String> = self.mounts.keys().cloned().collect();
        paths.sort();
        paths
    }

    /// All mounts within a single namespace (tenant). Mirrors openbao
    /// `vault/mount.go:328` (MountTable.findAllNamespaceMounts).
    pub fn for_namespace(&self, ns_id: &str) -> Vec<&MountEntry> {
        self.mounts
            .values()
            .filter(|e| e.namespace_id == ns_id)
            .collect()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct MountConfig {
    pub default_lease_ttl: i64,
    pub max_lease_ttl: i64,
    pub force_no_cache: bool,
    pub token_type: String,
}

#[derive(Default)]
pub struct AuthTable {
    pub methods: HashMap<String, AuthEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthEntry {
    pub path: String,
    pub auth_type: String,
    pub description: String,
    pub config: MountConfig,
    pub local: bool,
    pub seal_wrap: bool,
    pub uuid: String,
    pub accessor: String,
}

#[derive(Default)]
pub struct NamespaceStore {
    pub namespaces: HashMap<String, Namespace>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Namespace {
    pub id: String,
    pub path: String,
    pub metadata: HashMap<String, String>,
    /// Cave extension: external tenant identifier. A single tenant may own
    /// multiple namespaces (e.g. prod / staging) and a namespace belongs to
    /// exactly one tenant.  See openbao `helper/namespace/namespace.go:40`
    /// (Namespace) which exposes ID/Path/CustomMetadata; `tenant_id` is the
    /// cave-runtime multi-tenancy correlation key.
    #[serde(default)]
    pub tenant_id: String,
}

impl Namespace {
    /// Build a namespace bound to a tenant. Path is canonicalised the same way
    /// as openbao `helper/namespace/namespace.go:259` (Canonicalize) — a
    /// trailing `/` is enforced for non-empty paths.
    pub fn new(
        id: impl Into<String>,
        path: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> Self {
        let mut p: String = path.into();
        if !p.is_empty() && !p.ends_with('/') {
            p.push('/');
        }
        Self {
            id: id.into(),
            path: p,
            metadata: HashMap::new(),
            tenant_id: tenant_id.into(),
        }
    }

    /// Validate the namespace path. Mirrors openbao
    /// `helper/namespace/namespace.go:54` (Namespace.Validate) — reject
    /// reserved prefixes and forbid the literal `root` name.
    pub fn validate(&self) -> Result<(), String> {
        const RESERVED: &[&str] = &["sys/", "audit/", "auth/", "cubbyhole/", "identity/"];
        if self.path == "root" || self.path == "root/" {
            return Err("namespace path cannot be 'root'".into());
        }
        for r in RESERVED {
            if self.path == *r {
                return Err(format!("namespace path '{}' is reserved", r));
            }
        }
        Ok(())
    }
}

impl NamespaceStore {
    /// Insert/update a namespace. Mirrors openbao
    /// `helper/namespace/namespace.go` mutation site — the store is keyed by
    /// the namespace ID, not the path.
    pub fn create(&mut self, ns: Namespace) -> Result<(), String> {
        ns.validate()?;
        self.namespaces.insert(ns.id.clone(), ns);
        Ok(())
    }

    /// Lookup by ID. Mirrors openbao `helper/namespace/namespace.go:220`
    /// (FromContext) which resolves the active namespace from a context tag.
    pub fn get(&self, id: &str) -> Option<&Namespace> {
        self.namespaces.get(id)
    }

    /// Lookup by canonical path. Mirrors openbao
    /// `helper/namespace/namespace.go:259` (Canonicalize) — callers are
    /// expected to canonicalise before lookup.
    pub fn get_by_path(&self, path: &str) -> Option<&Namespace> {
        let mut needle: String = path.to_string();
        if !needle.is_empty() && !needle.ends_with('/') {
            needle.push('/');
        }
        self.namespaces.values().find(|n| n.path == needle)
    }

    /// All namespaces owned by a tenant. Cave extension on top of openbao
    /// `helper/namespace/namespace.go:40` (Namespace).
    pub fn for_tenant(&self, tenant_id: &str) -> Vec<&Namespace> {
        let mut out: Vec<&Namespace> = self
            .namespaces
            .values()
            .filter(|n| n.tenant_id == tenant_id)
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Delete a namespace by ID. Returns whether it existed.
    pub fn delete(&mut self, id: &str) -> bool {
        self.namespaces.remove(id).is_some()
    }
}

impl VaultState {
    pub fn new() -> Arc<Self> {
        let rng = ring::rand::SystemRandom::new();
        let mut hmac_key = vec![0u8; 32];
        let _ = ring::rand::SecureRandom::fill(&rng, &mut hmac_key);

        // Initialize default mounts
        let mut default_mounts = HashMap::new();
        for (path, mtype, desc) in [
            ("secret/", "kv", "key/value secret storage"),
            (
                "cubbyhole/",
                "cubbyhole",
                "per-token private secret storage",
            ),
            ("identity/", "identity", "identity store"),
            ("sys/", "system", "system endpoints"),
        ] {
            default_mounts.insert(
                path.to_string(),
                MountEntry {
                    path: path.to_string(),
                    mount_type: mtype.to_string(),
                    description: desc.to_string(),
                    config: MountConfig::default(),
                    local: false,
                    seal_wrap: false,
                    uuid: uuid::Uuid::new_v4().to_string(),
                    accessor: uuid::Uuid::new_v4().to_string(),
                    namespace_id: String::new(),
                },
            );
        }

        // Initialize default auth methods
        let mut default_auth = HashMap::new();
        for (path, atype, desc) in [
            ("token/", "token", "token based credentials"),
            ("approle/", "approle", "AppRole credentials"),
            ("userpass/", "userpass", "username and password credentials"),
        ] {
            default_auth.insert(
                path.to_string(),
                AuthEntry {
                    path: path.to_string(),
                    auth_type: atype.to_string(),
                    description: desc.to_string(),
                    config: MountConfig::default(),
                    local: false,
                    seal_wrap: false,
                    uuid: uuid::Uuid::new_v4().to_string(),
                    accessor: uuid::Uuid::new_v4().to_string(),
                },
            );
        }

        Arc::new(VaultState {
            storage: Arc::new(RwLock::new(StorageBackend::default())),
            seal_state: Arc::new(RwLock::new(core::seal::SealState::default())),
            token_store: Arc::new(RwLock::new(TokenStore::default())),
            policy_store: Arc::new(RwLock::new(PolicyStore::new())),
            lease_store: Arc::new(RwLock::new(LeaseStore::default())),
            audit_logger: Arc::new(AuditLogger::new(hmac_key)),
            mount_table: Arc::new(RwLock::new(MountTable {
                mounts: default_mounts,
            })),
            auth_table: Arc::new(RwLock::new(AuthTable {
                methods: default_auth,
            })),
            wrap_store: Arc::new(RwLock::new(WrapStore::default())),
            kv1_store: Arc::new(RwLock::new(kv1::Kv1Store::default())),
            kv2_store: Arc::new(RwLock::new(kv2::Kv2Store::default())),
            transit_store: Arc::new(RwLock::new(transit::TransitStore::default())),
            pki_store: Arc::new(RwLock::new(pki::PkiStore::default())),
            database_store: Arc::new(RwLock::new(database::DatabaseStore::default())),
            aws_store: Arc::new(RwLock::new(aws::AwsStore::default())),
            ssh_store: Arc::new(RwLock::new(ssh::SshStore::default())),
            totp_store: Arc::new(RwLock::new(totp::TotpStore::default())),
            cubbyhole_store: Arc::new(RwLock::new(cubbyhole::CubbyholeStore::default())),
            identity_store: Arc::new(RwLock::new(identity::IdentityStore::default())),
            approle_store: Arc::new(RwLock::new(approle::ApproleStore::default())),
            userpass_store: Arc::new(RwLock::new(userpass::UserpassStore::default())),
            kubernetes_store: Arc::new(RwLock::new(kubernetes::KubernetesStore::default())),
            ldap_store: Arc::new(RwLock::new(ldap::LdapStore::default())),
            oidc_store: Arc::new(RwLock::new(oidc::OidcStore::default())),
            cert_store: Arc::new(RwLock::new(cert::CertStore::default())),
            namespace_store: Arc::new(RwLock::new(NamespaceStore::default())),
        })
    }
}

pub fn router(state: Arc<VaultState>) -> Router {
    Router::new()
        .merge(api::sys::router(state.clone()))
        .merge(auth::token::router(state.clone()))
        .merge(auth::approle::router(state.clone()))
        .merge(auth::userpass::router(state.clone()))
        .merge(auth::kubernetes::router(state.clone()))
        .merge(auth::ldap::router(state.clone()))
        .merge(auth::oidc::router(state.clone()))
        .merge(auth::cert::router(state.clone()))
        .merge(engines::kv1::router(state.clone(), "secret"))
        .merge(engines::kv2::router(state.clone(), "kv"))
        .merge(engines::transit::router(state.clone(), "transit"))
        .merge(engines::pki::router(state.clone(), "pki"))
        .merge(engines::database::router(state.clone(), "database"))
        .merge(engines::aws::router(state.clone(), "aws"))
        .merge(engines::ssh::router(state.clone(), "ssh"))
        .merge(engines::totp::router(state.clone(), "totp"))
        .merge(engines::cubbyhole::router(state.clone()))
        .merge(engines::identity::router(state))
}

pub const MODULE_NAME: &str = "vault";

#[cfg(test)]
mod lib_tests {
    use super::*;

    fn make_entry(path: &str, ns: &str) -> MountEntry {
        MountEntry {
            path: path.to_string(),
            mount_type: "kv".to_string(),
            description: String::new(),
            config: MountConfig::default(),
            local: false,
            seal_wrap: false,
            uuid: uuid::Uuid::new_v4().to_string(),
            accessor: uuid::Uuid::new_v4().to_string(),
            namespace_id: ns.to_string(),
        }
    }

    #[test]
    fn test_mount_table_register_lookup() {
        let mut t = MountTable::default();
        t.register(make_entry("secret/", ""));
        assert!(t.lookup("secret/").is_some());
        assert!(t.lookup("notmounted/").is_none());
    }

    #[test]
    fn test_mount_table_longest_prefix_picks_specific() {
        let mut t = MountTable::default();
        t.register(make_entry("kv/", ""));
        t.register(make_entry("kv/special/", ""));
        let m = t.longest_prefix("kv/special/foo").unwrap();
        assert_eq!(m.path, "kv/special/");
    }

    #[test]
    fn test_mount_table_unregister_removes() {
        let mut t = MountTable::default();
        t.register(make_entry("kv/", ""));
        let removed = t.unregister("kv/").unwrap();
        assert_eq!(removed.path, "kv/");
        assert!(t.lookup("kv/").is_none());
    }

    #[test]
    fn test_mount_table_list_sorted() {
        let mut t = MountTable::default();
        t.register(make_entry("zeta/", ""));
        t.register(make_entry("alpha/", ""));
        t.register(make_entry("beta/", ""));
        let list = t.list();
        assert_eq!(list, vec!["alpha/", "beta/", "zeta/"]);
    }

    #[test]
    fn test_mount_table_for_namespace_filter() {
        let mut t = MountTable::default();
        t.register(make_entry("a/", "ns-1"));
        t.register(make_entry("b/", "ns-1"));
        t.register(make_entry("c/", "ns-2"));
        assert_eq!(t.for_namespace("ns-1").len(), 2);
        assert_eq!(t.for_namespace("ns-2").len(), 1);
        assert_eq!(t.for_namespace("nope").len(), 0);
    }

    #[test]
    fn test_namespace_new_canonicalises_path() {
        let ns = Namespace::new("id-1", "tenant-a", "t-1");
        assert_eq!(ns.path, "tenant-a/");
        let ns2 = Namespace::new("id-2", "tenant-b/", "t-1");
        assert_eq!(ns2.path, "tenant-b/");
        let ns_empty = Namespace::new("id-3", "", "t-1");
        assert_eq!(ns_empty.path, "");
    }

    #[test]
    fn test_namespace_validate_rejects_reserved() {
        let bad = Namespace::new("id", "sys/", "t-1");
        assert!(bad.validate().is_err());
        let bad2 = Namespace::new("id", "auth/", "t-1");
        assert!(bad2.validate().is_err());
        let bad3 = Namespace::new("id", "root", "t-1");
        assert!(bad3.validate().is_err());
        let ok = Namespace::new("id", "team-a/", "t-1");
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn test_namespace_store_create_get() {
        let mut s = NamespaceStore::default();
        let ns = Namespace::new("ns-1", "tenant-a", "t-1");
        s.create(ns).unwrap();
        assert!(s.get("ns-1").is_some());
        assert!(s.get_by_path("tenant-a").is_some());
        assert!(s.get_by_path("tenant-a/").is_some()); // canonicalised either way
    }

    #[test]
    fn test_namespace_store_for_tenant_sorted() {
        let mut s = NamespaceStore::default();
        s.create(Namespace::new("ns-z", "z-team", "tenant-1"))
            .unwrap();
        s.create(Namespace::new("ns-a", "a-team", "tenant-1"))
            .unwrap();
        s.create(Namespace::new("ns-other", "x", "tenant-2"))
            .unwrap();
        let listed = s.for_tenant("tenant-1");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].path, "a-team/");
        assert_eq!(listed[1].path, "z-team/");
    }

    #[test]
    fn test_namespace_store_delete() {
        let mut s = NamespaceStore::default();
        s.create(Namespace::new("ns-1", "team", "t-1")).unwrap();
        assert!(s.delete("ns-1"));
        assert!(!s.delete("ns-1"));
        assert!(s.get("ns-1").is_none());
    }

    #[test]
    fn test_namespace_store_create_validates_reserved() {
        let mut s = NamespaceStore::default();
        let bad = Namespace::new("ns-x", "sys/", "t-1");
        assert!(s.create(bad).is_err());
    }
}
