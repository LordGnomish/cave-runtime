// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OCI artifact signature attachment.
//!
//! Maps to:
//!   * pkg/cosign/sign           → SignOci
//!   * pkg/cosign/verify.go      → VerifyImageSignatures
//!   * pkg/oci/static            → static (in-memory) signature layers
//!   * cmd/cosign/cli/sign       → `cosign sign <image>`
//!
//! Real OCI push is the registry adapter's job (cave-artifacts wires
//! Pulp/Harbor/Nexus). This module owns the **envelope**: how a signature
//! is packaged as an OCI image manifest pointing at the artifact digest,
//! plus the discovery URI (`<digest>.sig` tag-by-digest).

use crate::bundle::CosignBundle;
use crate::error::{Result, SignError};
use crate::models::{SigKind, Signature};
use crate::rekor::{HashedRekordEntry, RekorClient};
use crate::signature::Keypair;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Subject reference — image manifest digest the signature covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    pub digest: String,
}

impl ImageRef {
    /// Parse `registry/repo@sha256:<hex>`.
    pub fn parse(input: &str) -> Result<Self> {
        let (host_repo, digest) = input
            .split_once('@')
            .ok_or_else(|| SignError::InvalidDigest("expected ref@digest".into()))?;
        if !digest.starts_with("sha256:") || digest.len() != "sha256:".len() + 64 {
            return Err(SignError::InvalidDigest(format!(
                "not a sha256 digest: {}",
                digest
            )));
        }
        let (registry, repository) = host_repo
            .split_once('/')
            .ok_or_else(|| SignError::InvalidDigest("expected registry/repo".into()))?;
        Ok(Self {
            registry: registry.into(),
            repository: repository.into(),
            digest: digest.into(),
        })
    }

    /// Cosign discovery tag — `<algorithm>-<hex>.sig`.
    pub fn signature_tag(&self) -> String {
        let hex = self.digest.trim_start_matches("sha256:");
        format!("sha256-{}.sig", hex)
    }

    pub fn attestation_tag(&self) -> String {
        let hex = self.digest.trim_start_matches("sha256:");
        format!("sha256-{}.att", hex)
    }

    /// Cosign SBOM discovery tag — `<algorithm>-<hex>.sbom`.
    pub fn sbom_tag(&self) -> String {
        let hex = self.digest.trim_start_matches("sha256:");
        format!("sha256-{}.sbom", hex)
    }

    pub fn signature_uri(&self) -> String {
        self.tag_uri(&self.signature_tag())
    }

    pub fn attestation_uri(&self) -> String {
        self.tag_uri(&self.attestation_tag())
    }

    pub fn sbom_uri(&self) -> String {
        self.tag_uri(&self.sbom_tag())
    }

    fn tag_uri(&self, tag: &str) -> String {
        format!("{}/{}:{}", self.registry, self.repository, tag)
    }

    /// `cosign triangulate --type <signature|attestation|sbom>` — derive the
    /// OCI reference where the associated artifact is stored, by digest-tag
    /// convention. Maps cmd/cosign/cli/triangulate.go + ociremote.*Tag.
    pub fn triangulate(&self, kind: CosignArtifactType) -> String {
        match kind {
            CosignArtifactType::Signature => self.signature_uri(),
            CosignArtifactType::Attestation => self.attestation_uri(),
            CosignArtifactType::Sbom => self.sbom_uri(),
        }
    }
}

/// The kinds of cosign-attached artifacts a `triangulate` can resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CosignArtifactType {
    Signature,
    Attestation,
    Sbom,
}

impl CosignArtifactType {
    /// Parse the `--type` flag value (cosign accepts `signature`, the default,
    /// plus `attestation` and `sbom`).
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "signature" | "sig" => Ok(Self::Signature),
            "attestation" | "att" => Ok(Self::Attestation),
            "sbom" => Ok(Self::Sbom),
            other => Err(SignError::InvalidDigest(format!(
                "unknown triangulate type: {} (want signature|attestation|sbom)",
                other
            ))),
        }
    }
}

