// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). sshpop node-attestor proof-of-
// possession flow line-ported from pkg/common/plugin/sshpop + pkg/agent/plugin/
// nodeattestor/sshpop + pkg/server/plugin/nodeattestor/sshpop.

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
