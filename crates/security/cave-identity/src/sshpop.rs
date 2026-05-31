// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). sshpop node-attestor proof-of-
// possession flow line-ported from pkg/common/plugin/sshpop + pkg/agent/plugin/
// nodeattestor/sshpop + pkg/server/plugin/nodeattestor/sshpop.
//
//! `sshpop` node attestor — proof-of-possession of an SSH host key.
//!
//! SPIRE's sshpop flow: an agent presents an SSH **host certificate** signed by
//! a trusted SSH CA; the server issues a random challenge nonce; the agent
//! proves possession of the host private key by signing the nonce. On success
//! the server derives a canonical agent SPIFFE ID from the host-key
//! fingerprint and emits `sshpop:` selectors (the certificate principals +
//! the fingerprint).
//!
//! This is real ed25519 cryptography (no placeholder) — the `tpm_devid`
//! sibling stays a Charter-scope_cut because it requires a hardware TPM the
//! sandbox cannot exercise. We model the certificate as the CA-signed tuple
//! `(host_public_key, principals)`; the full OpenSSH wire `Certificate`
//! encoding is delegated to the agent's SSH stack.

use crate::error::{IdentityError, Result};
use crate::models::{Selector, SpiffeId, TrustDomain};
use crate::spiffe_id::agent_id;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// An SSH host certificate, reduced to the CA-signed fields sshpop binds to.
#[derive(Debug, Clone)]
pub struct SshCertificate {
    /// Ed25519 host public key (32 bytes) — the cert's `Key`.
    pub host_public_key: [u8; 32],
    /// Certificate `ValidPrincipals` (hostnames).
    pub principals: Vec<String>,
    /// CA signature over `cert_signing_bytes(host_public_key, principals)`.
    pub ca_signature: Vec<u8>,
}

/// Canonical bytes the SSH CA signs — host key followed by each principal,
/// length-delimited so principals can't be ambiguously concatenated.
pub fn cert_signing_bytes(host_public_key: &[u8; 32], principals: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + principals.iter().map(|p| p.len() + 4).sum::<usize>());
    out.extend_from_slice(host_public_key);
    for p in principals {
        out.extend_from_slice(&(p.len() as u32).to_be_bytes());
        out.extend_from_slice(p.as_bytes());
    }
    out
}

/// SHA-256 hex fingerprint of an SSH host public key — the agent-id suffix.
pub fn fingerprint_hex(host_public_key: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(host_public_key);
    hex::encode(h.finalize())
}

/// Server-side `sshpop` node attestor.
pub struct SshPopAttestor {
    trust_domain: TrustDomain,
    /// Ed25519 public keys of the SSH CAs whose host certs we accept.
    trusted_cas: Vec<[u8; 32]>,
}

impl SshPopAttestor {
    pub fn new(trust_domain: impl Into<String>, trusted_cas: Vec<[u8; 32]>) -> Self {
        Self {
            trust_domain: TrustDomain::new(trust_domain),
            trusted_cas,
        }
    }

    /// Verify the certificate's CA signature against a trusted SSH CA.
    pub fn verify_cert(&self, cert: &SshCertificate) -> Result<()> {
        let signing_bytes = cert_signing_bytes(&cert.host_public_key, &cert.principals);
        let sig = parse_signature(&cert.ca_signature)?;
        for ca in &self.trusted_cas {
            let Ok(vk) = VerifyingKey::from_bytes(ca) else {
                continue;
            };
            if vk.verify_strict(&signing_bytes, &sig).is_ok() {
                return Ok(());
            }
        }
        Err(IdentityError::AttestationFailed(
            "sshpop: certificate not signed by a trusted CA".into(),
        ))
    }

    /// Issue a challenge nonce bound to server entropy. Deterministic in the
    /// entropy so the server can recompute + match the value it stored.
    pub fn new_challenge(&self, server_entropy: &[u8]) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update(b"sshpop-challenge:");
        h.update(server_entropy);
        h.finalize().to_vec()
    }

    /// Complete attestation: verify the cert chains to a trusted CA, verify the
    /// agent's proof-of-possession signature over the nonce, then derive the
    /// agent SPIFFE ID + `sshpop:` selectors.
    pub fn attest_challenge(
        &self,
        cert: &SshCertificate,
        nonce: &[u8],
        pop_response: &[u8],
    ) -> Result<(SpiffeId, Vec<Selector>)> {
        self.verify_cert(cert)?;
        let host_vk = VerifyingKey::from_bytes(&cert.host_public_key)
            .map_err(|e| IdentityError::AttestationFailed(format!("sshpop: bad host key: {e}")))?;
        let sig = parse_signature(pop_response)?;
        host_vk.verify_strict(nonce, &sig).map_err(|_| {
            IdentityError::AttestationFailed("sshpop: proof-of-possession failed".into())
        })?;

        let fp = fingerprint_hex(&cert.host_public_key);
        let id = agent_id(&self.trust_domain, "sshpop", &fp)?;
        let mut selectors = vec![Selector::new("sshpop", format!("fingerprint:{fp}"))];
        for p in &cert.principals {
            selectors.push(Selector::new("sshpop", format!("hostname:{p}")));
        }
        Ok((id, selectors))
    }
}

