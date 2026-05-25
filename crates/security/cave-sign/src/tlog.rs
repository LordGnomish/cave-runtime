// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Transparency log binding — the cross-check between a Sigstore bundle
//! and the Rekor entry it claims.
//!
//! Maps to:
//!   * pkg/cosign/tlog.go        → VerifyTLogEntry
//!   * pkg/cosign/verify.go      → CheckSignatureContents (rekor branch)
//!
//! `verify_against_rekor` is the only function the verifier calls — it
//! looks up the entry by log_index, re-decodes the embedded
//! HashedRekord, and confirms (digest, signature, public key) match.

use crate::bundle::CosignBundle;
use crate::error::{Result, SignError};
use crate::rekor::{decode_entry_body, RekorClient};

/// Confirm that a bundle is consistent with the Rekor entry it points at.
pub fn verify_against_rekor(bundle: &CosignBundle, rekor: &RekorClient) -> Result<()> {
    let log_index = bundle
        .rekor_log_index
        .ok_or_else(|| SignError::Tlog("bundle missing rekor_log_index".into()))?;
    let entry = rekor.get_by_index_offline(log_index)?;
    let body = decode_entry_body(&entry)?;
    let expected_hex = bundle
        .artifact_digest
        .strip_prefix("sha256:")
        .unwrap_or(&bundle.artifact_digest);
    if body.digest_hex != expected_hex {
        return Err(SignError::Tlog(format!(
            "rekor entry covers {}, bundle says {}",
            body.digest_hex, expected_hex
        )));
    }
    if body.signature_b64 != bundle.signed_payload_b64 {
        return Err(SignError::Tlog("rekor signature bytes diverge from bundle".into()));
    }
    if body.public_key_pem != bundle.cert_pem {
        return Err(SignError::Tlog("rekor public key/cert diverges from bundle".into()));
    }
    if let Some(uuid) = &bundle.rekor_uuid {
        if uuid != &entry.uuid {
            return Err(SignError::Tlog("rekor uuid diverges from bundle".into()));
        }
    }
    if let Some(t) = bundle.rekor_integrated_time {
        if t != entry.integrated_time {
            return Err(SignError::Tlog("rekor integrated_time diverges".into()));
        }
    }
    Ok(())
}

/// Build a witness — `(tree_size, root_hash, inclusion_proof)` — that a
/// caller can pin into an audit trail.
pub fn build_witness(
    bundle: &CosignBundle,
    rekor: &RekorClient,
) -> Result<TlogWitness> {
    let log_index = bundle
        .rekor_log_index
        .ok_or_else(|| SignError::Tlog("bundle missing rekor_log_index".into()))?;
    let (tree_size, root_hash) = rekor.tree_state_offline()?;
    let proof = rekor.inclusion_proof_offline(log_index)?;
    Ok(TlogWitness {
        log_index,
        tree_size,
        root_hash,
        sibling_hashes: proof.hashes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlogWitness {
    pub log_index: u64,
    pub tree_size: u64,
    pub root_hash: String,
    pub sibling_hashes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::sign_blob_keypair_with_rekor;
    use crate::models::KeyAlgorithm;
    use crate::signature::Keypair;

    #[test]
    fn verify_against_rekor_happy_path() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
        let rk = RekorClient::default();
        let b = sign_blob_keypair_with_rekor(b"payload", &kp, &rk).unwrap();
        verify_against_rekor(&b.bundle, &rk).unwrap();
    }

    #[test]
    fn missing_log_index_rejected() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[2u8; 32]).unwrap();
        let rk = RekorClient::default();
        let mut b = sign_blob_keypair_with_rekor(b"x", &kp, &rk).unwrap();
        b.bundle.rekor_log_index = None;
        assert!(verify_against_rekor(&b.bundle, &rk).is_err());
    }

    #[test]
    fn tampered_digest_rejected() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[3u8; 32]).unwrap();
        let rk = RekorClient::default();
        let mut b = sign_blob_keypair_with_rekor(b"y", &kp, &rk).unwrap();
        b.bundle.artifact_digest =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".into();
        assert!(verify_against_rekor(&b.bundle, &rk).is_err());
    }

    #[test]
    fn tampered_signature_rejected() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[4u8; 32]).unwrap();
        let rk = RekorClient::default();
        let mut b = sign_blob_keypair_with_rekor(b"z", &kp, &rk).unwrap();
        b.bundle.signed_payload_b64.push('X');
        assert!(verify_against_rekor(&b.bundle, &rk).is_err());
    }

    #[test]
    fn divergent_uuid_rejected() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[5u8; 32]).unwrap();
        let rk = RekorClient::default();
        let mut b = sign_blob_keypair_with_rekor(b"w", &kp, &rk).unwrap();
        b.bundle.rekor_uuid = Some("not-matching".into());
        assert!(verify_against_rekor(&b.bundle, &rk).is_err());
    }

    #[test]
    fn witness_carries_inclusion_proof() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[7u8; 32]).unwrap();
        let rk = RekorClient::default();
        let mut sigs = Vec::new();
        for i in 0..4 {
            sigs.push(sign_blob_keypair_with_rekor(&[i; 8], &kp, &rk).unwrap());
        }
        let w = build_witness(&sigs[1].bundle, &rk).unwrap();
        assert_eq!(w.tree_size, 4);
        assert_eq!(w.log_index, 1);
        // 4 leaves → 2-level proof.
        assert_eq!(w.sibling_hashes.len(), 2);
    }
}
