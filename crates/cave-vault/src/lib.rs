pub mod api;
pub mod auth;
pub mod core;
pub mod engines;
pub mod error;
pub mod response;
pub mod token;

// ── Enterprise trait boundaries ───────────────────────────────────────────────
pub mod adapters;
pub mod backend;
pub mod factory;

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use axum::Router;

use crate::core::{AuditLogger, LeaseStore, PolicyStore, StorageBackend, WrapStore};
use crate::token::TokenStore;
use crate::engines::{kv1, kv2, transit, pki, database, aws, ssh, totp, cubbyhole, identity};
use crate::auth::{approle, cert, kubernetes, ldap, oidc, userpass};

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
            ("cubbyhole/", "cubbyhole", "per-token private secret storage"),
            ("identity/", "identity", "identity store"),
            ("sys/", "system", "system endpoints"),
        ] {
            default_mounts.insert(path.to_string(), MountEntry {
                path: path.to_string(),
                mount_type: mtype.to_string(),
                description: desc.to_string(),
                config: MountConfig::default(),
                local: false,
                seal_wrap: false,
                uuid: uuid::Uuid::new_v4().to_string(),
                accessor: uuid::Uuid::new_v4().to_string(),
            });
        }

        // Initialize default auth methods
        let mut default_auth = HashMap::new();
        for (path, atype, desc) in [
            ("token/", "token", "token based credentials"),
            ("approle/", "approle", "AppRole credentials"),
            ("userpass/", "userpass", "username and password credentials"),
        ] {
            default_auth.insert(path.to_string(), AuthEntry {
                path: path.to_string(),
                auth_type: atype.to_string(),
                description: desc.to_string(),
                config: MountConfig::default(),
                local: false,
                seal_wrap: false,
                uuid: uuid::Uuid::new_v4().to_string(),
                accessor: uuid::Uuid::new_v4().to_string(),
            });
        }

        Arc::new(VaultState {
            storage: Arc::new(RwLock::new(StorageBackend::default())),
            seal_state: Arc::new(RwLock::new(core::seal::SealState::default())),
            token_store: Arc::new(RwLock::new(TokenStore::default())),
            policy_store: Arc::new(RwLock::new(PolicyStore::new())),
            lease_store: Arc::new(RwLock::new(LeaseStore::default())),
            audit_logger: Arc::new(AuditLogger::new(hmac_key)),
            mount_table: Arc::new(RwLock::new(MountTable { mounts: default_mounts })),
            auth_table: Arc::new(RwLock::new(AuthTable { methods: default_auth })),
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
        .merge(engines::identity::router(state.clone()))
}

/// Enterprise secrets engine trait + built-in implementation.
pub use backend::{SecretsEngine, SecretsError, SecretsEngineProfile, SecretsResult, SecretValue, BuiltinSecretsEngine};

/// Factory: build the secrets engine from config/env.
pub use factory::{create_secrets_engine, create_secrets_engine_from_env};

pub const MODULE_NAME: &str = "vault";
