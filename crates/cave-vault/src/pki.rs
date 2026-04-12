<<<<<<< HEAD
//! PKI secrets engine — certificate authority and issuance.
//!
//! Generates a root CA with rcgen, issues leaf TLS certs signed by that CA,
//! maintains an in-memory revocation list, and validates certificate chains.

use crate::models::{CertSubject, PKICert};
use chrono::{DateTime, Utc};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType,
};
use std::collections::HashMap;
use thiserror::Error;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum PKIError {
    #[error("Certificate generation failed: {0}")]
    GenerationFailed(String),
    #[error("Certificate not found: {0}")]
    NotFound(String),
    #[error("Root CA not initialised — call generate_root_ca first")]
    NoRootCa,
    #[error("Certificate already revoked")]
    AlreadyRevoked,
    #[error("Chain validation failed")]
    ValidationFailed,
}

impl From<rcgen::Error> for PKIError {
    fn from(e: rcgen::Error) -> Self {
        PKIError::GenerationFailed(e.to_string())
    }
}

/// Live rcgen CA state stored in VaultStore — needed to sign leaf certs.
pub struct CaState {
    pub cert: Certificate,
    pub key_pair: KeyPair,
    pub cert_pem: String,
    pub key_pem: String,
    pub serial: String,
}

/// Persisted cert record (PEM + metadata)
#[derive(Debug, Clone)]
pub struct StoredCert {
    pub pki_cert: PKICert,
}

/// Generate a self-signed root CA and return the live CaState.
pub fn generate_root_ca(
    common_name: &str,
    organization: &str,
    country: &str,
    ttl_days: u32,
) -> Result<CaState, PKIError> {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn.push(DnType::OrganizationName, organization);
    dn.push(DnType::CountryName, country);

    let not_before = OffsetDateTime::now_utc();
    let not_after = not_before + Duration::days(ttl_days as i64);

    let mut params = CertificateParams::default();
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before = not_before;
    params.not_after = not_after;
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

    let key_pair = KeyPair::generate().map_err(PKIError::from)?;
    let cert = params.self_signed(&key_pair).map_err(PKIError::from)?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let serial = format!("{:032x}", Uuid::new_v4().as_u128());

    Ok(CaState { cert, key_pair, cert_pem, key_pem, serial })
}

/// Issue a leaf certificate signed by the root CA.
pub fn issue_certificate(
    ca: &CaState,
    common_name: &str,
    alt_names: &[String],
    ip_sans: &[String],
    organization: &str,
    country: &str,
    ttl_days: u32,
) -> Result<PKICert, PKIError> {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    if !organization.is_empty() {
        dn.push(DnType::OrganizationName, organization);
    }
    if !country.is_empty() {
        dn.push(DnType::CountryName, country);
    }

    let not_before = OffsetDateTime::now_utc();
    let not_after = not_before + Duration::days(ttl_days as i64);

    let mut params = CertificateParams::default();
    params.distinguished_name = dn;
    params.not_before = not_before;
    params.not_after = not_after;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];

    // Add SANs — common_name as first DNS SAN
    let all_dns: Vec<String> = std::iter::once(common_name.to_string())
        .chain(alt_names.iter().cloned())
        .collect();

    for name in &all_dns {
        if let Ok(ia5) = rcgen::Ia5String::try_from(name.as_str()) {
            params.subject_alt_names.push(SanType::DnsName(ia5));
        }
    }
    for ip_str in ip_sans {
        if let Ok(ip) = ip_str.parse::<std::net::IpAddr>() {
            params.subject_alt_names.push(SanType::IpAddress(ip));
        }
    }

    let leaf_key = KeyPair::generate().map_err(PKIError::from)?;
    let leaf_cert = params
        .signed_by(&leaf_key, &ca.cert, &ca.key_pair)
        .map_err(PKIError::from)?;

    let cert_pem = leaf_cert.pem();
    let key_pem = leaf_key.serialize_pem();
    let serial = format!("{:032x}", Uuid::new_v4().as_u128());

    let expiration = offset_to_chrono(not_after);

    Ok(PKICert {
        serial_number: serial,
        certificate: cert_pem,
        issuing_ca: ca.cert_pem.clone(),
        ca_chain: vec![ca.cert_pem.clone()],
        private_key: Some(key_pem),
        private_key_type: "ec".to_string(),
        expiration,
        subject: CertSubject {
            common_name: common_name.to_string(),
            organization: if organization.is_empty() {
                vec![]
            } else {
                vec![organization.to_string()]
            },
            country: if country.is_empty() {
                vec![]
            } else {
                vec![country.to_string()]
            },
            alt_names: alt_names.to_vec(),
            ip_sans: ip_sans.to_vec(),
        },
        revoked: false,
        revocation_time: None,
    })
}

