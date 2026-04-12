//! CAVE Vault — HashiCorp Vault replacement.
//!
//! Replaces: HashiCorp Vault (KV v2, PKI, Transit, AppRole/K8s/OIDC auth)
//!
//! All state is held in an `Arc<Mutex<VaultStore>>` — no external dependencies
//! required for a development / CI environment.

pub mod auth;
pub mod kv;
pub mod models;
pub mod pki;
pub mod routes;
pub mod transit;

use axum::Router;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Shared in-memory state for the entire vault module.
pub struct VaultStore {
    /// KV secrets engine v2 storage: path → versioned entry
    pub kv: HashMap<String, kv::KVEntry>,
    /// PKI certificate store: serial → stored cert
    pub pki_certs: HashMap<String, pki::StoredCert>,
    /// Live rcgen root CA (needed to sign leaf certs)
    pub root_ca: Option<pki::CaState>,
    /// Revoked cert serials → revocation timestamp
    pub revoked_certs: HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Transit key store: key_name → all versions
    pub transit_keys: HashMap<String, transit::TransitKeyEntry>,
    /// Auth tokens: token_id → token info
    pub tokens: HashMap<String, auth::TokenInfo>,
    /// AppRole definitions: role_name → role config
    pub approles: HashMap<String, auth::AppRole>,
    /// Access control policies: policy_name → policy
    pub policies: HashMap<String, models::Policy>,
    /// Append-only audit log
    pub audit_log: Vec<models::AuditEntry>,
    /// Whether the vault is sealed (requests are rejected when true)
    pub sealed: bool,
    /// Whether the vault has been initialised
    pub initialized: bool,
    /// Stable cluster UUID
    pub cluster_id: String,
}

impl Default for VaultStore {
    fn default() -> Self {
        Self {
            kv: HashMap::new(),
            pki_certs: HashMap::new(),
            root_ca: None,
            revoked_certs: HashMap::new(),
            transit_keys: HashMap::new(),
            tokens: HashMap::new(),
            approles: HashMap::new(),
            policies: built_in_policies(),
            audit_log: Vec::new(),
            sealed: false,
            initialized: true,
            cluster_id: Uuid::new_v4().to_string(),
        }
    }
}

fn built_in_policies() -> HashMap<String, models::Policy> {
    let now = chrono::Utc::now();
    let mut map = HashMap::new();

    map.insert(
        "root".to_string(),
        models::Policy {
            name: "root".to_string(),
            rules: vec![models::PolicyRule {
                path: "*".to_string(),
                capabilities: vec![
                    models::PolicyCapability::Create,
                    models::PolicyCapability::Read,
                    models::PolicyCapability::Update,
                    models::PolicyCapability::Delete,
                    models::PolicyCapability::List,
                    models::PolicyCapability::Sudo,
                ],
            }],
            created_at: now,
            updated_at: now,
        },
    );

    map.insert(
        "default".to_string(),
        models::Policy {
            name: "default".to_string(),
            rules: vec![models::PolicyRule {
                path: "secret/data/*".to_string(),
                capabilities: vec![
                    models::PolicyCapability::Read,
                    models::PolicyCapability::List,
                ],
            }],
            created_at: now,
            updated_at: now,
        },
    );

    map
}

pub type SharedVaultStore = Arc<Mutex<VaultStore>>;

/// Create the axum router for the vault module.
pub fn router(store: SharedVaultStore) -> Router {
    routes::create_router(store)
}

pub const MODULE_NAME: &str = "vault";
