// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// Credential persistence — Keycloak `WebAuthnCredentialProvider` analogue.
//
// Cave-auth doesn't own a persistent backend yet (see parity manifest:
// `keycloak:models/jpa/`). The trait below lets the higher modules
// (`registration`, `authentication`, `resident_key`) write/read against
// either an in-memory store (tests + dev) or a future RDBMS/etcd backend.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use crate::webauthn::cose::CoseKey;
use crate::webauthn::WebAuthnError;

/// Stored credential record.
///
/// Field shape mirrors webauthn4j `Authenticator` + Keycloak's
/// `WebAuthnCredentialModel`. We keep the COSE public key as a parsed
/// `CoseKey` plus its raw CBOR bytes — the raw form is needed for byte-
/// stable persistence and for diagnostics; the parsed form is what
/// signature verification consumes.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub credential_id: Vec<u8>,
    /// Internal user handle. Empty for non-discoverable credentials.
    pub user_handle: Vec<u8>,
    pub public_key: CoseKey,
    pub public_key_raw: Vec<u8>,
    /// Signature counter — monotonically non-decreasing per W3C §6.1.1.
    pub sign_count: u32,
    /// AAGUID of the registering authenticator.
    pub aaguid: [u8; 16],
    /// Backup-eligible flag (BE).
    pub backup_eligible: bool,
    /// Backup-state flag at last seen ceremony (BS).
    pub backup_state: bool,
    /// "transports" hint from the client (e.g. ["usb","nfc","internal","hybrid"]).
    pub transports: Vec<String>,
}

/// Persistence trait. Implementations may be in-memory, etcd-backed, RDBMS.
pub trait CredentialStore: Send + Sync {
    fn put(&self, cred: StoredCredential) -> Result<(), WebAuthnError>;
    fn get(&self, credential_id: &[u8]) -> Result<Option<StoredCredential>, WebAuthnError>;
    fn list_by_user(&self, user_handle: &[u8]) -> Result<Vec<StoredCredential>, WebAuthnError>;
    fn update_sign_count(
        &self,
        credential_id: &[u8],
        new_count: u32,
    ) -> Result<(), WebAuthnError>;
    fn delete(&self, credential_id: &[u8]) -> Result<(), WebAuthnError>;
}

/// In-memory backend — primary use case is tests + ephemeral dev clusters.
#[derive(Default)]
pub struct InMemoryCredentialStore {
    inner: Mutex<HashMap<Vec<u8>, StoredCredential>>,
}

impl InMemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }
    fn lock(&self) -> Result<MutexGuard<'_, HashMap<Vec<u8>, StoredCredential>>, WebAuthnError> {
        self.inner
            .lock()
            .map_err(|e| WebAuthnError::Registration(format!("store lock poisoned: {e}")))
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn put(&self, cred: StoredCredential) -> Result<(), WebAuthnError> {
        let mut g = self.lock()?;
        g.insert(cred.credential_id.clone(), cred);
        Ok(())
    }

    fn get(&self, credential_id: &[u8]) -> Result<Option<StoredCredential>, WebAuthnError> {
        let g = self.lock()?;
        Ok(g.get(credential_id).cloned())
    }

    fn list_by_user(&self, user_handle: &[u8]) -> Result<Vec<StoredCredential>, WebAuthnError> {
        let g = self.lock()?;
        Ok(g.values()
            .filter(|c| c.user_handle == user_handle)
            .cloned()
            .collect())
    }

    fn update_sign_count(
        &self,
        credential_id: &[u8],
        new_count: u32,
    ) -> Result<(), WebAuthnError> {
        let mut g = self.lock()?;
        let entry = g
            .get_mut(credential_id)
            .ok_or_else(|| WebAuthnError::CredentialNotFound(hex::encode(credential_id)))?;
        if new_count < entry.sign_count {
            return Err(WebAuthnError::Authentication(format!(
                "sign_count regressed: stored={} incoming={}",
                entry.sign_count, new_count
            )));
        }
        entry.sign_count = new_count;
        Ok(())
    }

    fn delete(&self, credential_id: &[u8]) -> Result<(), WebAuthnError> {
        let mut g = self.lock()?;
        g.remove(credential_id)
            .ok_or_else(|| WebAuthnError::CredentialNotFound(hex::encode(credential_id)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy(id: u8, user: u8, count: u32) -> StoredCredential {
        StoredCredential {
            credential_id: vec![id],
            user_handle: vec![user],
            public_key: CoseKey::EdDsa { x: [id; 32] },
            public_key_raw: vec![],
            sign_count: count,
            aaguid: [0; 16],
            backup_eligible: false,
            backup_state: false,
            transports: vec!["internal".into()],
        }
    }

    #[test]
    fn put_then_get_roundtrip() {
        let s = InMemoryCredentialStore::new();
        s.put(dummy(1, 7, 0)).unwrap();
        let got = s.get(&[1]).unwrap().unwrap();
        assert_eq!(got.user_handle, vec![7]);
    }

    #[test]
    fn list_by_user_filters_correctly() {
        let s = InMemoryCredentialStore::new();
        s.put(dummy(1, 7, 0)).unwrap();
        s.put(dummy(2, 7, 0)).unwrap();
        s.put(dummy(3, 8, 0)).unwrap();
        let user7 = s.list_by_user(&[7]).unwrap();
        assert_eq!(user7.len(), 2);
        let user8 = s.list_by_user(&[8]).unwrap();
        assert_eq!(user8.len(), 1);
    }

    #[test]
    fn update_sign_count_accepts_monotone_increase() {
        let s = InMemoryCredentialStore::new();
        s.put(dummy(1, 7, 5)).unwrap();
        s.update_sign_count(&[1], 6).unwrap();
        assert_eq!(s.get(&[1]).unwrap().unwrap().sign_count, 6);
    }

    #[test]
    fn update_sign_count_rejects_regression() {
        let s = InMemoryCredentialStore::new();
        s.put(dummy(1, 7, 5)).unwrap();
        assert!(s.update_sign_count(&[1], 4).is_err());
    }

    #[test]
    fn delete_removes_record() {
        let s = InMemoryCredentialStore::new();
        s.put(dummy(1, 7, 0)).unwrap();
        s.delete(&[1]).unwrap();
        assert!(s.get(&[1]).unwrap().is_none());
    }

    #[test]
    fn delete_unknown_errors() {
        let s = InMemoryCredentialStore::new();
        assert!(matches!(
            s.delete(&[42]),
            Err(WebAuthnError::CredentialNotFound(_))
        ));
    }
}
