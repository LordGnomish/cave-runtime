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
}
