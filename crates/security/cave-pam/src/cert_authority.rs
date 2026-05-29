// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Short-lived certificate authority model for PAM.
//!
//! Teleport issues short-lived SSH/TLS certificates so that every privileged
//! access is cryptographically scoped to a specific TTL, user, and resource.
//! This module models the issuance tracking layer. Actual cryptographic
//! operations (key generation, X.509/OpenSSH signing) are owned by cave-pki;
//! this layer records the cert metadata and tracks active/expired certs for
//! revocation and audit.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock,
};
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// Whether this is an SSH certificate or an X.509 TLS certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertKind {
    /// OpenSSH certificate (used for server/DB/kubectl access).
    Ssh,
    /// X.509 TLS certificate (used for mutual TLS in app/web proxy).
    Tls,
}

/// Parameters for requesting a new certificate.
#[derive(Debug, Clone)]
pub struct CertRequest {
    /// Who is requesting the certificate.
    pub requester_id: Uuid,
    /// Primary identity principal (usually the username).
    pub principal: String,
    /// Certificate type.
    pub kind: CertKind,
    /// Additional principals to embed (e.g., Linux usernames, K8s groups).
    pub allowed_principals: Vec<String>,
    /// How long the certificate should be valid.
    pub ttl: Duration,
    /// Arbitrary extension key/value pairs embedded in the cert.
    pub extensions: HashMap<String, String>,
}

/// A certificate that has been issued.
#[derive(Debug, Clone)]
pub struct IssuedCert {
    /// Unique serial number (monotonic within the CA instance).
    pub serial: String,
    /// Certificate type.
    pub kind: CertKind,
    /// Primary identity principal.
    pub principal: String,
    /// Who requested this certificate.
    pub requester_id: Uuid,
    /// PEM-encoded certificate data (deterministic placeholder for testing;
    /// real crypto lives in cave-pki).
    pub cert_pem: String,
    /// When the certificate was issued.
    pub issued_at: DateTime<Utc>,
    /// When the certificate expires.
    pub expires_at: DateTime<Utc>,
    /// Whether this certificate has been explicitly revoked.
    pub revoked: bool,
}

impl IssuedCert {
    /// Return true if the certificate's validity window has passed.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Return true if the certificate is usable (not expired, not revoked).
    pub fn is_valid(&self) -> bool {
        !self.is_expired() && !self.revoked
    }
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the certificate authority.
#[derive(Debug, PartialEq, Clone)]
pub enum CaError {
    /// No certificate found with the given serial.
    CertNotFound,
    /// Attempted to revoke an already-revoked certificate.
    AlreadyRevoked,
    /// TTL is invalid (zero or negative not permitted for issuance checks).
    InvalidTtl,
}

impl std::fmt::Display for CaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CertNotFound => write!(f, "certificate not found"),
            Self::AlreadyRevoked => write!(f, "certificate is already revoked"),
            Self::InvalidTtl => write!(f, "invalid certificate TTL"),
        }
    }
}

impl std::error::Error for CaError {}

// ── Certificate authority ─────────────────────────────────────────────────────

/// PAM certificate authority — tracks issuance and active certs.
///
/// The actual cryptographic signing is delegated to cave-pki; this layer owns
/// the issuance lifecycle, serial numbering, and revocation list.
pub struct CertAuthority {
    cluster_name: String,
    serial_counter: AtomicU64,
    issued: Arc<RwLock<HashMap<String, IssuedCert>>>,
}

