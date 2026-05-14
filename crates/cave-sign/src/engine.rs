// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{ArtifactType, SignedArtifact, VerifyResult};

pub fn is_valid_digest(digest: &str) -> bool {
    if let Some(hex) = digest.strip_prefix("sha256:") {
        hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        false
    }
}

pub fn find_signatures<'a>(
    artifacts: &'a [SignedArtifact],
    digest: &str,
) -> Vec<&'a SignedArtifact> {
    artifacts
        .iter()
        .filter(|a| a.artifact_digest == digest)
        .collect()
}

pub fn is_signed_by(artifacts: &[SignedArtifact], digest: &str, signer: &str) -> bool {
    artifacts.iter().any(|a| {
        a.artifact_digest == digest && a.signer_identity == signer && a.verified
    })
}

pub fn filter_by_type<'a>(
    artifacts: &'a [SignedArtifact],
    artifact_type: &ArtifactType,
) -> Vec<&'a SignedArtifact> {
    artifacts
        .iter()
        .filter(|a| &a.artifact_type == artifact_type)
        .collect()
}

pub fn count_verified(artifacts: &[SignedArtifact]) -> usize {
    artifacts.iter().filter(|a| a.verified).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_artifact(digest: &str, signer: &str, verified: bool, atype: ArtifactType) -> SignedArtifact {
        SignedArtifact {
            id: Uuid::new_v4(),
            artifact_digest: digest.to_string(),
            artifact_type: atype,
            signature: "c2lnbmF0dXJl".to_string(),
            signer_identity: signer.to_string(),
            signed_at: Utc::now(),
            verified,
        }
    }

    const VALID_DIGEST: &str =
        "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

    #[test]
    fn test_valid_digest_sha256() {
        assert!(is_valid_digest(VALID_DIGEST));
    }

    #[test]
    fn test_invalid_digest_no_prefix() {
        assert!(!is_valid_digest(
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        ));
    }

    #[test]
    fn test_invalid_digest_wrong_length() {
        assert!(!is_valid_digest("sha256:abc123"));
    }

    #[test]
    fn test_find_signatures_by_digest() {
        let artifacts = vec![
            make_artifact(VALID_DIGEST, "alice@example.com", true, ArtifactType::ContainerImage),
            make_artifact("sha256:0000000000000000000000000000000000000000000000000000000000000000", "bob@example.com", true, ArtifactType::Binary),
        ];
        let found = find_signatures(&artifacts, VALID_DIGEST);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].signer_identity, "alice@example.com");
    }

    #[test]
    fn test_is_signed_by_verified() {
        let artifacts = vec![
            make_artifact(VALID_DIGEST, "alice@example.com", true, ArtifactType::ContainerImage),
        ];
        assert!(is_signed_by(&artifacts, VALID_DIGEST, "alice@example.com"));
        assert!(!is_signed_by(&artifacts, VALID_DIGEST, "bob@example.com"));
    }

    #[test]
    fn test_count_verified() {
        let artifacts = vec![
            make_artifact(VALID_DIGEST, "alice@example.com", true, ArtifactType::ContainerImage),
            make_artifact(VALID_DIGEST, "bob@example.com", false, ArtifactType::Binary),
            make_artifact(VALID_DIGEST, "carol@example.com", true, ArtifactType::Chart),
        ];
        assert_eq!(count_verified(&artifacts), 2);
    }
}
