// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/WebAuthnAuthenticationManager.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/AuthenticationParameters.java
//   webauthn4j-core/src/main/java/com/webauthn4j/data/AuthenticationData.java
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/AuthenticationDataValidator.java
//
// Authentication ceremony (W3C §7.2).

use crate::webauthn::attestation::AttestationError;
use crate::webauthn::attestation::packed::verify_signature;
use crate::webauthn::model::{AuthenticatorData, Credential, ParseError};
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct AuthenticationOptions {
    pub rp_id: String,
    pub allow_credentials: Vec<Vec<u8>>,
    pub user_verification: String,
    pub timeout_ms: u32,
}

#[derive(Debug, Clone)]
pub struct PublicKeyCredentialRequestOptions {
    pub challenge: Vec<u8>,
    pub allow_credentials: Vec<Vec<u8>>,
    pub rp_id: String,
    pub user_verification: String,
    pub timeout: u32,
}

pub fn start_authentication(opts: &AuthenticationOptions) -> PublicKeyCredentialRequestOptions {
    let mut challenge = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut challenge);
    PublicKeyCredentialRequestOptions {
        challenge,
        allow_credentials: opts.allow_credentials.clone(),
        rp_id: opts.rp_id.clone(),
        user_verification: opts.user_verification.clone(),
        timeout: opts.timeout_ms,
    }
}

/// What the RP submits after the browser hands back the assertion.
#[derive(Debug, Clone)]
pub struct AuthenticationRequest {
    pub challenge: Vec<u8>,
    pub expected_origins: Vec<String>,
    pub rp_id: String,
    pub require_user_verification: bool,
    /// The Credential record looked up by `credentialId`.
    pub credential: Credential,
    pub authenticator_data: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub signature: Vec<u8>,
    /// Optional `userHandle` returned by the authenticator (W3C §6.3.3).
    pub user_handle: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct AuthenticationResult {
    pub credential_id: Vec<u8>,
    pub new_sign_counter: u32,
    pub user_verified: bool,
    pub backup_state: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthenticationError {
    #[error("clientDataJSON is not valid UTF-8 JSON: {0}")]
    BadClientDataJson(String),
    #[error("clientData.type is {got:?}, expected webauthn.get")]
    WrongType { got: String },
    #[error("clientData.challenge does not match expected challenge")]
    ChallengeMismatch,
    #[error("clientData.origin {got:?} not in allow-list {allowed:?}")]
    OriginMismatch { got: String, allowed: Vec<String> },
    #[error("authData parse failure: {0}")]
    AuthData(#[from] ParseError),
    #[error("rpIdHash does not match SHA-256(rpId)")]
    RpIdHashMismatch,
    #[error("authData.flags.UP=0 — user-presence required")]
    UserNotPresent,
    #[error("authData.flags.UV=0 but user-verification was required")]
    UserVerificationRequired,
    #[error("signCounter regression: stored={stored} got={got}")]
    CounterRegression { stored: u32, got: u32 },
    #[error("signature verification failed")]
    BadSignature,
    #[error("userHandle does not match stored credential")]
    UserHandleMismatch,
    #[error("signature verifier internal error: {0}")]
    Internal(String),
}

impl From<AttestationError> for AuthenticationError {
    fn from(value: AttestationError) -> Self {
        match value {
            AttestationError::BadSignature => Self::BadSignature,
            other => Self::Internal(other.to_string()),
        }
    }
}

#[derive(serde::Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    type_: String,
    challenge: String,
    origin: String,
}

pub fn finish_authentication(
    req: AuthenticationRequest,
) -> Result<AuthenticationResult, AuthenticationError> {
    // §7.2 step 11-14: clientDataJSON.
    let client_data: ClientData = serde_json::from_slice(&req.client_data_json)
        .map_err(|e| AuthenticationError::BadClientDataJson(e.to_string()))?;
    if client_data.type_ != "webauthn.get" {
        return Err(AuthenticationError::WrongType {
            got: client_data.type_,
        });
    }
    use base64::Engine;
    let client_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(client_data.challenge.as_bytes())
        .map_err(|e| AuthenticationError::BadClientDataJson(e.to_string()))?;
    if client_challenge != req.challenge {
        return Err(AuthenticationError::ChallengeMismatch);
    }
    if !req
        .expected_origins
        .iter()
        .any(|allowed| allowed == &client_data.origin)
    {
        return Err(AuthenticationError::OriginMismatch {
            got: client_data.origin,
            allowed: req.expected_origins.clone(),
        });
    }

    // §7.2 step 15: authData (no attestedCredentialData expected).
    let auth_data = AuthenticatorData::parse(&req.authenticator_data)?;

    // §7.2 step 16-17.
    let expected = Sha256::digest(req.rp_id.as_bytes());
    if expected.as_slice() != auth_data.rp_id_hash {
        return Err(AuthenticationError::RpIdHashMismatch);
    }
    if !auth_data.flags.user_present {
        return Err(AuthenticationError::UserNotPresent);
    }
    if req.require_user_verification && !auth_data.flags.user_verified {
        return Err(AuthenticationError::UserVerificationRequired);
    }

    // §7.2 step 6.1: passkey userHandle check.
    if let Some(stored) = &req.credential.user_handle {
        match &req.user_handle {
            Some(got) if got == stored => {}
            Some(_) => return Err(AuthenticationError::UserHandleMismatch),
            None => return Err(AuthenticationError::UserHandleMismatch),
        }
    }

    // §7.2 step 20: verify the assertion signature.
    let client_data_hash = Sha256::digest(&req.client_data_json);
    let mut signed = Vec::with_capacity(req.authenticator_data.len() + 32);
    signed.extend_from_slice(&req.authenticator_data);
    signed.extend_from_slice(&client_data_hash);
    verify_signature(&req.credential.public_key, &signed, &req.signature)?;

    // §7.2 step 21: signCounter must strictly advance unless both are 0.
    let stored = req.credential.sign_counter;
    let got = auth_data.sign_count;
    if !(got == 0 && stored == 0) && got <= stored {
        return Err(AuthenticationError::CounterRegression { stored, got });
    }

    Ok(AuthenticationResult {
        credential_id: req.credential.credential_id,
        new_sign_counter: got,
        user_verified: auth_data.flags.user_verified,
        backup_state: auth_data.flags.backup_state,
    })
}
