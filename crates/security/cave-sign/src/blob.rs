// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Blob signing — files + arbitrary byte payloads.
//!
//! Maps to:
//!   * cmd/cosign/cli/signblob.go    → `cosign sign-blob`
//!   * cmd/cosign/cli/verify-blob.go → `cosign verify-blob`
//!
//! Differs from `oci.rs` in two ways: (1) the artifact is not pushed to
//! a registry; (2) the digest covers the file's bytes (not an OCI manifest).

use crate::bundle::CosignBundle;
use crate::error::Result;
use crate::models::{Signature, SigKind};
use crate::rekor::{HashedRekordEntry, RekorClient};
use crate::signature::{Keypair, sha256_digest_string};
use base64::Engine;

/// Result of signing a blob: enough material to write the cosign triple
/// (`.sig`, `.crt`, `.bundle`) to disk.
#[derive(Debug, Clone)]
pub struct BlobSignature {
    pub artifact_digest: String,
    pub signature: Signature,
    pub bundle: CosignBundle,
}

/// Sign a blob with a long-lived keypair (no Fulcio/Rekor).
pub fn sign_blob_keypair(payload: &[u8], keypair: &Keypair) -> Result<BlobSignature> {
    let digest = sha256_digest_string(payload);
    let raw = keypair.sign(payload)?;
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let cert_pem = crate::keypair::encode_public_pem(keypair.algorithm, keypair.public_key_bytes());
    let signature = Signature {
        kind: SigKind::Keypair,
        sig_b64: sig_b64.clone(),
        cert_pem: cert_pem.clone(),
        chain_pem: None,
        log_index: None,
    };
    let bundle = CosignBundle {
        kind: SigKind::Keypair,
        signed_payload_b64: sig_b64,
        cert_pem,
        chain_pem: None,
        rekor_log_index: None,
        rekor_uuid: None,
        rekor_integrated_time: None,
        artifact_digest: digest.clone(),
    };
    Ok(BlobSignature {
        artifact_digest: digest,
        signature,
        bundle,
    })
}

/// Sign a blob and upload the entry to a Rekor instance (offline log here).
pub fn sign_blob_keypair_with_rekor(
    payload: &[u8],
    keypair: &Keypair,
    rekor: &RekorClient,
) -> Result<BlobSignature> {
    let mut b = sign_blob_keypair(payload, keypair)?;
    let digest_hex = b
        .artifact_digest
        .strip_prefix("sha256:")
        .unwrap_or(&b.artifact_digest)
        .to_string();
    let entry = HashedRekordEntry {
        digest_hex,
        signature_b64: b.signature.sig_b64.clone(),
        public_key_pem: b.signature.cert_pem.clone(),
    };
    let log = rekor.upload_offline(entry)?;
    b.signature.log_index = Some(log.log_index);
    b.bundle.rekor_log_index = Some(log.log_index);
    b.bundle.rekor_uuid = Some(log.uuid);
    b.bundle.rekor_integrated_time = Some(log.integrated_time);
    Ok(b)
}

/// Verify a blob+bundle. The caller supplies the bundle; we re-derive the
/// digest from the bytes and check it matches.
pub fn verify_blob(
    payload: &[u8],
    bundle: &CosignBundle,
) -> Result<()> {
    use crate::error::SignError;
    let expected = sha256_digest_string(payload);
    if expected != bundle.artifact_digest {
        return Err(SignError::Verify(format!(
            "digest mismatch: bundle={}, actual={}",
            bundle.artifact_digest, expected
        )));
    }
    let sig = base64::engine::general_purpose::STANDARD
        .decode(bundle.signed_payload_b64.as_bytes())
        .map_err(|e| SignError::InvalidSignature(format!("sig base64: {}", e)))?;
    let (alg, pk_bytes) = crate::keypair::decode_public_pem(&bundle.cert_pem)?;
    crate::signature::verify(alg, &pk_bytes, payload, &sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::KeyAlgorithm;

    #[test]
    fn sign_then_verify_roundtrip() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
        let payload = b"hello cave-sign";
        let b = sign_blob_keypair(payload, &kp).unwrap();
        assert!(b.artifact_digest.starts_with("sha256:"));
        verify_blob(payload, &b.bundle).unwrap();
    }

    #[test]
    fn verify_fails_on_tampered_payload() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[2u8; 32]).unwrap();
        let b = sign_blob_keypair(b"hello", &kp).unwrap();
        let err = verify_blob(b"helLO", &b.bundle).expect_err("digest mismatch must fire");
        assert!(format!("{}", err).contains("digest mismatch"));
    }

    #[test]
    fn rekor_upload_records_log_index() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[3u8; 32]).unwrap();
        let rk = RekorClient::default();
        let b = sign_blob_keypair_with_rekor(b"payload-1", &kp, &rk).unwrap();
        assert!(b.signature.log_index.is_some());
        assert!(b.bundle.has_rekor_entry());
        let again = sign_blob_keypair_with_rekor(b"payload-2", &kp, &rk).unwrap();
        assert_ne!(b.signature.log_index, again.signature.log_index);
    }

    #[test]
    fn ed25519_blob_roundtrip() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[4u8; 32]).unwrap();
        let b = sign_blob_keypair(b"a", &kp).unwrap();
        verify_blob(b"a", &b.bundle).unwrap();
    }

    #[test]
    fn bundle_carries_artifact_digest() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[5u8; 32]).unwrap();
        let b = sign_blob_keypair(b"abc", &kp).unwrap();
        assert_eq!(
            b.bundle.artifact_digest,
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
