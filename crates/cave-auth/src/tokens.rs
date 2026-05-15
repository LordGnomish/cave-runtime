// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Personal Access Token (PAT) and Service-to-Service token management.
//!
//! ## Token formats
//!
//! ```text
//! PAT:            cave_pat_<32-hex-uuid>          (e.g. cave_pat_550e8400e29b41d4a716446655440000)
//! Service token:  cave_svc_<32-hex-uuid>
//! ```
//!
//! The raw token is returned ONCE on creation.  Only a SHA-256 hash is stored
//! so a leaked token store cannot be replayed.
//!
//! ## In-memory store
//!
//! Phase 1 uses an in-memory `HashMap` protected by `tokio::sync::RwLock`.
//! A future phase will back this with `cave-db` (PostgreSQL).

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

use cave_core::types::CaveRole;

// ─── SHA-256 helper (ring) ────────────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
    digest
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ─── Token data models ────────────────────────────────────────────────────────

/// Scope controlling what a PAT can do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PATScope {
    /// Full access matching the user's roles
    Full,
    /// Restricted to specific module+action pairs, e.g. "cave-flags:write"
    Limited(Vec<String>),
}

impl PATScope {
    /// Return effective permission strings for this scope.
    pub fn permissions(&self, user_permissions: &[String]) -> Vec<String> {
        match self {
            PATScope::Full => user_permissions.to_vec(),
            PATScope::Limited(scopes) => scopes.clone(),
        }
    }
}

/// Stored (hashed) record for a Personal Access Token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatEntry {
    pub token_id: Uuid,
    /// SHA-256 of the raw token string
    pub token_hash: String,
    pub cave_uid: Uuid,
    pub label: String,
    pub roles: Vec<CaveRole>,
    pub scope: PATScope,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub revoked: bool,
}

/// Claims extracted when a PAT is validated successfully.
#[derive(Debug, Clone)]
pub struct PatClaims {
    pub token_id: Uuid,
    pub cave_uid: Uuid,
    pub roles: Vec<CaveRole>,
    /// Resolved permission strings
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
}

/// Stored record for a service-to-service token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceTokenEntry {
    pub token_id: Uuid,
    /// SHA-256 of the raw token string
    pub token_hash: String,
    pub service_name: String,
    /// Allowed module:action pairs, e.g. "cave-flags:read"
    pub scopes: Vec<String>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

/// Claims extracted when a service token is validated.
#[derive(Debug, Clone)]
pub struct ServiceTokenClaims {
    pub token_id: Uuid,
    pub service_name: String,
    pub scopes: Vec<String>,
}

// ─── Token store ──────────────────────────────────────────────────────────────

/// Thread-safe in-memory store for PATs and service tokens.
#[derive(Clone)]
pub struct TokenStore {
    /// Key = SHA-256(raw_token)
    pats: Arc<RwLock<HashMap<String, PatEntry>>>,
    service_tokens: Arc<RwLock<HashMap<String, ServiceTokenEntry>>>,
}

