// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JWKS publication — `/realms/{realm}/protocol/openid-connect/certs`.
//!
//! Upstream: `services/src/main/java/org/keycloak/protocol/oidc/endpoints/JWKSResource.java`.

use serde::{Deserialize, Serialize};

use crate::signer::SignerRegistry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwksDocument {
    pub keys: Vec<serde_json::Value>,
}

pub fn jwks_for(realm_id: &str, signer: &SignerRegistry) -> JwksDocument {
    let raw = signer.jwks(realm_id);
    let keys: Vec<_> = raw["keys"].as_array().cloned().unwrap_or_default();
    JwksDocument { keys }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::SigningKeyEntry;

    #[test]
    fn jwks_collects_realm_keys() {
        let reg = SignerRegistry::default();
        reg.install(
            "r1",
            SigningKeyEntry::es256_from_seed("k1", &[1u8; 32]).unwrap(),
            true,
        );
        reg.install(
            "r1",
            SigningKeyEntry::eddsa_from_seed("k2", &[2u8; 32]),
            false,
        );
        let doc = jwks_for("r1", &reg);
        assert_eq!(doc.keys.len(), 2);
    }

    #[test]
    fn jwks_is_realm_scoped() {
        let reg = SignerRegistry::default();
        reg.install(
            "r1",
            SigningKeyEntry::es256_from_seed("k1", &[1u8; 32]).unwrap(),
            true,
        );
        reg.install(
            "r2",
            SigningKeyEntry::es256_from_seed("k2", &[2u8; 32]).unwrap(),
            true,
        );
        let r1 = jwks_for("r1", &reg);
        let r2 = jwks_for("r2", &reg);
        assert_eq!(r1.keys.len(), 1);
        assert_eq!(r2.keys.len(), 1);
        assert_ne!(r1.keys[0]["kid"], r2.keys[0]["kid"]);
    }
}
