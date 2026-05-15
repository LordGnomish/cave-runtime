// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/WebAuthnRegistrationManager.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/RegistrationParameters.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/RegistrationData.java
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/RegistrationDataValidator.java
//
// Registration ceremony (W3C §7.1).  Two entry points:
//
//   1. `start_registration` — produces a `PublicKeyCredentialCreationOptions`
//      payload the JS caller serialises into `navigator.credentials.create()`.
//   2. `finish_registration` — verifies the `AuthenticatorAttestationResponse`
//      returned by the browser, validates attestation, and returns a
//      `Credential` record the RP must persist.

use crate::webauthn::attestation::{self, AttestationStatement};
use crate::webauthn::model::{AuthenticatorData, Credential, ParseError, Transport};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// User Verification policy from the RP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserVerificationRequirement {
    Required,
    Preferred,
    Discouraged,
}

impl UserVerificationRequirement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Preferred => "preferred",
            Self::Discouraged => "discouraged",
        }
    }
}

/// RP-side configuration for `start_registration`.
#[derive(Debug, Clone)]
pub struct RegistrationOptions {
    pub rp_id: String,
    pub rp_name: String,
    pub user_id: Vec<u8>,
    pub user_name: String,
    pub user_display_name: String,
    pub user_verification: UserVerificationRequirement,
    pub timeout_ms: u32,
    pub attestation: String, // none|indirect|direct|enterprise
}

/// Public surface returned by `start_registration` — mirrors the W3C
/// `PublicKeyCredentialCreationOptions` dictionary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PublicKeyCredentialCreationOptions {
    pub rp: Rp,
    pub user: PublicKeyCredentialUserEntity,
    pub challenge: Vec<u8>,
    pub pub_key_cred_params: Vec<PublicKeyCredentialParameters>,
    pub timeout: u32,
    pub attestation: String,
    pub authenticator_selection: AuthenticatorSelectionCriteria,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Rp {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PublicKeyCredentialUserEntity {
    pub id: Vec<u8>,
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PublicKeyCredentialParameters {
    pub r#type: &'static str,
    pub alg: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthenticatorSelectionCriteria {
    pub user_verification: &'static str,
    pub resident_key: &'static str,
}

/// Generate a 32-byte cryptographic challenge + canonical option dict.
pub fn start_registration(opts: &RegistrationOptions) -> PublicKeyCredentialCreationOptions {
    let mut challenge = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut challenge);
    PublicKeyCredentialCreationOptions {
        rp: Rp {
            id: opts.rp_id.clone(),
            name: opts.rp_name.clone(),
        },
        user: PublicKeyCredentialUserEntity {
            id: opts.user_id.clone(),
            name: opts.user_name.clone(),
            display_name: opts.user_display_name.clone(),
        },
        challenge,
        pub_key_cred_params: vec![
            PublicKeyCredentialParameters {
                r#type: "public-key",
                alg: -7,
            },
            PublicKeyCredentialParameters {
                r#type: "public-key",
                alg: -35,
            },
            PublicKeyCredentialParameters {
                r#type: "public-key",
                alg: -257,
            },
            PublicKeyCredentialParameters {
                r#type: "public-key",
                alg: -8,
            },
        ],
        timeout: opts.timeout_ms,
        attestation: opts.attestation.clone(),
        authenticator_selection: AuthenticatorSelectionCriteria {
            user_verification: opts.user_verification.as_str(),
            resident_key: "preferred",
        },
    }
}

/// Inputs to `finish_registration` — what the RP submits server-side
/// after the JS layer hands back the attestation response.
#[derive(Debug, Clone)]
pub struct RegistrationRequest {
    pub challenge: Vec<u8>,
    pub expected_origins: Vec<String>,
    pub rp_id: String,
    pub require_user_verification: bool,
    pub client_data_json: Vec<u8>,
    pub attestation_object: Vec<u8>,
    pub client_extension_results: serde_json::Value,
}

/// Verified result of a registration ceremony.
#[derive(Debug, Clone)]
pub struct RegistrationResult {
    pub credential: Credential,
    pub attestation_format: String,
    pub attestation_trust_path: AttestationTrustPath,
}

/// Trust path the attestation verifier returned (W3C §8.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationTrustPath {
    /// No attestation (None / Self).
    None,
    /// Basic / Privacy CA — DER-encoded X.509 chain.
    X5c(Vec<Vec<u8>>),
    /// Self attestation — credential key signed itself.
    SelfAttested,
    /// ECDAA — deprecated path, not enforced.
    Ecdaa,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("clientDataJSON is not valid UTF-8 JSON: {0}")]
    BadClientDataJson(String),
    #[error("clientData.type is {got:?}, expected webauthn.create")]
    WrongType { got: String },
    #[error("clientData.challenge does not match expected challenge")]
    ChallengeMismatch,
    #[error("clientData.origin {got:?} not in allow-list {allowed:?}")]
    OriginMismatch { got: String, allowed: Vec<String> },
    #[error("attestationObject is not valid CBOR: {0}")]
    BadAttestationObject(String),
    #[error("attestationObject has no `fmt` field")]
    MissingFormat,
    #[error("attestationObject has no `authData` field")]
    MissingAuthData,
    #[error("attestationObject has no `attStmt` field")]
    MissingAttStmt,
    #[error("authData parse failure: {0}")]
    AuthData(#[from] ParseError),
    #[error("rpIdHash does not match SHA-256(rpId)")]
    RpIdHashMismatch,
    #[error("authData.flags.UP=0 — user-presence required")]
    UserNotPresent,
    #[error("authData.flags.UV=0 but user-verification was required")]
    UserVerificationRequired,
    #[error("authData has no attestedCredentialData")]
    MissingAttestedCredentialData,
    #[error("attestation format {0:?} is not supported")]
    UnsupportedFormat(String),
    #[error("attestation statement failed verification: {0}")]
    AttestationFailed(String),
}

#[derive(serde::Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    type_: String,
    challenge: String,
    origin: String,
    #[serde(default, rename = "crossOrigin")]
    _cross_origin: Option<bool>,
}