/// OCI image manifest envelope a registry stores under `signature_tag`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignatureLayer {
    pub media_type: String,
    pub size: u64,
    pub digest: String,
    pub annotations: SignatureAnnotations,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignatureAnnotations {
    #[serde(rename = "dev.cosignproject.cosign/signature")]
    pub signature_b64: String,
    #[serde(rename = "dev.sigstore.cosign/certificate", skip_serializing_if = "Option::is_none")]
    pub certificate_pem: Option<String>,
    #[serde(rename = "dev.sigstore.cosign/chain", skip_serializing_if = "Option::is_none")]
    pub chain_pem: Option<String>,
    #[serde(rename = "dev.sigstore.cosign/bundle", skip_serializing_if = "Option::is_none")]
    pub bundle_json: Option<String>,
}

pub fn build_layer(bundle: &CosignBundle, payload_size: u64) -> Result<SignatureLayer> {
    Ok(SignatureLayer {
        media_type: "application/vnd.dev.cosign.simplesigning.v1+json".into(),
        size: payload_size,
        digest: bundle.artifact_digest.clone(),
        annotations: SignatureAnnotations {
            signature_b64: bundle.signed_payload_b64.clone(),
            certificate_pem: Some(bundle.cert_pem.clone()),
            chain_pem: bundle.chain_pem.clone(),
            bundle_json: Some(bundle.encode_json()?),
        },
    })
}

/// Sign an OCI image by its manifest digest. Returns the signature layer
/// the caller can `PUT` to the registry under `signature_tag`.
pub fn sign_image_keypair(
    image: &ImageRef,
    manifest_size: u64,
    keypair: &Keypair,
) -> Result<(Signature, SignatureLayer, CosignBundle)> {
    let payload = image_payload(image);
    let raw = keypair.sign(&payload)?;
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let cert_pem = crate::keypair::encode_public_pem(keypair.algorithm, keypair.public_key_bytes());
    let bundle = CosignBundle {
        kind: SigKind::Keypair,
        signed_payload_b64: sig_b64.clone(),
        cert_pem: cert_pem.clone(),
        chain_pem: None,
        rekor_log_index: None,
        rekor_uuid: None,
        rekor_integrated_time: None,
        artifact_digest: image.digest.clone(),
    };
    let layer = build_layer(&bundle, manifest_size)?;
    let signature = Signature {
        kind: SigKind::Keypair,
        sig_b64,
        cert_pem,
        chain_pem: None,
        log_index: None,
    };
    Ok((signature, layer, bundle))
}

pub fn sign_image_keypair_with_rekor(
    image: &ImageRef,
    manifest_size: u64,
    keypair: &Keypair,
    rekor: &RekorClient,
) -> Result<(Signature, SignatureLayer, CosignBundle)> {
    let (mut sig, mut layer, mut bundle) = sign_image_keypair(image, manifest_size, keypair)?;
    let digest_hex = image
        .digest
        .strip_prefix("sha256:")
        .unwrap_or(&image.digest)
        .to_string();
    let entry = HashedRekordEntry {
        digest_hex,
        signature_b64: sig.sig_b64.clone(),
        public_key_pem: sig.cert_pem.clone(),
    };
    let log = rekor.upload_offline(entry)?;
    sig.log_index = Some(log.log_index);
    bundle.rekor_log_index = Some(log.log_index);
    bundle.rekor_uuid = Some(log.uuid.clone());
    bundle.rekor_integrated_time = Some(log.integrated_time);
    layer.annotations.bundle_json = Some(bundle.encode_json()?);
    Ok((sig, layer, bundle))
}

pub fn verify_image(image: &ImageRef, bundle: &CosignBundle) -> Result<()> {
    if image.digest != bundle.artifact_digest {
        return Err(SignError::Verify(format!(
            "image digest {} != bundle digest {}",
            image.digest, bundle.artifact_digest
        )));
    }
    let payload = image_payload(image);
    let sig = base64::engine::general_purpose::STANDARD
        .decode(bundle.signed_payload_b64.as_bytes())
        .map_err(|e| SignError::InvalidSignature(format!("sig base64: {}", e)))?;
    let (alg, pk_bytes) = crate::keypair::decode_public_pem(&bundle.cert_pem)?;
    crate::signature::verify(alg, &pk_bytes, &payload, &sig)
}

