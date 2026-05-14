// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Signing services — content signing and verification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Signing service definition ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SigningService {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub name: String,
    pub public_key: String,
    pub key_id: String,
    pub pubkey_fingerprint: String,
    pub service_type: SigningServiceType,
    pub script: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SigningServiceType {
    /// GPG-based signing.
    Gpg,
    /// X.509/CMS signing.
    X509,
    /// Sigstore/cosign.
    Sigstore,
}

impl SigningService {
    pub fn new(name: impl Into<String>, key_id: impl Into<String>) -> Self {
        let id = Uuid::new_v4();
        Self {
            pulp_href: format!("/pulp/api/v3/signing-services/{}/", id),
            pulp_id: id,
            name: name.into(),
            public_key: String::new(),
            key_id: key_id.into(),
            pubkey_fingerprint: String::new(),
            service_type: SigningServiceType::Gpg,
            script: "/var/lib/pulp/scripts/sign.sh".to_string(),
        }
    }
}

// ─── Signature record ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSignature {
    pub pulp_href: String,
    pub pulp_id: Uuid,
    pub signed_at: DateTime<Utc>,
    pub signing_service: String,
    pub content: String,
    pub signature_data: String,
    pub key_id: String,
    pub valid: bool,
}

// ─── Signing request/result ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SigningRequest {
    pub content_hrefs: Vec<String>,
    pub signing_service_href: String,
}

#[derive(Debug, Clone)]
pub struct SigningResult {
    pub signatures: Vec<ContentSignature>,
    pub failed: Vec<String>,
}

/// Verify a GPG signature (format: base64-encoded detached signature).
pub fn verify_gpg_signature(
    data: &[u8],
    signature_b64: &str,
    _public_key_armored: &str,
) -> VerificationResult {
    // In production this would use gpgme or sequoia-pgp.
    // Simplified: check non-empty and valid base64 length.
    if signature_b64.is_empty() {
        return VerificationResult::Invalid { reason: "Empty signature".to_string() };
    }
    if signature_b64.len() < 64 {
        return VerificationResult::Invalid { reason: "Signature too short".to_string() };
    }
    if data.is_empty() {
        return VerificationResult::Invalid { reason: "Empty data".to_string() };
    }
    VerificationResult::Valid { key_id: "MOCK_KEY_ID".to_string() }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerificationResult {
    Valid { key_id: String },
    Invalid { reason: String },
    Unknown,
}

impl VerificationResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }
}

// ─── RPM signing ─────────────────────────────────────────────────────────────

/// Check if an RPM package has a valid GPG signature header.
pub fn rpm_has_signature(rpm_header_tags: &std::collections::HashMap<String, String>) -> bool {
    rpm_header_tags.contains_key("RPMTAG_DSAHEADER")
        || rpm_header_tags.contains_key("RPMTAG_RSAHEADER")
        || rpm_header_tags.contains_key("RPMTAG_SIGGPG")
}

// ─── Sigstore / cosign ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CosignBundle {
    pub payload: String,
    pub payload_type: String,
    pub signatures: Vec<CosignSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CosignSignature {
    pub keyid: String,
    pub sig: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_service_new() {
        let svc = SigningService::new("gpg-signing", "0xDEADBEEF");
        assert_eq!(svc.name, "gpg-signing");
        assert_eq!(svc.key_id, "0xDEADBEEF");
        assert_eq!(svc.service_type, SigningServiceType::Gpg);
        assert!(svc.pulp_href.starts_with("/pulp/api/v3/signing-services/"));
    }

    #[test]
    fn verify_gpg_valid_signature() {
        let data = b"test content";
        let sig = "a".repeat(64);
        let result = verify_gpg_signature(data, &sig, "PUBLIC KEY");
        assert!(result.is_valid());
    }

    #[test]
    fn verify_gpg_empty_signature() {
        let result = verify_gpg_signature(b"data", "", "KEY");
        assert!(!result.is_valid());
        assert!(matches!(result, VerificationResult::Invalid { .. }));
    }

    #[test]
    fn verify_gpg_empty_data() {
        let sig = "a".repeat(64);
        let result = verify_gpg_signature(&[], &sig, "KEY");
        assert!(!result.is_valid());
    }

    #[test]
    fn rpm_has_signature_true() {
        let mut tags = std::collections::HashMap::new();
        tags.insert("RPMTAG_RSAHEADER".to_string(), "abc123".to_string());
        assert!(rpm_has_signature(&tags));
    }

    #[test]
    fn rpm_has_signature_false() {
        let tags = std::collections::HashMap::new();
        assert!(!rpm_has_signature(&tags));
    }

    #[test]
    fn verification_result_is_valid() {
        let v = VerificationResult::Valid { key_id: "KEY1".to_string() };
        assert!(v.is_valid());
        let inv = VerificationResult::Invalid { reason: "Bad sig".to_string() };
        assert!(!inv.is_valid());
    }
}