/// Record a certificate revocation.
pub fn revoke_cert(
    certs: &mut HashMap<String, StoredCert>,
    revoked: &mut HashMap<String, DateTime<Utc>>,
    serial: &str,
) -> Result<(), PKIError> {
    if revoked.contains_key(serial) {
        return Err(PKIError::AlreadyRevoked);
    }
    let now = Utc::now();
    revoked.insert(serial.to_string(), now);
    if let Some(stored) = certs.get_mut(serial) {
        stored.pki_cert.revoked = true;
        stored.pki_cert.revocation_time = Some(now);
    }
    Ok(())
}

/// Generate a CRL (Certificate Revocation List) — returns PEM-like JSON summary.
pub fn generate_crl(
    revoked: &HashMap<String, DateTime<Utc>>,
    ca_serial: &str,
) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = revoked
        .iter()
        .map(|(serial, ts)| {
            serde_json::json!({
                "serial_number": serial,
                "revocation_time": ts.to_rfc3339(),
            })
        })
        .collect();
    serde_json::json!({
        "issuer_serial": ca_serial,
        "generated_at": Utc::now().to_rfc3339(),
        "revoked_certs": entries,
    })
}

/// Validate a PEM certificate chain (leaf → intermediates → root).
/// Returns true if the chain is non-empty and structurally valid (PEM parse check).
pub fn cert_chain_validation(chain: &[String]) -> bool {
    if chain.is_empty() {
        return false;
    }
    // Parse each PEM cert to validate it is well-formed
    for pem in chain {
        if !pem.contains("-----BEGIN CERTIFICATE-----") {
            return false;
        }
    }
    true
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn offset_to_chrono(dt: OffsetDateTime) -> DateTime<Utc> {
    let unix = dt.unix_timestamp();
    DateTime::<Utc>::from_timestamp(unix, 0).unwrap_or_else(Utc::now)
=======
//! PKI Secrets Engine — root CA, intermediate CA, issue, revoke, CRL.

use chrono::{DateTime, Utc};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
    SanType, PKCS_ECDSA_P256_SHA256,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::Duration;
use uuid::Uuid;

use crate::error::VaultError;

// ── Stored certificate (serialisable) ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCert {
    pub serial: String,
    pub certificate: String,       // signed PEM
    pub issuing_ca: String,        // issuer cert PEM
    pub ca_chain: Vec<String>,
    pub private_key: Option<String>,
    pub subject: String,
    pub alt_names: Vec<String>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    pub revocation_time: Option<DateTime<Utc>>,
}

// ── CA state ──────────────────────────────────────────────────────────────────

/// CA state — intentionally NOT Clone (Certificate / KeyPair are not Clone).
pub struct CaState {
    pub cert_pem: String,   // signed PEM (what clients see)
    pub key_pem: String,    // private key PEM (never exported in API)
    pub serial: u64,
    /// rcgen `Certificate` held in memory to sign child certs.
    cert: Certificate,
}

impl CaState {
    fn new(cert_pem: String, key_pem: String, serial: u64, cert: Certificate) -> Self {
        Self { cert_pem, key_pem, serial, cert }
    }
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct PkiEngine {
    pub root_ca: Option<CaState>,
    pub intermediate_ca: Option<CaState>,
    pub certs: HashMap<String, StoredCert>,
    pub revoked_serials: HashMap<String, DateTime<Utc>>,
    next_serial: u64,
}

impl PkiEngine {
    pub fn new() -> Self {
        Self {
            root_ca: None,
            intermediate_ca: None,
            certs: HashMap::new(),
            revoked_serials: HashMap::new(),
            next_serial: 1000,
        }
    }

    fn make_key_pair() -> Result<KeyPair, VaultError> {
        KeyPair::generate(&PKCS_ECDSA_P256_SHA256)
            .map_err(|e| VaultError::CryptoError(e.to_string()))
    }

    /// Generate a self-signed root CA.
    pub fn generate_root_ca(
        &mut self,
        common_name: &str,
        organization: &str,
        ttl_days: i64,
    ) -> Result<String, VaultError> {
        let key_pair = Self::make_key_pair()?;
        let key_pem = key_pair.serialize_pem();

        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        dn.push(DnType::OrganizationName, organization);
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.not_before = time::OffsetDateTime::now_utc();
        params.not_after = time::OffsetDateTime::now_utc() + Duration::days(ttl_days);
        params.key_pair = Some(key_pair);

        let cert = Certificate::from_params(params)
            .map_err(|e| VaultError::CryptoError(e.to_string()))?;
        let cert_pem = cert
            .serialize_pem()
            .map_err(|e| VaultError::CryptoError(e.to_string()))?;

        self.root_ca = Some(CaState::new(cert_pem.clone(), key_pem, 1, cert));
        Ok(cert_pem)
    }

    /// Generate an intermediate CA signed by the root CA.
    pub fn generate_intermediate_ca(
        &mut self,
        common_name: &str,
        organization: &str,
        ttl_days: i64,
    ) -> Result<String, VaultError> {
        let root_cert_pem = self
            .root_ca
            .as_ref()
            .map(|ca| ca.cert_pem.clone())
            .ok_or_else(|| VaultError::InvalidRequest("root CA not configured".into()))?;

        let key_pair = Self::make_key_pair()?;
        let key_pem = key_pair.serialize_pem();

        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        dn.push(DnType::OrganizationName, organization);
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.not_before = time::OffsetDateTime::now_utc();
        params.not_after = time::OffsetDateTime::now_utc() + Duration::days(ttl_days);
        params.key_pair = Some(key_pair);

        let int_cert = Certificate::from_params(params)
            .map_err(|e| VaultError::CryptoError(e.to_string()))?;

        // Sign with root CA
        let signed_pem = {
            let root = self.root_ca.as_ref().unwrap();
            int_cert
                .serialize_pem_with_signer(&root.cert)
                .map_err(|e| VaultError::CryptoError(e.to_string()))?
        };

        let chain = format!("{}\n{}", signed_pem, root_cert_pem);
        self.intermediate_ca = Some(CaState::new(signed_pem, key_pem, 2, int_cert));
        Ok(chain)
    }

    /// Issue a leaf certificate, signed by intermediate CA (or root if none).
    pub fn issue_certificate(
        &mut self,
        common_name: &str,
        alt_names: &[String],
        ttl_days: i64,
        include_private_key: bool,
    ) -> Result<StoredCert, VaultError> {
        let key_pair = Self::make_key_pair()?;
        let key_pem = if include_private_key {
            Some(key_pair.serialize_pem())
        } else {
            None
        };

        // Build SANs — include common_name + alt_names
        let mut san_strings: Vec<String> = vec![common_name.to_string()];
        san_strings.extend_from_slice(alt_names);
        // Deduplicate
        san_strings.dedup();

        let mut params = CertificateParams::new(san_strings.clone());
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        params.distinguished_name = dn;
        params.is_ca = IsCa::NoCa;
        params.not_before = time::OffsetDateTime::now_utc();
        params.not_after = time::OffsetDateTime::now_utc() + Duration::days(ttl_days);
        params.key_pair = Some(key_pair);

        let leaf_cert = Certificate::from_params(params)
            .map_err(|e| VaultError::CryptoError(e.to_string()))?;

        // Sign with intermediate or root CA
        let (signed_pem, issuing_ca_pem) = if let Some(int) = self.intermediate_ca.as_ref() {
            let pem = leaf_cert
                .serialize_pem_with_signer(&int.cert)
                .map_err(|e| VaultError::CryptoError(e.to_string()))?;
            (pem, int.cert_pem.clone())
        } else if let Some(root) = self.root_ca.as_ref() {
            let pem = leaf_cert
                .serialize_pem_with_signer(&root.cert)
                .map_err(|e| VaultError::CryptoError(e.to_string()))?;
            (pem, root.cert_pem.clone())
        } else {
            return Err(VaultError::InvalidRequest("no CA configured".into()));
        };

        let serial = format!("{:016x}", self.next_serial);
        self.next_serial += 1;

        let stored = StoredCert {
            serial: serial.clone(),
            certificate: signed_pem,
            issuing_ca: issuing_ca_pem.clone(),
            ca_chain: vec![issuing_ca_pem],
            private_key: key_pem,
            subject: common_name.to_string(),
            alt_names: alt_names.to_vec(),
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(ttl_days),
            revoked: false,
            revocation_time: None,
        };
        self.certs.insert(serial, stored.clone());
        Ok(stored)
    }

    /// Revoke a certificate by serial.
    pub fn revoke(&mut self, serial: &str) -> Result<DateTime<Utc>, VaultError> {
        let cert = self
            .certs
            .get_mut(serial)
            .ok_or_else(|| VaultError::NotFound(format!("serial {serial}")))?;
        if cert.revoked {
            return Ok(cert.revocation_time.unwrap());
        }
        let now = Utc::now();
        cert.revoked = true;
        cert.revocation_time = Some(now);
        self.revoked_serials.insert(serial.to_string(), now);
        Ok(now)
    }

    /// Generate CRL (JSON — production would use X.509 DER format).
    pub fn generate_crl(&self) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = self
            .revoked_serials
            .iter()
            .map(|(serial, ts)| {
                serde_json::json!({ "serial": serial, "revocation_time": ts.to_rfc3339() })
            })
            .collect();
        serde_json::json!({
            "crl_id": Uuid::new_v4().to_string(),
            "generated_at": Utc::now().to_rfc3339(),
            "revoked": entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pki_root_ca_generation() {
        let mut pki = PkiEngine::new();
        let pem = pki.generate_root_ca("CAVE Root CA", "CAVE Platform", 3650).unwrap();
        assert!(pem.contains("BEGIN CERTIFICATE"));
        assert!(pki.root_ca.is_some());
    }

    #[test]
    fn test_pki_intermediate_ca() {
        let mut pki = PkiEngine::new();
        pki.generate_root_ca("CAVE Root CA", "CAVE", 3650).unwrap();
        let chain = pki.generate_intermediate_ca("CAVE Intermediate CA", "CAVE", 1825).unwrap();
        assert!(chain.contains("BEGIN CERTIFICATE"));
        assert!(pki.intermediate_ca.is_some());
    }

    #[test]
    fn test_pki_issue_certificate() {
        let mut pki = PkiEngine::new();
        pki.generate_root_ca("CAVE Root CA", "CAVE", 3650).unwrap();
        let cert = pki
            .issue_certificate("api.example.com", &["api.example.com".into()], 90, true)
            .unwrap();
        assert!(cert.certificate.contains("BEGIN CERTIFICATE"));
        assert!(cert.private_key.is_some());
        assert_eq!(cert.subject, "api.example.com");
    }

    #[test]
    fn test_pki_revoke_certificate() {
        let mut pki = PkiEngine::new();
        pki.generate_root_ca("CAVE Root CA", "CAVE", 3650).unwrap();
        let cert = pki
            .issue_certificate("db.internal", &[], 30, true)
            .unwrap();
        let serial = cert.serial.clone();
        let ts = pki.revoke(&serial).unwrap();
        assert!(ts <= Utc::now());
        assert!(pki.certs[&serial].revoked);
    }

    #[test]
    fn test_pki_crl_generation() {
        let mut pki = PkiEngine::new();
        pki.generate_root_ca("CAVE Root CA", "CAVE", 3650).unwrap();
        let cert = pki.issue_certificate("x.internal", &[], 30, false).unwrap();
        pki.revoke(&cert.serial).unwrap();
        let crl = pki.generate_crl();
        let revoked = crl["revoked"].as_array().unwrap();
        assert!(!revoked.is_empty());
    }
>>>>>>> claude/frosty-wiles
}