impl CertAuthority {
    /// Create a new CA for the named cluster.
    pub fn new(cluster_name: &str) -> Self {
        Self {
            cluster_name: cluster_name.to_string(),
            serial_counter: AtomicU64::new(1),
            issued: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Issue a new certificate for the given request.
    ///
    /// Returns the certificate record. The `cert_pem` field contains a
    /// deterministic placeholder that encodes the issuance metadata; real
    /// certificate generation is in cave-pki.
    pub fn issue(&self, req: CertRequest) -> Result<IssuedCert, CaError> {
        let serial = self
            .serial_counter
            .fetch_add(1, Ordering::SeqCst)
            .to_string();

        let now = Utc::now();
        let expires_at = now + req.ttl;

        // Deterministic placeholder PEM: encodes principal + serial + cluster.
        // Real X.509/OpenSSH certificate signing lives in cave-pki.
        let cert_pem = format!(
            "-----BEGIN CERTIFICATE-----\n\
             {kind}:{cluster}:{principal}:{serial}\n\
             -----END CERTIFICATE-----",
            kind = match req.kind {
                CertKind::Ssh => "SSH",
                CertKind::Tls => "TLS",
            },
            cluster = self.cluster_name,
            principal = req.principal,
            serial = serial,
        );

        let cert = IssuedCert {
            serial: serial.clone(),
            kind: req.kind,
            principal: req.principal,
            requester_id: req.requester_id,
            cert_pem,
            issued_at: now,
            expires_at,
            revoked: false,
        };

        self.issued.write().unwrap().insert(serial, cert.clone());
        Ok(cert)
    }

    /// Look up an issued certificate by its serial number.
    pub fn get(&self, serial: &str) -> Option<IssuedCert> {
        self.issued.read().unwrap().get(serial).cloned()
    }

    /// Revoke a certificate by serial. Returns an error if not found or
    /// already revoked.
    pub fn revoke(&self, serial: &str) -> Result<(), CaError> {
        let mut map = self.issued.write().unwrap();
        let cert = map.get_mut(serial).ok_or(CaError::CertNotFound)?;
        if cert.revoked {
            return Err(CaError::AlreadyRevoked);
        }
        cert.revoked = true;
        Ok(())
    }

    /// Return all certificates that are currently valid (not expired, not
    /// revoked).
    pub fn list_active(&self) -> Vec<IssuedCert> {
        self.issued
            .read()
            .unwrap()
            .values()
            .filter(|c| c.is_valid())
            .cloned()
            .collect()
    }

    /// Return all revoked certificate serials (for CRL generation).
    pub fn revocation_list(&self) -> Vec<String> {
        self.issued
            .read()
            .unwrap()
            .values()
            .filter(|c| c.revoked)
            .map(|c| c.serial.clone())
            .collect()
    }

    /// Return all certificates issued to a principal.
    pub fn certs_for_principal(&self, principal: &str) -> Vec<IssuedCert> {
        self.issued
            .read()
            .unwrap()
            .values()
            .filter(|c| c.principal == principal)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revocation_removes_from_active() {
        let ca = CertAuthority::new("test");
        let cert = ca
            .issue(CertRequest {
                requester_id: Uuid::new_v4(),
                principal: "alice".to_string(),
                kind: CertKind::Ssh,
                allowed_principals: vec!["alice".to_string()],
                ttl: Duration::hours(1),
                extensions: HashMap::new(),
            })
            .unwrap();
        ca.revoke(&cert.serial).unwrap();
        assert!(ca.list_active().is_empty());
        assert_eq!(ca.revocation_list().len(), 1);
    }

    #[test]
    fn double_revoke_errors() {
        let ca = CertAuthority::new("test");
        let cert = ca
            .issue(CertRequest {
                requester_id: Uuid::new_v4(),
                principal: "bob".to_string(),
                kind: CertKind::Tls,
                allowed_principals: vec![],
                ttl: Duration::hours(1),
                extensions: HashMap::new(),
            })
            .unwrap();
        ca.revoke(&cert.serial).unwrap();
        assert_eq!(ca.revoke(&cert.serial).unwrap_err(), CaError::AlreadyRevoked);
    }

    #[test]
    fn certs_for_principal_filters_correctly() {
        let ca = CertAuthority::new("test");
        let uid = Uuid::new_v4();
        ca.issue(CertRequest {
            requester_id: uid,
            principal: "alice".to_string(),
            kind: CertKind::Ssh,
            allowed_principals: vec!["alice".to_string()],
            ttl: Duration::hours(1),
            extensions: HashMap::new(),
        })
        .unwrap();
        ca.issue(CertRequest {
            requester_id: uid,
            principal: "bob".to_string(),
            kind: CertKind::Ssh,
            allowed_principals: vec!["bob".to_string()],
            ttl: Duration::hours(1),
            extensions: HashMap::new(),
        })
        .unwrap();
        assert_eq!(ca.certs_for_principal("alice").len(), 1);
        assert_eq!(ca.certs_for_principal("bob").len(), 1);
    }
}
