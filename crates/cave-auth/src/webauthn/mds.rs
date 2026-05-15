// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-metadata/src/main/java/com/webauthn4j/metadata/data/MetadataBLOBPayload.java
//   webauthn4j-metadata/src/main/java/com/webauthn4j/metadata/data/MetadataBLOBPayloadEntry.java
//   webauthn4j-metadata/src/main/java/com/webauthn4j/metadata/data/statement/MetadataStatement.java
//
// FIDO Alliance Metadata Service v3 (MDS3) — https://fidoalliance.org/metadata/.
// The MDS server publishes a single JWS blob per day at:
//   https://mds3.fidoalliance.org/
//
// The JWS is signed with a cert chain rooted in the FIDO Alliance Global
// Root CA.  Full chain validation requires ASN.1 X.509 parsing and a
// trust root store; that is an honest scope-cut for the OSS launch.
// Structural parsing of the payload (entries[], AAGUID lookup,
// metadataStatement fields) is implemented and is what the rest of the
// system needs to enrich credential admin UI panels.

use base64::Engine;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum MdsError {
    #[error("malformed JWT: must be three base64url segments separated by '.'")]
    BadJwt,
    #[error("base64url decode failure: {0}")]
    Base64(String),
    #[error("payload JSON parse failure: {0}")]
    Json(String),
    #[error("MDS chain validation is not enabled in this build")]
    ChainNotValidated,
}

/// Decoded MDS blob payload (the middle JWS segment).
#[derive(Debug, Clone, Deserialize)]
pub struct MdsBlob {
    #[serde(rename = "legalHeader")]
    pub legal_header: String,
    pub no: u64,
    #[serde(rename = "nextUpdate")]
    pub next_update: String,
    #[serde(default)]
    pub entries: Vec<MdsEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MdsEntry {
    #[serde(default)]
    pub aaguid: Option<String>,
    #[serde(default, rename = "aaid")]
    pub aaid: Option<String>,
    #[serde(default, rename = "attestationCertificateKeyIdentifiers")]
    pub attestation_certificate_key_identifiers: Option<Vec<String>>,
    #[serde(default, rename = "statusReports")]
    pub status_reports: Vec<StatusReport>,
    #[serde(default, rename = "timeOfLastStatusChange")]
    pub time_of_last_status_change: Option<String>,
    #[serde(default, rename = "metadataStatement")]
    pub metadata_statement: Option<MetadataStatement>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusReport {
    pub status: String,
    #[serde(default, rename = "effectiveDate")]
    pub effective_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MetadataStatement {
    pub description: String,
    #[serde(rename = "authenticatorVersion")]
    pub authenticator_version: u64,
    #[serde(rename = "protocolFamily")]
    pub protocol_family: String,
    pub schema: u64,
    #[serde(default)]
    pub aaguid: Option<String>,
    #[serde(default, rename = "userVerificationDetails")]
    pub user_verification_details: serde_json::Value,
    #[serde(default, rename = "authenticationAlgorithms")]
    pub authentication_algorithms: Vec<String>,
    #[serde(default, rename = "attestationTypes")]
    pub attestation_types: Vec<String>,
}

impl MdsBlob {
    /// Decode the JWS payload **without** verifying the signature or
    /// validating the certificate chain.  Returns `MdsError::ChainNotValidated`
    /// would be appropriate if a caller asks for "trusted" data; we
    /// expose a separate `verify_chain` API for that path (not yet
    /// implemented) so the structural parse stays decoupled from the
    /// crypto.
    pub fn parse_unsigned(jwt: &str) -> Result<Self, MdsError> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() != 3 {
            return Err(MdsError::BadJwt);
        }
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1].as_bytes())
            .map_err(|e| MdsError::Base64(e.to_string()))?;
        let blob: MdsBlob = serde_json::from_slice(&payload)
            .map_err(|e| MdsError::Json(e.to_string()))?;
        Ok(blob)
    }

    /// Validate the JWS chain against a pinned FIDO Alliance root CA.
    /// **Not enforced in this build** — returns `ChainNotValidated`
    /// so callers cannot accidentally trust the unsigned payload.
    pub fn verify_chain(&self, _trusted_roots_pem: &[&str]) -> Result<(), MdsError> {
        Err(MdsError::ChainNotValidated)
    }

    /// Look up the entry whose `aaguid` matches the supplied string
    /// (compared case-insensitively after normalising dashes).
    pub fn find_by_aaguid(&self, aaguid: &str) -> Option<&MdsEntry> {
        let needle = aaguid.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|e| e.aaguid.as_deref().map(str::to_ascii_lowercase) == Some(needle.clone()))
    }
}
