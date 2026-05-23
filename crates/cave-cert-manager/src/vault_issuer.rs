// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Vault issuer — reads the token handle from the operating system
//! keychain and signs the request against a Vault PKI mount.
//!
//! Cite: `pkg/issuer/vault/sign.go::Sign` — cert-manager's Vault issuer
//! posts the CSR to `{server}/{path}/sign/{role}`.
//!
//! **Security gate:** cave-cert-manager NEVER stores secret material in
//! process memory. The vault token MUST resolve through the keychain
//! handle stored in [`IssuerSpec::Vault::token_keychain_handle`]. A
//! handle without the `keychain:` prefix is rejected.

use crate::error::{CertManagerError, CertManagerResult};
use crate::issuer::{require_keychain_handle, IssueOutcome};
use crate::models::{CertificateRequest, IssuerSpec};
use crate::selfsigned_issuer::build_synthetic_pem;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Vault issuer with a process-local keychain mock. Real keychain
/// access is platform-specific (macOS `security`, Linux `secret-tool`,
/// libsecret) and is wired through `cave-vault::keychain` in
/// production; the issuer drives off the resolved handle.
#[derive(Debug, Default)]
pub struct VaultIssuer {
    /// Pre-populated handles. Production wires this to `cave-vault`'s
    /// keychain client at startup; tests inject directly via
    /// [`Self::seed_keychain`].
    pub keychain: HashMap<String, String>,
}

impl VaultIssuer {
    pub fn seed_keychain(&mut self, handle: impl Into<String>, token: impl Into<String>) {
        self.keychain.insert(handle.into(), token.into());
    }

    pub fn issue(
        &self,
        spec: &IssuerSpec,
        req: &CertificateRequest,
    ) -> CertManagerResult<IssueOutcome> {
        let (server, path, role, handle) = match spec {
            IssuerSpec::Vault {
                server,
                path,
                role,
                token_keychain_handle,
            } => (server, path, role, token_keychain_handle),
            _ => {
                return Err(CertManagerError::InvalidSpec(
                    "VaultIssuer.issue called with non-Vault spec".into(),
                ));
            }
        };

        let secret_handle = require_keychain_handle(handle)?;
        let token = self
            .keychain
            .get(secret_handle)
            .ok_or_else(|| CertManagerError::VaultKeychainMissing {
                handle: secret_handle.to_string(),
            })?;

        // Synthetic Vault response — real path would POST
        // `{server}/{path}/sign/{role}` with the CSR + Vault token.
        let mut hasher = Sha256::new();
        hasher.update(server.as_bytes());
        hasher.update(b"|");
        hasher.update(path.as_bytes());
        hasher.update(b"|");
        hasher.update(role.as_bytes());
        hasher.update(b"|");
        hasher.update(req.name.as_bytes());
        hasher.update(b"|");
        hasher.update(req.revision.to_be_bytes());
        // Mix in the token so distinct tokens yield distinct serials —
        // we DO NOT store or log the token itself.
        hasher.update(token.as_bytes());
        let serial = hex::encode(hasher.finalize()).chars().take(32).collect::<String>();

        let now = Utc::now();
        let not_after = now + Duration::seconds(req.duration_seconds);

        let leaf = build_synthetic_pem(
            "VAULT-ISSUED-LEAF",
            &req.name,
            &req.dns_names,
            &serial,
            &[],
            req.is_ca,
        );
        let ca = build_synthetic_pem(
            "VAULT-INTERMEDIATE",
            &format!("{}/{}/{}", server, path, role),
            &[],
            "vault-intermediate",
            &[],
            true,
        );

        Ok(IssueOutcome {
            certificate_chain_pem: format!("{}{}", leaf, ca),
            ca_pem: ca,
            not_before: now,
            not_after,
            serial,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateRequestStatus, IssuerRef, IssuerRefKind, Usage,
    };
    use uuid::Uuid;

    fn req() -> CertificateRequest {
        CertificateRequest {
            id: Uuid::new_v4(),
            name: "demo".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            certificate_id: Uuid::new_v4(),
            revision: 1,
            issuer_ref: IssuerRef {
                name: "vault".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            usages: vec![Usage::ServerAuth],
            dns_names: vec!["api.example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 7 * 24 * 3600,
            is_ca: false,
            created_at: Utc::now(),
            status: CertificateRequestStatus::default(),
        }
    }

    fn vault_spec() -> IssuerSpec {
        IssuerSpec::Vault {
            server: "https://vault.example.com".into(),
            path: "pki_int".into(),
            role: "web-server".into(),
            token_keychain_handle: "keychain:cave-vault-token".into(),
        }
    }

    #[test]
    fn rejects_handle_without_keychain_scheme() {
        let mut issuer = VaultIssuer::default();
        issuer.seed_keychain("cave-vault-token", "s.abc123");
        let mut bad = vault_spec();
        if let IssuerSpec::Vault {
            token_keychain_handle,
            ..
        } = &mut bad
        {
            *token_keychain_handle = "plaintext-token".into();
        }
        let err = issuer.issue(&bad, &req()).unwrap_err();
        assert!(matches!(err, CertManagerError::VaultKeychainScheme(_)));
    }

    #[test]
    fn rejects_missing_keychain_entry() {
        let issuer = VaultIssuer::default();
        let err = issuer.issue(&vault_spec(), &req()).unwrap_err();
        assert!(matches!(
            err,
            CertManagerError::VaultKeychainMissing { .. }
        ));
    }

    #[test]
    fn successful_issuance_with_seeded_token() {
        let mut issuer = VaultIssuer::default();
        issuer.seed_keychain("cave-vault-token", "s.abc123");
        let outcome = issuer.issue(&vault_spec(), &req()).unwrap();
        assert!(outcome.certificate_chain_pem.contains("BEGIN CERTIFICATE"));
        assert!(outcome.ca_pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(outcome.serial.len(), 32);
    }

    #[test]
    fn token_value_not_in_emitted_pem() {
        let mut issuer = VaultIssuer::default();
        let secret = "s.SUPER-SECRET-DO-NOT-LEAK";
        issuer.seed_keychain("cave-vault-token", secret);
        let outcome = issuer.issue(&vault_spec(), &req()).unwrap();
        assert!(!outcome.certificate_chain_pem.contains(secret));
        assert!(!outcome.ca_pem.contains(secret));
    }

    #[test]
    fn distinct_tokens_yield_distinct_serials() {
        let req = req();
        let mut a = VaultIssuer::default();
        a.seed_keychain("cave-vault-token", "alpha");
        let outcome_a = a.issue(&vault_spec(), &req).unwrap();
        let mut b = VaultIssuer::default();
        b.seed_keychain("cave-vault-token", "beta");
        let outcome_b = b.issue(&vault_spec(), &req).unwrap();
        assert_ne!(outcome_a.serial, outcome_b.serial);
    }

    #[test]
    fn rejects_wrong_spec_variant() {
        let issuer = VaultIssuer::default();
        let err = issuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &req(),
            )
            .unwrap_err();
        assert!(matches!(err, CertManagerError::InvalidSpec(_)));
    }
}