fn parse_signature(bytes: &[u8]) -> Result<Signature> {
    let arr: [u8; 64] = bytes
        .try_into()
        .map_err(|_| IdentityError::AttestationFailed("sshpop: signature not 64 bytes".into()))?;
    Ok(Signature::from_bytes(&arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn ca_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }
    fn host_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// Build a CA-signed cert the way an agent would present it.
    fn signed_cert(ca: &SigningKey, host: &SigningKey, principals: &[&str]) -> SshCertificate {
        let host_pub = host.verifying_key().to_bytes();
        let principals: Vec<String> = principals.iter().map(|s| s.to_string()).collect();
        let to_sign = cert_signing_bytes(&host_pub, &principals);
        let sig = ca.sign(&to_sign).to_bytes().to_vec();
        SshCertificate {
            host_public_key: host_pub,
            principals,
            ca_signature: sig,
        }
    }

    #[test]
    fn verify_cert_accepts_trusted_ca() {
        let ca = ca_key(1);
        let host = host_key(9);
        let att = SshPopAttestor::new("example.org", vec![ca.verifying_key().to_bytes()]);
        let cert = signed_cert(&ca, &host, &["web01.prod"]);
        assert!(att.verify_cert(&cert).is_ok());
    }

    #[test]
    fn verify_cert_rejects_untrusted_ca() {
        let ca = ca_key(1);
        let rogue = ca_key(2);
        let host = host_key(9);
        // attestor only trusts `rogue`, but cert is signed by `ca`
        let att = SshPopAttestor::new("example.org", vec![rogue.verifying_key().to_bytes()]);
        let cert = signed_cert(&ca, &host, &["web01.prod"]);
        assert!(att.verify_cert(&cert).is_err());
    }

    #[test]
    fn verify_cert_rejects_tampered_principals() {
        let ca = ca_key(1);
        let host = host_key(9);
        let att = SshPopAttestor::new("example.org", vec![ca.verifying_key().to_bytes()]);
        let mut cert = signed_cert(&ca, &host, &["web01.prod"]);
        cert.principals.push("admin".into()); // not covered by ca_signature
        assert!(att.verify_cert(&cert).is_err());
    }

    #[test]
    fn attest_challenge_returns_agent_id_and_selectors() {
        let ca = ca_key(1);
        let host = host_key(9);
        let att = SshPopAttestor::new("example.org", vec![ca.verifying_key().to_bytes()]);
        let cert = signed_cert(&ca, &host, &["web01.prod", "web01"]);
        let nonce = att.new_challenge(b"server-entropy");
        // agent signs the nonce with its host private key (proof of possession)
        let response = host.sign(&nonce).to_bytes().to_vec();
        let (id, selectors) = att.attest_challenge(&cert, &nonce, &response).unwrap();
        // agent id: spiffe://<td>/spire/agent/sshpop/<fingerprint-hex>
        let fp = fingerprint_hex(&cert.host_public_key);
        assert_eq!(
            id.as_str(),
            format!("spiffe://example.org/spire/agent/sshpop/{}", fp)
        );
        assert!(selectors
            .iter()
            .any(|s| s.canonical() == "sshpop:hostname:web01.prod"));
        assert!(selectors
            .iter()
            .any(|s| s.canonical() == format!("sshpop:fingerprint:{}", fp)));
    }

    #[test]
    fn attest_challenge_rejects_wrong_pop_signature() {
        let ca = ca_key(1);
        let host = host_key(9);
        let imposter = host_key(8);
        let att = SshPopAttestor::new("example.org", vec![ca.verifying_key().to_bytes()]);
        let cert = signed_cert(&ca, &host, &["web01.prod"]);
        let nonce = att.new_challenge(b"server-entropy");
        // imposter signs — does not possess the cert's host key
        let response = imposter.sign(&nonce).to_bytes().to_vec();
        assert!(att.attest_challenge(&cert, &nonce, &response).is_err());
    }

    #[test]
    fn attest_challenge_rejects_untrusted_cert() {
        let ca = ca_key(1);
        let rogue = ca_key(2);
        let host = host_key(9);
        let att = SshPopAttestor::new("example.org", vec![ca.verifying_key().to_bytes()]);
        let cert = signed_cert(&rogue, &host, &["web01.prod"]);
        let nonce = att.new_challenge(b"x");
        let response = host.sign(&nonce).to_bytes().to_vec();
        assert!(att.attest_challenge(&cert, &nonce, &response).is_err());
    }

    #[test]
    fn challenge_is_bound_to_entropy() {
        let att = SshPopAttestor::new("example.org", vec![]);
        let a = att.new_challenge(b"one");
        let b = att.new_challenge(b"two");
        assert_ne!(a, b);
        // same entropy → same deterministic nonce (server stores it to match)
        assert_eq!(att.new_challenge(b"one"), a);
    }

    #[test]
    fn fingerprint_is_sha256_hex() {
        let host = host_key(9);
        let fp = fingerprint_hex(&host.verifying_key().to_bytes());
        assert_eq!(fp.len(), 64); // 32-byte SHA-256 hex-encoded
    }
}
