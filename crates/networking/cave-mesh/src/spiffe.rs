// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SPIFFE identity and internal certificate authority.
//!
//! Implements:
//!   • SpiffeId parsing / formatting / validation
//!   • TrustDomain registry
//!   • InternalCa — in-memory CA using rcgen (issues X.509 SVIDs)
//!   • CertRotationManager — tracks cert expiry and triggers re-issuance
//!   • SVID (SPIFFE Verifiable Identity Document) type

use crate::{
    error::{MeshError, MeshResult},
    models::{CertBundle, SpiffeId},
};
use chrono::{DateTime, Datelike, Duration, Utc};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, Ia5String,
    IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::{info, warn};

// ─────────────────────────────────────────────────────────────
// TrustDomain
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustDomain {
    pub name: String,
    /// PEM-encoded root CA certificate for this trust domain.
    pub root_cert_pem: String,
    pub created_at: DateTime<Utc>,
}

impl TrustDomain {
    pub fn new(name: impl Into<String>, root_cert_pem: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            root_cert_pem: root_cert_pem.into(),
            created_at: Utc::now(),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// SVID — SPIFFE Verifiable Identity Document
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Svid {
    pub spiffe_id: SpiffeId,
    pub cert_pem: String,
    pub key_pem: String,
    pub bundle_pem: String,
    pub serial: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
}

impl Svid {
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.not_after
    }

    pub fn expires_within(&self, secs: i64) -> bool {
        let threshold = Utc::now() + Duration::seconds(secs);
        self.not_after < threshold
    }

    pub fn remaining_seconds(&self) -> i64 {
        (self.not_after - Utc::now()).num_seconds().max(0)
    }
}

// ─────────────────────────────────────────────────────────────
// InternalCa
// ─────────────────────────────────────────────────────────────

/// In-memory certificate authority for issuing SPIFFE SVIDs.
///
/// Uses rcgen to generate a self-signed root CA and issue leaf certificates.
/// In production this would be backed by Vault, cert-manager, or an HSM.
pub struct InternalCa {
    /// Signed CA certificate (used as the issuer for leaf certs).
    ca_cert: Certificate,
    /// CA key pair (used to sign leaf certificates).
    ca_kp: KeyPair,
    root_cert_pem: String,
    trust_domain: String,
    issued_count: std::sync::atomic::AtomicU64,
}

impl InternalCa {
    /// Create a new in-memory CA for a trust domain.
    pub fn new(trust_domain: impl Into<String>) -> MeshResult<Self> {
        let trust_domain = trust_domain.into();

        let ca_kp = KeyPair::generate()
            .map_err(|e| MeshError::Spiffe(format!("CA key generation failed: {e}")))?;

        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, format!("CAVE Mesh CA — {trust_domain}"));
        params
            .distinguished_name
            .push(DnType::OrganizationName, "CAVE Platform");
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.not_before = rcgen::date_time_ymd(2024, 1, 1);
        params.not_after = rcgen::date_time_ymd(2034, 1, 1);

        let ca_cert = params
            .self_signed(&ca_kp)
            .map_err(|e| MeshError::Spiffe(format!("CA cert generation failed: {e}")))?;

        let root_cert_pem = ca_cert.pem();

        info!(trust_domain = %trust_domain, "SPIFFE internal CA initialized");

        Ok(Self {
            ca_cert,
            ca_kp,
            root_cert_pem,
            trust_domain,
            issued_count: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Root CA certificate PEM.
    pub fn root_cert_pem(&self) -> &str {
        &self.root_cert_pem
    }

    pub fn trust_domain(&self) -> &str {
        &self.trust_domain
    }

    /// Issue an SVID for a workload identity.
    pub fn issue_svid(
        &self,
        namespace: &str,
        service_account: &str,
        ttl_hours: u32,
    ) -> MeshResult<Svid> {
        let spiffe_id = SpiffeId::for_workload(&self.trust_domain, namespace, service_account);
        let uri_san = spiffe_id.to_uri();

        let leaf_kp = KeyPair::generate()
            .map_err(|e| MeshError::Spiffe(format!("SVID key generation failed: {e}")))?;

        let uri_ia5 = Ia5String::try_from(uri_san.as_str())
            .map_err(|e| MeshError::Spiffe(format!("Invalid SPIFFE URI: {e}")))?;

        let mut params = CertificateParams::default();
        params.is_ca = IsCa::NoCa;
        params
            .distinguished_name
            .push(DnType::CommonName, format!("{namespace}/{service_account}"));
        params.subject_alt_names = vec![SanType::URI(uri_ia5)];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ClientAuth,
            ExtendedKeyUsagePurpose::ServerAuth,
        ];
        // Compute validity window using chrono (avoids direct `time` crate dependency).
        let now = Utc::now();
        let expire = now + Duration::hours(ttl_hours as i64);
        params.not_before = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day() as u8);
        params.not_after =
            rcgen::date_time_ymd(expire.year(), expire.month() as u8, expire.day() as u8);

        let leaf = params
            .signed_by(&leaf_kp, &self.ca_cert, &self.ca_kp)
            .map_err(|e| MeshError::Spiffe(format!("SVID cert signing failed: {e}")))?;

        let cert_pem = leaf.pem();
        let key_pem = leaf_kp.serialize_pem();

        let serial_num = self
            .issued_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let serial = format!("{:016x}", serial_num);

        let not_before = Utc::now();
        let not_after = not_before + Duration::hours(ttl_hours as i64);

        info!(
            spiffe_id = %spiffe_id,
            ttl_hours = ttl_hours,
            "SVID issued"
        );

        Ok(Svid {
            spiffe_id,
            cert_pem: cert_pem.clone(),
            key_pem,
            bundle_pem: format!("{cert_pem}\n{}", self.root_cert_pem),
            serial,
            not_before,
            not_after,
        })
    }

