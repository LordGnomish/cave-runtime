// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// Discoverable credentials (passkeys) — webauthn4j `passkey` flow + Keycloak
// `WebAuthnPasswordlessAuthenticator`.
//
// A "passkey" is a credential that:
//   - was registered with `residentKey="required"` (or "preferred") AND
//   - has the BE flag set (backup-eligible), AND
//   - typically has the BS flag set (currently backed up).
//
// The passwordless / username-less assertion flow lets the user pick the
// account themselves — the authenticator returns the `userHandle` inside
// `AuthenticatorAssertionResponse`. We use that to drive the lookup.

use super::WebAuthnError;
use super::authentication::{AssertionResponse, AuthenticationManager};
use super::credential_store::{CredentialStore, StoredCredential};

/// Outcome of a discoverable-credential ceremony — includes the resolved
/// user handle alongside the stored credential row.
#[derive(Debug, Clone)]
pub struct DiscoverableAssertion {
    pub user_handle: Vec<u8>,
    pub credential: StoredCredential,
}

impl<S: CredentialStore> AuthenticationManager<S> {
    /// Discoverable-credential entry point — Keycloak
    /// `WebAuthnPasswordlessAuthenticator#processAuthenticate`.
    pub fn verify_discoverable(
        &self,
        opts: &super::authentication::AssertionOptions,
        resp: &AssertionResponse,
    ) -> Result<DiscoverableAssertion, WebAuthnError> {
        // §7.2 — passkey ceremonies REQUIRE the client to surface user_handle.
        let user_handle = resp
            .user_handle
            .as_ref()
            .ok_or_else(|| {
                WebAuthnError::Authentication("passkey ceremony missing userHandle".into())
            })?
            .clone();

        // Sanity-check: stored credential's user_handle MUST match the
        // user_handle the client returned. Prevents a hostile authenticator
        // from spoofing identity.
        let stored = self
            .store
            .get(&resp.credential_id)?
            .ok_or_else(|| WebAuthnError::CredentialNotFound(hex::encode(&resp.credential_id)))?;
        if stored.user_handle != user_handle {
            return Err(WebAuthnError::Authentication(
                "userHandle mismatch with stored credential".into(),
            ));
        }
        // Run the normal assertion ceremony.
        let cred = self.verify(opts, resp)?;
        Ok(DiscoverableAssertion {
            user_handle,
            credential: cred,
        })
    }
}

/// Return all discoverable credentials registered for a user — used by the
/// `/admin/auth/webauthn` portal page.
pub fn list_passkeys<S: CredentialStore>(
    store: &S,
    user_handle: &[u8],
) -> Result<Vec<StoredCredential>, WebAuthnError> {
    let all = store.list_by_user(user_handle)?;
    Ok(all.into_iter().filter(|c| c.backup_eligible).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webauthn::authenticator_data::AuthFlags;
    use crate::webauthn::authenticator_data::{self};
    use crate::webauthn::cose::CoseKey;
    use crate::webauthn::credential_store::InMemoryCredentialStore;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use sha2::{Digest, Sha256};

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

    fn setup_passkey() -> (
        SigningKey,
        AuthenticationManager<InMemoryCredentialStore>,
        Vec<u8>,
        Vec<u8>,
    ) {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let store = InMemoryCredentialStore::new();
        store
            .put(StoredCredential {
                credential_id: b"passkey-1".to_vec(),
                user_handle: b"user-7".to_vec(),
                public_key: CoseKey::EdDsa { x: vk.to_bytes() },
                public_key_raw: vec![],
                sign_count: 0,
                aaguid: [0xab; 16],
                backup_eligible: true,
                backup_state: true,
                transports: vec!["internal".into(), "hybrid".into()],
            })
            .unwrap();
        let mgr = AuthenticationManager::new(store, "login.cave.dev", "https://login.cave.dev");
        (sk, mgr, b"passkey-1".to_vec(), b"user-7".to_vec())
    }

    #[test]
    fn passkey_discoverable_assertion_succeeds() {
        let (sk, mgr, cid, uid) = setup_passkey();
        let auth_data = auth_data_for(
            "login.cave.dev",
            AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
            1,
        );
        let challenge = b"passkey-chal".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let mut hasher = Sha256::new();
        hasher.update(&cd);
        let cdh: [u8; 32] = hasher.finalize().into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cdh);
        let sig = sk.sign(&signed);
        let out = mgr
            .verify_discoverable(
                &super::super::authentication::AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: super::super::registration::UserVerification::Required,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: cid.clone(),
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: Some(uid.clone()),
                },
            )
            .unwrap();
        assert_eq!(out.user_handle, uid);
        assert_eq!(out.credential.credential_id, cid);
    }

    #[test]
    fn passkey_missing_user_handle_errors() {
        let (sk, mgr, cid, _uid) = setup_passkey();
        let auth_data = auth_data_for(
            "login.cave.dev",
            AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
            1,
        );
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let sig = sk.sign(b"anything");
        assert!(
            mgr.verify_discoverable(
                &super::super::authentication::AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: super::super::registration::UserVerification::Required,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: cid,
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: None,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn passkey_mismatched_user_handle_errors() {
        let (sk, mgr, cid, _uid) = setup_passkey();
        let auth_data = auth_data_for(
            "login.cave.dev",
            AuthFlags::UP | AuthFlags::UV | AuthFlags::BE | AuthFlags::BS,
            1,
        );
        let challenge = b"x".to_vec();
        let cd = client_data_get(&challenge, "https://login.cave.dev");
        let sig = sk.sign(b"x");
        assert!(
            mgr.verify_discoverable(
                &super::super::authentication::AssertionOptions {
                    challenge,
                    rp_id: "login.cave.dev".into(),
                    user_verification: super::super::registration::UserVerification::Required,
                    allow_credentials: vec![],
                },
                &AssertionResponse {
                    credential_id: cid,
                    client_data_json: cd,
                    authenticator_data: auth_data,
                    signature: sig.to_bytes().to_vec(),
                    user_handle: Some(b"wrong-user".to_vec()),
                },
            )
            .is_err()
        );
    }

    #[test]
    fn list_passkeys_returns_only_backup_eligible() {
        let store = InMemoryCredentialStore::new();
        // Backup-eligible.
        store
            .put(StoredCredential {
                credential_id: b"a".to_vec(),
                user_handle: b"u".to_vec(),
                public_key: CoseKey::EdDsa { x: [0; 32] },
                public_key_raw: vec![],
                sign_count: 0,
                aaguid: [0; 16],
                backup_eligible: true,
                backup_state: true,
                transports: vec![],
            })
            .unwrap();
        // Non-backup-eligible.
        store
            .put(StoredCredential {
                credential_id: b"b".to_vec(),
                user_handle: b"u".to_vec(),
                public_key: CoseKey::EdDsa { x: [0; 32] },
                public_key_raw: vec![],
                sign_count: 0,
                aaguid: [0; 16],
                backup_eligible: false,
                backup_state: false,
                transports: vec![],
            })
            .unwrap();
        let pk = list_passkeys(&store, b"u").unwrap();
        assert_eq!(pk.len(), 1);
        assert_eq!(pk[0].credential_id, b"a".to_vec());
    }
}
