//! CAVE Vault — 100% HashiCorp Vault replacement.
//!
//! Provides: KV v1/v2, Transit (AES-256-GCM / Ed25519 / RSA-2048), PKI,
//! Database secrets, Token/UserPass/AppRole/OIDC auth, path-ACL policies,
//! leases, Shamir seal/unseal, and structured audit logging.

pub mod audit;
pub mod auth;
pub mod database;
pub mod error;
pub mod kv;
pub mod lease;
pub mod models;
pub mod pki;
pub mod policy;
pub mod routes;
pub mod shamir;
pub mod transit;

use axum::Router;
use std::sync::Arc;
use tokio::sync::RwLock;

use audit::AuditLog;
use auth::AuthEngine;
use database::DatabaseEngine;
use kv::{KVEntry, KVV1Entry};
use lease::LeaseStore;
use models::SealStatus;
use pki::PkiEngine;
use policy::PolicyEngine;
use base64::Engine as _;
use ring::rand::{SecureRandom, SystemRandom};
use std::collections::HashMap;
use transit::TransitKeyEntry;

// ── Seal config ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SealConfig {
    pub secret_shares: u8,
    pub secret_threshold: u8,
}

impl Default for SealConfig {
    fn default() -> Self {
        Self {
            secret_shares: 5,
            secret_threshold: 3,
        }
    }
}

// ── VaultStore ────────────────────────────────────────────────────────────────

/// Central in-memory vault state.
pub struct VaultStore {
    // KV stores
    pub kv_v2: HashMap<String, KVEntry>,
    pub kv_v1: HashMap<String, KVV1Entry>,

    // Transit engine
    pub transit: HashMap<String, TransitKeyEntry>,

    // PKI
    pub pki: PkiEngine,

    // Database secrets engine
    pub db: DatabaseEngine,

    // Auth
    pub auth: AuthEngine,

    // Policy
    pub policy: PolicyEngine,

    // Leases
    pub leases: LeaseStore,

    // Audit
    pub audit: AuditLog,

    // Seal / init state
    pub sealed: bool,
    pub initialized: bool,
    pub seal_config: SealConfig,
    pub unseal_buffer: Vec<Vec<u8>>,    // collected unseal shares
    pub master_key: Option<Vec<u8>>,    // in-memory only
    pub cluster_id: String,
}

impl VaultStore {
    /// Create a new, uninitialized (and sealed) vault store.
    pub fn new() -> Self {
        use uuid::Uuid;
        Self {
            kv_v2: HashMap::new(),
            kv_v1: HashMap::new(),
            transit: HashMap::new(),
            pki: PkiEngine::new(),
            db: DatabaseEngine::new(),
            auth: AuthEngine::new(),
            policy: PolicyEngine::new(),
            leases: LeaseStore::new(),
            audit: AuditLog::new(10_000),
            sealed: true,
            initialized: false,
            seal_config: SealConfig::default(),
            unseal_buffer: Vec::new(),
            master_key: None,
            cluster_id: Uuid::new_v4().to_string(),
        }
    }

    /// Create a pre-initialized, unsealed store for testing and dev mode.
    pub fn dev() -> Self {
        let mut store = Self::new();
        // Generate a random master key
        let rng = SystemRandom::new();
        let mut key = vec![0u8; 32];
        rng.fill(&mut key).expect("rng fill");
        store.master_key = Some(key);
        store.initialized = true;
        store.sealed = false;
        store
    }

    pub fn seal_status(&self) -> SealStatus {
        SealStatus {
            sealed: self.sealed,
            initialized: self.initialized,
            t: self.seal_config.secret_threshold,
            n: self.seal_config.secret_shares,
            progress: self.unseal_buffer.len() as u8,
            cluster_id: self.cluster_id.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Initialize the vault: generate master key, split with Shamir.
    /// Returns (key_shares, root_token).
    pub fn initialize(
        &mut self,
        secret_shares: u8,
        secret_threshold: u8,
    ) -> Result<(Vec<String>, String), error::VaultError> {
        if self.initialized {
            return Err(error::VaultError::AlreadyInitialized);
        }
        let rng = SystemRandom::new();
        let mut master_key = vec![0u8; 32];
        rng.fill(&mut master_key)
            .map_err(|_| error::VaultError::CryptoError("master key gen".into()))?;

        let shares = shamir::split(&master_key, secret_threshold, secret_shares);
        let b64_shares: Vec<String> = shares
            .iter()
            .map(|s| base64::engine::general_purpose::STANDARD.encode(s))
            .collect();

        self.seal_config = SealConfig {
            secret_shares,
            secret_threshold,
        };
        self.master_key = Some(master_key);
        self.initialized = true;
        self.sealed = true; // still sealed until threshold shares provided

        let root_token = self.auth.root_token().to_string();
        Ok((b64_shares, root_token))
    }

    /// Provide one unseal share. Returns true when fully unsealed.
    pub fn unseal(&mut self, share_b64: &str) -> Result<bool, error::VaultError> {
        if !self.initialized {
            return Err(error::VaultError::NotInitialized);
        }
        if !self.sealed {
            return Ok(true);
        }

        let share = base64::engine::general_purpose::STANDARD
            .decode(share_b64)
            .map_err(|_| error::VaultError::InvalidRequest("invalid base64 share".into()))?;

        self.unseal_buffer.push(share);

        if self.unseal_buffer.len() >= self.seal_config.secret_threshold as usize {
            // Try to reconstruct master key
            let reconstructed = shamir::combine(&self.unseal_buffer);
            // Verify by comparing with stored master key
            if self.master_key.as_deref() == Some(&reconstructed) {
                self.sealed = false;
                self.unseal_buffer.clear();
                return Ok(true);
            } else {
                // Wrong shares — clear buffer and return error
                self.unseal_buffer.clear();
                return Err(error::VaultError::InvalidRequest(
                    "unseal failed: shares did not reconstruct the master key".into(),
                ));
            }
        }

        Ok(false)
    }

    pub fn seal(&mut self) {
        self.sealed = true;
        self.unseal_buffer.clear();
        // Do NOT clear master_key — it needs to survive seal/unseal cycles
    }

    /// Validate a request token and return the caller's policies.
    pub fn authenticate(&self, token: &str) -> Result<Vec<String>, error::VaultError> {
        if self.sealed {
            return Err(error::VaultError::Sealed);
        }
        let info = self.auth.lookup_token(token)?;
        Ok(info.policies.clone())
    }
}

impl Default for VaultStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Shared state type ─────────────────────────────────────────────────────────

pub type SharedVaultState = Arc<RwLock<VaultStore>>;

pub fn new_shared_state() -> SharedVaultState {
    Arc::new(RwLock::new(VaultStore::dev()))
}

pub const MODULE_NAME: &str = "vault";

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: SharedVaultState) -> Router {
    routes::create_router(state)
}
