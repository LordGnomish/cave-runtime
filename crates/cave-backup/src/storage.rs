//! Backup storage location validation and lifecycle.

use crate::models::{BackupStorageLocation, BslPhase};

/// Validate a storage location, returning a list of error strings.
pub fn validate_bsl(bsl: &BackupStorageLocation) -> Vec<String> {
    let mut errors = Vec::new();
    if bsl.bucket.is_empty() {
        errors.push("bucket is required".into());
    }
    if bsl.name.is_empty() {
        errors.push("name is required".into());
    }
    errors
}

/// Mark a BSL as available after successful validation.
pub fn mark_available(bsl: &mut BackupStorageLocation) {
    bsl.phase = BslPhase::Available;
    bsl.last_validated_at = Some(chrono::Utc::now());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BslAccessMode, StorageProvider};
    use uuid::Uuid;

    fn make_bsl(name: &str, bucket: &str) -> BackupStorageLocation {
        BackupStorageLocation {
            id: Uuid::new_v4(),
            name: name.into(),
            provider: StorageProvider::S3,
            bucket: bucket.into(),
            prefix: None,
            region: None,
            endpoint: None,
            access_mode: BslAccessMode::ReadWrite,
            credential_secret: None,
            ca_bundle: None,
            insecure_skip_tls_verify: false,
            is_default: false,
            phase: BslPhase::Unavailable,
            last_validated_at: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn validate_bsl_valid() {
        let bsl = make_bsl("default", "cave-backups");
        assert!(validate_bsl(&bsl).is_empty());
    }

    #[test]
    fn validate_bsl_missing_bucket() {
        let bsl = make_bsl("default", "");
        let errors = validate_bsl(&bsl);
        assert!(errors.iter().any(|e| e.contains("bucket")));
    }

    #[test]
    fn validate_bsl_missing_name() {
        let bsl = make_bsl("", "my-bucket");
        let errors = validate_bsl(&bsl);
        assert!(errors.iter().any(|e| e.contains("name")));
    }

    #[test]
    fn validate_bsl_both_missing() {
        let bsl = make_bsl("", "");
        assert_eq!(validate_bsl(&bsl).len(), 2);
    }

    #[test]
    fn mark_available_sets_phase_and_timestamp() {
        let mut bsl = make_bsl("default", "bucket");
        assert_eq!(bsl.phase, BslPhase::Unavailable);
        assert!(bsl.last_validated_at.is_none());

        mark_available(&mut bsl);
        assert_eq!(bsl.phase, BslPhase::Available);
        assert!(bsl.last_validated_at.is_some());
    }
}