/// Canonical payload cosign signs for an image: the digest + repository
/// path. Matches the simplesigning v1 payload shape.
fn image_payload(image: &ImageRef) -> Vec<u8> {
    let payload = serde_json::json!({
        "critical": {
            "identity": {"docker-reference": format!("{}/{}", image.registry, image.repository)},
            "image": {"docker-manifest-digest": image.digest},
            "type": "cosign container image signature",
        },
        "optional": {}
    });
    serde_json::to_vec(&payload).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::KeyAlgorithm;

    fn img() -> ImageRef {
        ImageRef::parse(
            "ghcr.io/cave/runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap()
    }

    #[test]
    fn parse_image_ref() {
        let r = img();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "cave/runtime");
        assert!(r.digest.starts_with("sha256:"));
    }

    #[test]
    fn signature_tag_format() {
        let r = img();
        assert_eq!(
            r.signature_tag(),
            "sha256-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.sig"
        );
    }

    #[test]
    fn attestation_tag_format() {
        let r = img();
        assert!(r.attestation_tag().ends_with(".att"));
    }

    #[test]
    fn invalid_digest_rejected() {
        let err =
            ImageRef::parse("ghcr.io/cave/x@sha256:short").expect_err("must reject short digest");
        assert!(matches!(err, SignError::InvalidDigest(_)));
    }

    #[test]
    fn sign_then_verify_image() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[1u8; 32]).unwrap();
        let (_sig, layer, bundle) = sign_image_keypair(&img(), 4096, &kp).unwrap();
        assert!(layer.media_type.contains("simplesigning"));
        verify_image(&img(), &bundle).unwrap();
    }

    #[test]
    fn verify_rejects_different_digest() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[2u8; 32]).unwrap();
        let (_s, _l, bundle) = sign_image_keypair(&img(), 1024, &kp).unwrap();
        let mut other = img();
        other.digest =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        assert!(verify_image(&other, &bundle).is_err());
    }

    #[test]
    fn sign_with_rekor_includes_log_index() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[3u8; 32]).unwrap();
        let rk = RekorClient::default();
        let (sig, _l, bundle) = sign_image_keypair_with_rekor(&img(), 2048, &kp, &rk).unwrap();
        assert!(sig.log_index.is_some());
        assert!(bundle.has_rekor_entry());
    }

    #[test]
    fn layer_annotations_round_trip() {
        let kp = Keypair::from_seed(KeyAlgorithm::Ed25519, &[4u8; 32]).unwrap();
        let (_s, layer, _b) = sign_image_keypair(&img(), 100, &kp).unwrap();
        let j = serde_json::to_string(&layer).unwrap();
        assert!(j.contains("dev.cosignproject.cosign/signature"));
        let back: SignatureLayer = serde_json::from_str(&j).unwrap();
        assert_eq!(back, layer);
    }

    #[test]
    fn signature_uri_format() {
        let r = img();
        assert!(r.signature_uri().starts_with("ghcr.io/cave/runtime:sha256-"));
    }

    #[test]
    fn sbom_tag_and_uri() {
        let r = img();
        assert!(r.sbom_tag().ends_with(".sbom"));
        assert!(r.sbom_tag().starts_with("sha256-"));
        assert_eq!(r.sbom_uri(), format!("ghcr.io/cave/runtime:{}", r.sbom_tag()));
    }

    #[test]
    fn attestation_uri_format() {
        let r = img();
        assert_eq!(
            r.attestation_uri(),
            format!("ghcr.io/cave/runtime:{}", r.attestation_tag())
        );
    }

    #[test]
    fn triangulate_dispatches_by_type() {
        let r = img();
        assert_eq!(r.triangulate(CosignArtifactType::Signature), r.signature_uri());
        assert_eq!(r.triangulate(CosignArtifactType::Attestation), r.attestation_uri());
        assert_eq!(r.triangulate(CosignArtifactType::Sbom), r.sbom_uri());
    }

    #[test]
    fn artifact_type_parses_cosign_flag() {
        assert_eq!(
            CosignArtifactType::parse("signature").unwrap(),
            CosignArtifactType::Signature
        );
        assert_eq!(
            CosignArtifactType::parse("attestation").unwrap(),
            CosignArtifactType::Attestation
        );
        assert_eq!(CosignArtifactType::parse("sbom").unwrap(), CosignArtifactType::Sbom);
        assert!(CosignArtifactType::parse("bogus").is_err());
    }
}
