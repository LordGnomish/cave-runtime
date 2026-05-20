// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// Assertion (login) ceremony — webauthn4j `WebAuthnAuthenticationManager`.
//
// W3C §7.2 Verifying an Authentication Assertion:
//   1.  client returns AuthenticatorAssertionResponse { credentialId,
//       clientDataJSON, authenticatorData, signature, userHandle }.
//   2.  server looks up credentialId in the credential store.
//   3.  server parses clientDataJSON, verifies type="webauthn.get",
//       challenge, origin, !crossOrigin.
//   4.  server parses authenticatorData.
//   5.  server verifies authData.rpIdHash == SHA-256(rp_id).
//   6.  server verifies UP, UV (if required).
//   7.  server reconstructs signed payload = authenticatorData ||
//       SHA-256(clientDataJSON), verifies signature with stored public key.
//   8.  server checks signCount monotonicity, persists new signCount.

use sha2::{Digest, Sha256};

use super::WebAuthnError;
use super::authenticator_data::{self, AuthFlags};
use super::client_data::{self, ClientDataType};
use super::cose;
use super::credential_store::{CredentialStore, StoredCredential};
use super::registration::UserVerification;

#[derive(Debug, Clone)]
pub struct AssertionOptions {
    pub challenge: Vec<u8>,
    pub rp_id: String,
    pub user_verification: UserVerification,
    /// Optional allow-list of credential IDs (empty = any registered credential).
    pub allow_credentials: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct AssertionResponse {
    pub credential_id: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    /// User handle returned by discoverable-credential authenticators.
    pub user_handle: Option<Vec<u8>>,
}

pub struct AuthenticationManager<S: CredentialStore> {
    pub store: S,
    pub rp_id: String,
    pub origin: String,
}

impl<S: CredentialStore> AuthenticationManager<S> {
    pub fn new(store: S, rp_id: impl Into<String>, origin: impl Into<String>) -> Self {
        Self {
            store,
            rp_id: rp_id.into(),
            origin: origin.into(),
        }
    }

    /// Drive the assertion ceremony — webauthn4j
    /// `WebAuthnAuthenticationManager#verify`.
    pub fn verify(
        &self,
        opts: &AssertionOptions,
        resp: &AssertionResponse,
    ) -> Result<StoredCredential, WebAuthnError> {
        // §7.2 step 1 — credential lookup.
        let mut stored = self
            .store
            .get(&resp.credential_id)?
            .ok_or_else(|| WebAuthnError::CredentialNotFound(hex::encode(&resp.credential_id)))?;

        // §7.2 step 2 — allow-list (if present).
        if !opts.allow_credentials.is_empty()
            && !opts
                .allow_credentials
                .iter()
                .any(|c| c == &resp.credential_id)
        {
            return Err(WebAuthnError::Authentication(
                "credential id not in allowCredentials".into(),
            ));
        }

        // §7.2 step 3-6 — clientDataJSON.
        let cd = client_data::parse(&resp.client_data_json)?;
        client_data::verify(&cd, ClientDataType::Get, &opts.challenge, &self.origin)?;

        // §7.2 step 7 — authenticatorData.
        let auth = authenticator_data::parse(&resp.authenticator_data)?;
        let expected_rp_hash = authenticator_data::rp_id_hash(&self.rp_id);
        if auth.rp_id_hash != expected_rp_hash {
            return Err(WebAuthnError::Authentication(
                "rpIdHash != SHA-256(rp_id)".into(),
            ));
        }

        if !auth.flags.contains(AuthFlags::UP) {
            return Err(WebAuthnError::Authentication("UP flag not set".into()));
        }
        if opts.user_verification == UserVerification::Required
            && !auth.flags.contains(AuthFlags::UV)
        {
            return Err(WebAuthnError::Authentication(
                "UV required but not set".into(),
            ));
        }

        // §7.2 step 10 — signature check.
        let mut hasher = Sha256::new();
        hasher.update(&resp.client_data_json);
        let cd_hash: [u8; 32] = hasher.finalize().into();
        let mut signed = Vec::with_capacity(resp.authenticator_data.len() + cd_hash.len());
        signed.extend_from_slice(&resp.authenticator_data);
        signed.extend_from_slice(&cd_hash);
        cose::verify(&stored.public_key, &signed, &resp.signature)?;

        // §7.2 step 11 — sign-count.
        // If both stored and incoming are zero, no replay check applies (W3C
        // explicitly allows this). Otherwise incoming MUST be greater.
        if !(stored.sign_count == 0 && auth.sign_count == 0) && auth.sign_count <= stored.sign_count
        {
            return Err(WebAuthnError::Authentication(format!(
                "sign_count replay: stored={} incoming={}",
                stored.sign_count, auth.sign_count
            )));
        }
        // §7.2 step 11.a — persist new count (skip when both are zero).
        if auth.sign_count > 0 {
            self.store
                .update_sign_count(&resp.credential_id, auth.sign_count)?;
            stored.sign_count = auth.sign_count;
        }

        // §7.2 — backup-state-change detection (passkey roaming).
        let bs_now = auth.flags.contains(AuthFlags::BS);
        stored.backup_state = bs_now;
        // Sanity: BE flag cannot transition from false→true post-registration.
        if !stored.backup_eligible && auth.flags.contains(AuthFlags::BE) {
            return Err(WebAuthnError::Authentication(
                "BE flag changed false->true after registration".into(),
            ));
        }
        Ok(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webauthn::authenticator_data::AuthFlags;
    use crate::webauthn::cose::CoseKey;
    use crate::webauthn::credential_store::{InMemoryCredentialStore, StoredCredential};
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn b64u(s: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
    }

    fn client_data_get(challenge: &[u8], origin: &str) -> Vec<u8> {
        format!(
            r#"{{"type":"webauthn.get","challenge":"{ch}","origin":"{origin}","crossOrigin":false}}"#,
            ch = b64u(challenge),
            origin = origin
        )
        .into_bytes()
    }

    fn auth_data_for(rp_id: &str, flags: AuthFlags, sign_count: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&authenticator_data::rp_id_hash(rp_id));
        v.push(flags.bits());
        v.extend_from_slice(&sign_count.to_be_bytes());
        v
    }

    fn setup() -> (
        SigningKey,
        AuthenticationManager<InMemoryCredentialStore>,
        Vec<u8>,
    ) {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let store = InMemoryCredentialStore::new();
        store
            .put(StoredCredential {
                credential_id: b"cred-A".to_vec(),
                user_handle: b"user-1".to_vec(),
                public_key: CoseKey::EdDsa { x: vk.to_bytes() },
                public_key_raw: vec![],
                sign_count: 0,
                aaguid: [0; 16],
                backup_eligible: true,
                backup_state: true,
                transports: vec!["internal".into()],
            })
            .unwrap();
        let mgr = AuthenticationManager::new(store, "login.cave.dev", "https://login.cave.dev");
        (sk, mgr, b"cred-A".to_vec())
    }

    #[test]
    fn assertion_happy_path_eddsa() {
        let (sk, mgr, cred_id) = setup();
        let auth_data = auth_data_for(
            "login.cave.dev",
            AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
            5,
        );
        let challenge = b"login-challenge".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let mut hasher = Sha256::new();
        hasher.update(&cd);
        let cd_hash: [u8; 32] = hasher.finalize().into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cd_hash);
        let sig = sk.sign(&signed);

        let stored = mgr
            .verify(
                &AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: UserVerification::Required,
                    allow_credentials: vec![cred_id.clone()],
                },
                &AssertionResponse {
                    credential_id: cred_id.clone(),
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: Some(b"user-1".to_vec()),
                },
            )
            .unwrap();
        assert_eq!(stored.credential_id, cred_id);
        assert_eq!(stored.sign_count, 5);
    }