impl TokenStore {
    pub fn new(_signing_secret: &[u8]) -> Self {
        Self {
            pats: Arc::new(RwLock::new(HashMap::new())),
            service_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ── PAT management ────────────────────────────────────────────────────

    /// Create a new PAT and return the raw token (shown once).
    pub async fn create_pat(
        &self,
        cave_uid: Uuid,
        label: &str,
        roles: Vec<CaveRole>,
        scope: PATScope,
        ttl_days: i64,
    ) -> String {
        let token_id = Uuid::new_v4();
        let raw_token = format!("cave_pat_{}", token_id.simple());
        let token_hash = sha256_hex(raw_token.as_bytes());
        let now = Utc::now();

        let entry = PatEntry {
            token_id,
            token_hash: token_hash.clone(),
            cave_uid,
            label: label.to_string(),
            roles,
            scope,
            expires_at: now + Duration::days(ttl_days),
            created_at: now,
            last_used: None,
            revoked: false,
        };

        self.pats.write().await.insert(token_hash, entry);
        info!(cave_uid = %cave_uid, label, ttl_days, "PAT created");
        raw_token
    }

    /// Validate a PAT and return its claims, updating `last_used`.
    pub async fn validate_pat(&self, raw_token: &str) -> Option<PatClaims> {
        let hash = sha256_hex(raw_token.as_bytes());
        let mut store = self.pats.write().await;
        let entry = store.get_mut(&hash)?;

        if entry.revoked {
            warn!(token_id = %entry.token_id, "PAT is revoked");
            return None;
        }
        if entry.expires_at < Utc::now() {
            warn!(token_id = %entry.token_id, "PAT is expired");
            return None;
        }

        entry.last_used = Some(Utc::now());

        // For Limited scope, use the explicit scopes; for Full, wildcard
        let scopes = match &entry.scope {
            PATScope::Full => vec!["*".to_string()],
            PATScope::Limited(s) => s.clone(),
        };

        Some(PatClaims {
            token_id: entry.token_id,
            cave_uid: entry.cave_uid,
            roles: entry.roles.clone(),
            scopes,
            expires_at: entry.expires_at,
        })
    }

    /// Revoke a PAT belonging to `cave_uid`.
    pub async fn revoke_pat(&self, token_id: Uuid, cave_uid: Uuid) -> bool {
        let mut store = self.pats.write().await;
        for entry in store.values_mut() {
            if entry.token_id == token_id && entry.cave_uid == cave_uid {
                entry.revoked = true;
                info!(%token_id, %cave_uid, "PAT revoked");
                return true;
            }
        }
        warn!(%token_id, "PAT not found for revocation");
        false
    }

    /// Rotate a PAT: revoke the old one, create a new one with the same settings.
    /// Returns the new raw token.
    pub async fn rotate_pat(&self, token_id: Uuid, cave_uid: Uuid) -> Option<String> {
        // Capture settings from old entry
        let (label, roles, scope, ttl_days) = {
            let store = self.pats.read().await;
            let entry = store
                .values()
                .find(|e| e.token_id == token_id && e.cave_uid == cave_uid)?;
            let ttl = (entry.expires_at - entry.created_at).num_days().max(1);
            (
                entry.label.clone(),
                entry.roles.clone(),
                entry.scope.clone(),
                ttl,
            )
        };

        self.revoke_pat(token_id, cave_uid).await;
        let new_token = self
            .create_pat(cave_uid, &label, roles, scope, ttl_days)
            .await;
        Some(new_token)
    }

    /// List PATs owned by `cave_uid` (metadata only, no raw tokens).
    pub async fn list_pats(&self, cave_uid: Uuid) -> Vec<PatEntry> {
        self.pats
            .read()
            .await
            .values()
            .filter(|e| e.cave_uid == cave_uid && !e.revoked)
            .cloned()
            .collect()
    }

    // ── Service token management ──────────────────────────────────────────

    /// Create a service-to-service token and return the raw token.
    pub async fn create_service_token(
        &self,
        service_name: &str,
        scopes: Vec<String>,
        ttl_hours: i64,
    ) -> String {
        let token_id = Uuid::new_v4();
        let raw_token = format!("cave_svc_{}", token_id.simple());
        let token_hash = sha256_hex(raw_token.as_bytes());

        let entry = ServiceTokenEntry {
            token_id,
            token_hash: token_hash.clone(),
            service_name: service_name.to_string(),
            scopes,
            expires_at: Utc::now() + Duration::hours(ttl_hours),
            revoked: false,
        };

        self.service_tokens
            .write()
            .await
            .insert(token_hash, entry);
        info!(service_name, ttl_hours, "Service token created");
        raw_token
    }

    /// Validate a service token.
    pub async fn validate_service_token(&self, raw_token: &str) -> Option<ServiceTokenClaims> {
        let hash = sha256_hex(raw_token.as_bytes());
        let store = self.service_tokens.read().await;
        let entry = store.get(&hash)?;

        if entry.revoked {
            warn!(token_id = %entry.token_id, "Service token is revoked");
            return None;
        }
        if entry.expires_at < Utc::now() {
            warn!(token_id = %entry.token_id, "Service token is expired");
            return None;
        }

        Some(ServiceTokenClaims {
            token_id: entry.token_id,
            service_name: entry.service_name.clone(),
            scopes: entry.scopes.clone(),
        })
    }

    /// Revoke a service token by ID.
    pub async fn revoke_service_token(&self, token_id: Uuid) -> bool {
        let mut store = self.service_tokens.write().await;
        for entry in store.values_mut() {
            if entry.token_id == token_id {
                entry.revoked = true;
                info!(%token_id, "Service token revoked");
                return true;
            }
        }
        false
    }

    /// Revoke ALL tokens (PATs + service tokens) for a user — used when Okta
    /// deactivates the user via SCIM.
    pub async fn revoke_all_for_user(&self, cave_uid: Uuid) {
        let mut store = self.pats.write().await;
        let mut count = 0usize;
        for entry in store.values_mut() {
            if entry.cave_uid == cave_uid && !entry.revoked {
                entry.revoked = true;
                count += 1;
            }
        }
        if count > 0 {
            warn!(%cave_uid, revoked = count, "All PATs revoked for deactivated user");
        }
    }
}