    /// Convert an SVID into a CertBundle for storage in MeshState.
    pub fn svid_to_bundle(svid: &Svid) -> CertBundle {
        CertBundle {
            spiffe_id: svid.spiffe_id.clone(),
            cert_pem: svid.cert_pem.clone(),
            key_pem: Some(svid.key_pem.clone()),
            not_before: svid.not_before,
            not_after: svid.not_after,
            serial: svid.serial.clone(),
        }
    }
}

// ─────────────────────────────────────────────────────────────
// CertRotationManager
// ─────────────────────────────────────────────────────────────

/// Tracks issued SVIDs and triggers rotation before expiry.
#[derive(Clone)]
pub struct CertRotationManager {
    svids: Arc<RwLock<HashMap<String, Svid>>>,
    /// Rotate when cert has fewer than this many seconds remaining.
    rotation_threshold_secs: i64,
}

impl Default for CertRotationManager {
    fn default() -> Self {
        Self::new(3600) // rotate 1 h before expiry
    }
}

impl CertRotationManager {
    pub fn new(rotation_threshold_secs: i64) -> Self {
        Self {
            svids: Arc::new(RwLock::new(HashMap::new())),
            rotation_threshold_secs,
        }
    }

    /// Store an SVID keyed by SPIFFE ID URI.
    pub fn store(&self, svid: Svid) {
        let key = svid.spiffe_id.to_uri();
        self.svids.write().unwrap().insert(key, svid);
    }

    /// Get the current SVID for a SPIFFE ID.
    pub fn get(&self, spiffe_id: &SpiffeId) -> Option<Svid> {
        self.svids.read().unwrap().get(&spiffe_id.to_uri()).cloned()
    }

    /// Returns all SVIDs that need rotation (expiring soon or already expired).
    pub fn pending_rotation(&self) -> Vec<SpiffeId> {
        self.svids
            .read()
            .unwrap()
            .values()
            .filter(|s| s.expires_within(self.rotation_threshold_secs))
            .map(|s| s.spiffe_id.clone())
            .collect()
    }

    /// Revoke an SVID.
    pub fn revoke(&self, spiffe_id: &SpiffeId) {
        let key = spiffe_id.to_uri();
        self.svids.write().unwrap().remove(&key);
        warn!(spiffe_id = %spiffe_id, "SVID revoked");
    }

    /// List all tracked SVIDs.
    pub fn list(&self) -> Vec<Svid> {
        self.svids.read().unwrap().values().cloned().collect()
    }

    /// Rotation summary snapshot.
    pub fn rotation_snapshot(&self) -> RotationSnapshot {
        let svids = self.svids.read().unwrap();
        let total = svids.len();
        let pending = svids
            .values()
            .filter(|s| s.expires_within(self.rotation_threshold_secs))
            .count();
        let expired = svids.values().filter(|s| s.is_expired()).count();
        RotationSnapshot {
            total,
            pending_rotation: pending,
            expired,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationSnapshot {
    pub total: usize,
    pub pending_rotation: usize,
    pub expired: usize,
}

// ─────────────────────────────────────────────────────────────
// TrustDomainRegistry
// ─────────────────────────────────────────────────────────────

/// Registry of trust domains for federation / multi-cluster.
#[derive(Clone)]
pub struct TrustDomainRegistry {
    domains: Arc<RwLock<HashMap<String, TrustDomain>>>,
}

impl Default for TrustDomainRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustDomainRegistry {
    pub fn new() -> Self {
        Self {
            domains: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&self, domain: TrustDomain) {
        self.domains
            .write()
            .unwrap()
            .insert(domain.name.clone(), domain);
    }

    pub fn get(&self, name: &str) -> Option<TrustDomain> {
        self.domains.read().unwrap().get(name).cloned()
    }

    pub fn list(&self) -> Vec<TrustDomain> {
        self.domains.read().unwrap().values().cloned().collect()
    }

    pub fn remove(&self, name: &str) {
        self.domains.write().unwrap().remove(name);
    }

    /// Verify that a SPIFFE ID belongs to a known trust domain.
    pub fn is_trusted(&self, spiffe_id: &SpiffeId) -> bool {
        self.domains
            .read()
            .unwrap()
            .contains_key(&spiffe_id.trust_domain)
    }
}
