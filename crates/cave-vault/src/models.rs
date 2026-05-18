// SPDX-License-Identifier: AGPL-3.0-or-later
//! Data models for cave-vault.
//!
//! Mirrors HashiCorp Vault's API response shapes for drop-in compatibility.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Secret engine type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SecretEngine {
    Kv,
    Pki,
    Transit,
    Database,
    Aws,
    Azure,
}

/// KV secret with versioning (Vault KV v2 shape)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVSecret {
    pub path: String,
    pub data: HashMap<String, serde_json::Value>,
    pub version: u32,
    pub metadata: KVMetadata,
}

/// KV metadata for a path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVMetadata {
    pub created_time: DateTime<Utc>,
    pub updated_time: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
    pub custom_metadata: HashMap<String, String>,
    pub max_versions: u32,
    pub current_version: u32,
    pub oldest_version: u32,
    pub versions: HashMap<String, KVVersionMeta>,
}

/// Per-version metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KVVersionMeta {
    pub created_time: DateTime<Utc>,
    pub deletion_time: Option<DateTime<Utc>>,
    pub destroyed: bool,
}

/// PKI certificate (Vault PKI issue response shape)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PKICert {
    pub serial_number: String,
    pub certificate: String,         // PEM
    pub issuing_ca: String,          // PEM
    pub ca_chain: Vec<String>,       // PEM chain
    pub private_key: Option<String>, // PEM — only returned on issuance
    pub private_key_type: String,
    pub expiration: DateTime<Utc>,
    pub subject: CertSubject,
    pub revoked: bool,
    pub revocation_time: Option<DateTime<Utc>>,
}

/// Certificate subject / SAN fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertSubject {
    pub common_name: String,
    pub organization: Vec<String>,
    pub country: Vec<String>,
    pub alt_names: Vec<String>,
    pub ip_sans: Vec<String>,
}

/// Transit encryption key — public metadata only (no key material)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitKey {
    pub name: String,
    pub key_type: TransitKeyType,
    pub latest_version: u32,
    pub min_decryption_version: u32,
    pub min_encryption_version: u32,
    pub supports_encryption: bool,
    pub supports_decryption: bool,
    pub supports_signing: bool,
    pub supports_derivation: bool,
    pub deletion_allowed: bool,
    pub exportable: bool,
    pub allow_plaintext_backup: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Transit key algorithm
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransitKeyType {
    Aes256Gcm96,
    Ed25519,
    EcdsaP256,
    Rsa2048,
    ChaCha20Poly1305,
}

/// Token/secret lease info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub lease_id: String,
    pub renewable: bool,
    pub lease_duration: u64, // seconds
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Path-based access control policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub rules: Vec<PolicyRule>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Single path rule within a policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub path: String,
    pub capabilities: Vec<PolicyCapability>,
}

/// Capability granted on a path
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PolicyCapability {
    Create,
    Read,
    Update,
    Delete,
    List,
    Deny,
    Sudo,
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub operation: String,
    pub path: String,
    pub token_id: Option<String>,
    pub remote_addr: Option<String>,
    pub response_code: u16,
    pub error: Option<String>,
}

/// Vault seal / unseal status (mirrors /v1/sys/seal-status)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealStatus {
    pub sealed: bool,
    pub initialized: bool,
    /// Unseal key threshold
    pub t: u32,
    /// Total unseal key shares
    pub n: u32,
    /// Unseal key shares provided so far
    pub progress: u32,
    pub nonce: String,
    pub version: String,
    pub cluster_name: String,
    pub cluster_id: String,
}
