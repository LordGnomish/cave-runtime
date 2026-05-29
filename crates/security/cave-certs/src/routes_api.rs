// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! REST handler logic for cave-certs API.
//!
//! Cite: cert-manager v1.13.0 — the operations surfaced here mirror the
//! cert-manager CRD operations (issue, list, remove) as a REST façade.
//! The actual axum router wiring lives in `routes.rs`.

use crate::issuers::{IssueRequest, SelfSignedIssuer};
use crate::models::{CertState, Certificate};
use crate::store::CertificateStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Global in-process store (mirrors cert-manager's Kubernetes Secret store).
// In production this would be replaced by a real persistence layer.
// ---------------------------------------------------------------------------

static STORE: Mutex<Option<CertificateStore>> = Mutex::new(None);

fn with_store<F, R>(f: F) -> R
where
    F: FnOnce(&mut CertificateStore) -> R,
{
    let mut guard = STORE.lock().unwrap();
    if guard.is_none() {
        *guard = Some(CertificateStore::new());
    }
    f(guard.as_mut().unwrap())
}

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

/// Cite: cert-manager SelfSigned issuer issue request — equivalent to
/// `Certificate.spec` with issuerRef → `selfsigned`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfSignedIssueRequest {
    pub tenant_id: String,
    pub dns_names: Vec<String>,
    pub common_name: Option<String>,
    /// Cite: cert-manager `Certificate.spec.secretName`.
    pub secret_name: String,
    pub duration_seconds: i64,
    pub is_ca: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfSignedIssueResponse {
    pub secret_name: String,
    pub certificate_pem: String,
    pub domain: String,
    pub not_after: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertSummary {
    pub secret_name: String,
    pub domain: String,
    pub state: String,
    pub not_after: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertListResponse {
    pub tenant_id: String,
    pub certs: Vec<CertSummary>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Cite: cert-manager selfsigned issuer — issue a self-signed certificate
/// and store it under `(tenant_id, secret_name)`.
pub fn handle_issue_selfsigned(
    req: SelfSignedIssueRequest,
) -> Result<SelfSignedIssueResponse, String> {
    if req.tenant_id.trim().is_empty() {
        return Err("tenant_id must be non-empty".into());
    }
    if req.secret_name.trim().is_empty() {
        return Err("secret_name must be non-empty".into());
    }
    if req.dns_names.is_empty() && req.common_name.is_none() {
        return Err("at least one of dns_names or common_name must be set".into());
    }

    let issuer = SelfSignedIssuer::new(&req.tenant_id);
    let issue_req = IssueRequest {
        tenant_id: req.tenant_id.clone(),
        dns_names: req.dns_names.clone(),
        common_name: req.common_name.clone(),
        duration_seconds: req.duration_seconds,
        is_ca: req.is_ca,
    };
    let result = issuer.issue(&issue_req).map_err(|e| e.to_string())?;

    let now = Utc::now();
    let not_after = now + Duration::seconds(req.duration_seconds);

    // Derive fingerprint from the PEM.
    let mut h = Sha256::new();
    h.update(result.certificate_pem.as_bytes());
    let fingerprint = format!("{:x}", h.finalize());

    // The "primary" domain is the first DNS name or the CN.
    let domain = req
        .dns_names
        .first()
        .cloned()
        .or_else(|| req.common_name.clone())
        .unwrap_or_default();

    // Cite: cert-manager — the first SAN is the leaf domain for display.
    let cert = Certificate {
        id: Uuid::new_v4(),
        domain: domain.clone(),
        san_domains: req.dns_names.clone(),
        issuer: "self-signed".into(),
        not_before: now,
        not_after,
        serial_number: format!("{:08x}", Uuid::new_v4().as_u128() as u32),
        fingerprint_sha256: fingerprint[..16].to_string(),
        state: CertState::Valid,
        auto_renew: false,
    };

    with_store(|store| store.put(&req.tenant_id, &req.secret_name, cert));

    Ok(SelfSignedIssueResponse {
        secret_name: req.secret_name,
        certificate_pem: result.certificate_pem,
        domain,
        not_after,
    })
}

/// Cite: cert-manager — list all certificates for a tenant.
pub fn handle_list_certs(tenant_id: &str) -> CertListResponse {
    let certs = with_store(|store| {
        store
            .list(tenant_id)
            .into_iter()
            .map(|c| CertSummary {
                secret_name: c.fingerprint_sha256.clone(), // used as display key
                domain: c.domain.clone(),
                state: format!("{:?}", c.state),
                not_after: c.not_after,
            })
            .collect::<Vec<_>>()
    });
    CertListResponse {
        tenant_id: tenant_id.into(),
        certs,
    }
}

/// Cite: cert-manager certificate deletion — remove a cert by secret_name.
pub fn handle_remove_cert(tenant_id: &str, secret_name: &str) -> bool {
    with_store(|store| store.remove(tenant_id, secret_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_selfsigned_stores_cert() {
        let req = SelfSignedIssueRequest {
            tenant_id: "t-unit".into(),
            dns_names: vec!["unit.example.com".into()],
            common_name: None,
            secret_name: "unit-tls".into(),
            duration_seconds: 90 * 86_400,
            is_ca: false,
        };
        let resp = handle_issue_selfsigned(req).unwrap();
        assert!(!resp.certificate_pem.is_empty());
        assert_eq!(resp.domain, "unit.example.com");
    }

    #[test]
    fn remove_cert_returns_true_when_present() {
        let req = SelfSignedIssueRequest {
            tenant_id: "t-unit2".into(),
            dns_names: vec!["unit2.example.com".into()],
            common_name: None,
            secret_name: "unit2-tls".into(),
            duration_seconds: 90 * 86_400,
            is_ca: false,
        };
        handle_issue_selfsigned(req).unwrap();
        assert!(handle_remove_cert("t-unit2", "unit2-tls"));
        assert!(!handle_remove_cert("t-unit2", "unit2-tls"));
    }
}
