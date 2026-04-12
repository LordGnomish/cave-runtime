<<<<<<< HEAD
//! mTLS policy management.
//!
//! Implements PeerAuthentication (STRICT / PERMISSIVE / DISABLE) per namespace
//! or workload.  Provides a `validate_peer` function that the data-plane proxy
//! calls to decide whether to accept an incoming connection.

use crate::{
    error::{MeshError, MeshResult},
    models::{MtlsMode, PeerAuthentication, TlsContext},
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::debug;

/// Manages PeerAuthentication policies and enforces mTLS rules.
#[derive(Debug, Clone)]
pub struct MtlsManager {
    /// Keyed by "namespace/policy-name"
    policies: Arc<RwLock<HashMap<String, PeerAuthentication>>>,
}

impl Default for MtlsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MtlsManager {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ─── CRUD ────────────────────────────────────────────────

    pub fn upsert_policy(&self, policy: PeerAuthentication) {
        let key = format!("{}/{}", policy.namespace, policy.name);
        let mut map = self.policies.write().unwrap();
        map.insert(key, policy);
    }

    pub fn remove_policy(&self, namespace: &str, name: &str) {
        let key = format!("{namespace}/{name}");
        let mut map = self.policies.write().unwrap();
        map.remove(&key);
    }

    pub fn list_policies(&self) -> Vec<PeerAuthentication> {
        let map = self.policies.read().unwrap();
        map.values().cloned().collect()
    }

    pub fn get_policy(&self, namespace: &str, name: &str) -> Option<PeerAuthentication> {
        let key = format!("{namespace}/{name}");
        let map = self.policies.read().unwrap();
        map.get(&key).cloned()
    }

    // ─── Policy resolution ───────────────────────────────────

    /// Determine the effective mTLS mode for a workload (namespace + labels).
    ///
    /// Priority (highest first):
    ///   1. Workload-specific policy (selector matches)
    ///   2. Namespace-wide policy (no selector / empty selector)
    ///   3. Mesh-wide default (PERMISSIVE)
    pub fn effective_mode(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
    ) -> MtlsMode {
        let map = self.policies.read().unwrap();

        let mut namespace_mode: Option<MtlsMode> = None;
        let mut workload_mode: Option<MtlsMode> = None;

        for policy in map.values() {
            if policy.namespace != namespace {
                continue;
            }
            let is_namespace_wide = policy
                .selector
                .as_ref()
                .map(|s| s.is_empty())
                .unwrap_or(true);
            if is_namespace_wide {
                // Namespace-wide policy
                namespace_mode = Some(policy.mtls.mode.clone());
            } else if let Some(selector) = &policy.selector {
                // Workload-specific — selector must be a subset of workload labels
                if selector
                    .iter()
                    .all(|(k, v)| workload_labels.get(k).map(|vv| vv == v).unwrap_or(false))
                {
                    workload_mode = Some(policy.mtls.mode.clone());
                }
            }
        }

        workload_mode
            .or(namespace_mode)
            .unwrap_or(MtlsMode::Permissive)
    }

    // ─── Enforcement ─────────────────────────────────────────

    /// Validate an incoming peer connection against the effective mTLS policy.
    pub fn validate_peer(
        &self,
        namespace: &str,
        workload_labels: &HashMap<String, String>,
        ctx: &TlsContext,
    ) -> MeshResult<()> {
        let mode = self.effective_mode(namespace, workload_labels);

        debug!(
            namespace = %namespace,
            is_mtls = %ctx.is_mtls,
            mode = ?mode,
            "mTLS validation"
        );

        match mode {
            MtlsMode::Strict => {
                if !ctx.is_mtls {
                    return Err(MeshError::MtlsRejected(
                        "STRICT mode requires mTLS — plaintext rejected".to_string(),
                    ));
                }
            }
            MtlsMode::Permissive => {
                // Both plaintext and mTLS are accepted
            }
            MtlsMode::Disable => {
                // mTLS disabled — plaintext only (mTLS still accepted for compatibility)
            }
        }
        Ok(())
    }

    /// Extract the SPIFFE principal from the peer certificate (passed in ctx).
    pub fn peer_principal(ctx: &TlsContext) -> Option<&str> {
        ctx.peer_principal.as_deref()
    }
}
=======
//! mTLS management — certificate generation, rotation, verification, inventory.
//!
//! Uses Ed25519 key pairs (ring 0.17) and SPIFFE-style subject identifiers.

use crate::MeshState;
use chrono::{DateTime, Duration, Utc};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Certificate Record ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertRecord {
    pub id: Uuid,
    pub service_id: Uuid,
    pub service_name: String,
    /// SPIFFE URI, e.g. spiffe://cluster.local/ns/default/sa/my-service
    pub subject: String,
    pub public_key_pem: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub serial: String,
    pub fingerprint: String,
    pub revoked: bool,
}

impl CertRecord {
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn days_until_expiry(&self) -> i64 {
        (self.expires_at - Utc::now()).num_days()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertInventoryEntry {
    pub id: Uuid,
    pub service_name: String,
    pub subject: String,
    pub expires_at: DateTime<Utc>,
    pub days_until_expiry: i64,
    pub is_expired: bool,
    pub revoked: bool,
    pub fingerprint: String,
}

// ─── Certificate Operations ───────────────────────────────────────────────────

/// Generate a new Ed25519 mTLS certificate for a service and store it.
pub fn generate_cert(
    service_id: Uuid,
    service_name: &str,
    namespace: &str,
    validity_days: i64,
    state: &MeshState,
) -> Result<CertRecord, String> {
    let rng = SystemRandom::new();
    let pkcs8_bytes = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|_| "Ed25519 key generation failed".to_string())?;
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8_bytes.as_ref())
        .map_err(|e| format!("Key pair construction failed: {e:?}"))?;

    let pub_key_bytes = key_pair.public_key().as_ref().to_vec();
    let pub_key_b64 = base64_encode(&pub_key_bytes);
    let pub_key_hex: String = pub_key_bytes.iter().map(|b| format!("{b:02x}")).collect();

    let now = Utc::now();
    let serial = Uuid::new_v4().to_string().replace('-', "");
    let fingerprint = format!("{}:{}", &serial[..8], &pub_key_hex[..16]);

    let cert = CertRecord {
        id: Uuid::new_v4(),
        service_id,
        service_name: service_name.to_string(),
        subject: format!("spiffe://cluster.local/ns/{namespace}/sa/{service_name}"),
        public_key_pem: format!(
            "-----BEGIN PUBLIC KEY-----\n{pub_key_b64}\n-----END PUBLIC KEY-----"
        ),
        issued_at: now,
        expires_at: now + Duration::days(validity_days),
        serial,
        fingerprint,
        revoked: false,
    };

    state.certs.lock().unwrap().insert(cert.id, cert.clone());
    tracing::info!(
        service = service_name,
        cert_id = %cert.id,
        expires_days = validity_days,
        "Generated mTLS certificate"
    );
    Ok(cert)
}

/// Revoke all existing certs for a service and issue a fresh one (90-day validity).
pub fn rotate_cert(
    service_id: Uuid,
    service_name: &str,
    namespace: &str,
    state: &MeshState,
) -> Result<CertRecord, String> {
    {
        let mut certs = state.certs.lock().unwrap();
        for cert in certs.values_mut() {
            if cert.service_id == service_id && !cert.revoked {
                cert.revoked = true;
                tracing::info!(cert_id = %cert.id, "Revoked cert during rotation");
            }
        }
    }
    generate_cert(service_id, service_name, namespace, 90, state)
}

/// Verify a peer certificate by ID: checks existence, revocation, and expiry.
pub fn verify_peer(cert_id: Uuid, state: &MeshState) -> Result<bool, String> {
    let certs = state.certs.lock().unwrap();
    match certs.get(&cert_id) {
        None => Err(format!("Certificate {cert_id} not found")),
        Some(c) if c.revoked => Ok(false),
        Some(c) if c.is_expired() => Ok(false),
        Some(_) => Ok(true),
    }
}

/// List all certificates with computed status fields, sorted by expiry.
pub fn cert_inventory(state: &MeshState) -> Vec<CertInventoryEntry> {
    let certs = state.certs.lock().unwrap();
    let mut entries: Vec<CertInventoryEntry> = certs
        .values()
        .map(|c| CertInventoryEntry {
            id: c.id,
            service_name: c.service_name.clone(),
            subject: c.subject.clone(),
            expires_at: c.expires_at,
            days_until_expiry: c.days_until_expiry(),
            is_expired: c.is_expired(),
            revoked: c.revoked,
            fingerprint: c.fingerprint.clone(),
        })
        .collect();
    entries.sort_by_key(|e| e.expires_at);
    entries
}

// ─── Base64 (no external dep) ─────────────────────────────────────────────────

fn base64_encode(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as usize;
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] as usize } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] as usize } else { 0 };
        out.push(CHARS[b0 >> 2] as char);
        out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        out.push(if i + 1 < bytes.len() { CHARS[((b1 & 15) << 2) | (b2 >> 6)] as char } else { '=' });
        out.push(if i + 2 < bytes.len() { CHARS[b2 & 63] as char } else { '=' });
        i += 3;
    }
    out
}
>>>>>>> claude/peaceful-lederberg