/// Run the registration ceremony.  Returns a `Credential` the RP must
/// persist (id, public key, counter, flags, format).
pub fn finish_registration(req: RegistrationRequest) -> Result<RegistrationResult, RegistrationError> {
    // §7.1 step 5-9: parse clientDataJSON.
    let client_data: ClientData = serde_json::from_slice(&req.client_data_json)
        .map_err(|e| RegistrationError::BadClientDataJson(e.to_string()))?;
    if client_data.type_ != "webauthn.create" {
        return Err(RegistrationError::WrongType {
            got: client_data.type_,
        });
    }
    let client_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(client_data.challenge.as_bytes())
        .map_err(|e| RegistrationError::BadClientDataJson(e.to_string()))?;
    if client_challenge != req.challenge {
        return Err(RegistrationError::ChallengeMismatch);
    }
    if !req
        .expected_origins
        .iter()
        .any(|allowed| allowed == &client_data.origin)
    {
        return Err(RegistrationError::OriginMismatch {
            got: client_data.origin,
            allowed: req.expected_origins.clone(),
        });
    }

    // §7.1 step 10: parse attestationObject CBOR.
    let att_value: ciborium::Value = ciborium::de::from_reader(&req.attestation_object[..])
        .map_err(|e| RegistrationError::BadAttestationObject(e.to_string()))?;
    let map = match att_value {
        ciborium::Value::Map(m) => m,
        _ => return Err(RegistrationError::BadAttestationObject("not a CBOR map".into())),
    };
    let mut fmt: Option<String> = None;
    let mut auth_data_bytes: Option<Vec<u8>> = None;
    let mut att_stmt: Option<ciborium::Value> = None;
    for (k, v) in map.into_iter() {
        match k {
            ciborium::Value::Text(t) if t == "fmt" => {
                if let ciborium::Value::Text(s) = v {
                    fmt = Some(s);
                }
            }
            ciborium::Value::Text(t) if t == "authData" => {
                if let ciborium::Value::Bytes(b) = v {
                    auth_data_bytes = Some(b);
                }
            }
            ciborium::Value::Text(t) if t == "attStmt" => {
                att_stmt = Some(v);
            }
            _ => {}
        }
    }
    let fmt = fmt.ok_or(RegistrationError::MissingFormat)?;
    let auth_data_bytes = auth_data_bytes.ok_or(RegistrationError::MissingAuthData)?;
    let att_stmt = att_stmt.ok_or(RegistrationError::MissingAttStmt)?;

    // §7.1 step 11: parse authData.
    let auth_data = AuthenticatorData::parse(&auth_data_bytes)?;

    // §7.1 step 12: verify rpIdHash.
    let expected_rp_id_hash = Sha256::digest(req.rp_id.as_bytes());
    if expected_rp_id_hash.as_slice() != auth_data.rp_id_hash {
        return Err(RegistrationError::RpIdHashMismatch);
    }
    // §7.1 step 13-14: flag checks.
    if !auth_data.flags.user_present {
        return Err(RegistrationError::UserNotPresent);
    }
    if req.require_user_verification && !auth_data.flags.user_verified {
        return Err(RegistrationError::UserVerificationRequired);
    }

    let attested = auth_data
        .attested_credential_data
        .clone()
        .ok_or(RegistrationError::MissingAttestedCredentialData)?;

    // §7.1 step 19: dispatch attestation format.
    let client_data_hash = Sha256::digest(&req.client_data_json);
    let stmt = AttestationStatement {
        fmt: fmt.clone(),
        att_stmt,
        auth_data_bytes: auth_data_bytes.clone(),
        client_data_hash: client_data_hash.into(),
        attested: attested.clone(),
    };
    let trust_path = attestation::verify(&stmt)
        .map_err(|e| RegistrationError::AttestationFailed(e.to_string()))?;

    let credential = Credential {
        credential_id: attested.credential_id,
        public_key: attested.public_key,
        sign_counter: auth_data.sign_count,
        transports: Vec::<Transport>::new(),
        aaguid: attested.aaguid,
        attestation_format: fmt.clone(),
        user_handle: None,
        backup_eligible: auth_data.flags.backup_eligibility,
        backup_state: auth_data.flags.backup_state,
        uv_initialized: auth_data.flags.user_verified,
    };

    Ok(RegistrationResult {
        credential,
        attestation_format: fmt,
        attestation_trust_path: trust_path,
    })
}