    #[test]
    fn assertion_rejects_unknown_credential() {
        let (_sk, mgr, _id) = setup();
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let auth_data = auth_data_for("login.cave.dev", AuthFlags::UP, 1);
        assert!(matches!(
            mgr.verify(
                &AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: UserVerification::Discouraged,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: b"unknown".to_vec(),
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: vec![],
                    user_handle: None,
                },
            ),
            Err(WebAuthnError::CredentialNotFound(_))
        ));
    }

    #[test]
    fn assertion_rejects_signcount_regression() {
        let (sk, mgr, cred_id) = setup();
        // Bump store to count=10 first.
        mgr.store.update_sign_count(&cred_id, 10).unwrap();
        let auth_data = auth_data_for(
            "login.cave.dev",
            AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
            5,
        );
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let mut hasher = Sha256::new();
        hasher.update(&cd);
        let cd_hash: [u8; 32] = hasher.finalize().into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cd_hash);
        let sig = sk.sign(&signed);
        assert!(
            mgr.verify(
                &AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: UserVerification::Required,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: cred_id,
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: None,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn assertion_rejects_tampered_signature() {
        let (sk, mgr, cred_id) = setup();
        let auth_data = auth_data_for("login.cave.dev", AuthFlags::UP, 3);
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let sig = sk.sign(b"something-else");
        assert!(
            mgr.verify(
                &AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: UserVerification::Discouraged,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: cred_id,
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: None,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn assertion_rejects_disallowed_credential() {
        let (sk, mgr, cred_id) = setup();
        let auth_data = auth_data_for("login.cave.dev", AuthFlags::UP, 3);
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let mut hasher = Sha256::new();
        hasher.update(&cd);
        let cd_hash: [u8; 32] = hasher.finalize().into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cd_hash);
        let sig = sk.sign(&signed);
        assert!(
            mgr.verify(
                &AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: UserVerification::Discouraged,
                    allow_credentials: vec![b"other-cred".to_vec()],
                },
                &AssertionResponse {
                    credential_id: cred_id,
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: None,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn assertion_rejects_wrong_origin() {
        let (sk, mgr, cred_id) = setup();
        let auth_data = auth_data_for("login.cave.dev", AuthFlags::UP, 3);
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://evil.example");
        let mut hasher = Sha256::new();
        hasher.update(&cd);
        let cd_hash: [u8; 32] = hasher.finalize().into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cd_hash);
        let sig = sk.sign(&signed);
        assert!(
            mgr.verify(
                &AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: UserVerification::Discouraged,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: cred_id,
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: None,
                }
            )
            .is_err()
        );
    }
}
