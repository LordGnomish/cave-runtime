// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kubernetes version management — catalog, validation, upgrade paths.

use crate::error::{ClusterError, ClusterResult};
use serde::{Deserialize, Serialize};

/// A supported Kubernetes version entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sVersion {
    pub version: String,
    pub release_date: String,
    pub end_of_life: String,
    pub is_supported: bool,
    pub is_latest: bool,
    pub notes: String,
}

/// Full catalog of supported Kubernetes versions.
pub fn supported_versions() -> Vec<K8sVersion> {
    vec![
        K8sVersion {
            version: "1.27".into(),
            release_date: "2023-04-11".into(),
            end_of_life: "2024-06-28".into(),
            is_supported: false,
            is_latest: false,
            notes: "EOL".into(),
        },
        K8sVersion {
            version: "1.28".into(),
            release_date: "2023-08-15".into(),
            end_of_life: "2024-10-28".into(),
            is_supported: true,
            is_latest: false,
            notes: "Supported".into(),
        },
        K8sVersion {
            version: "1.29".into(),
            release_date: "2023-12-13".into(),
            end_of_life: "2025-02-28".into(),
            is_supported: true,
            is_latest: false,
            notes: "Supported".into(),
        },
        K8sVersion {
            version: "1.30".into(),
            release_date: "2024-04-17".into(),
            end_of_life: "2025-06-28".into(),
            is_supported: true,
            is_latest: false,
            notes: "Supported".into(),
        },
        K8sVersion {
            version: "1.31".into(),
            release_date: "2024-08-13".into(),
            end_of_life: "2025-10-28".into(),
            is_supported: true,
            is_latest: false,
            notes: "Supported".into(),
        },
        K8sVersion {
            version: "1.32".into(),
            release_date: "2024-12-11".into(),
            end_of_life: "2026-02-28".into(),
            is_supported: true,
            is_latest: true,
            notes: "Latest stable".into(),
        },
    ]
}

/// Parse a version string into (major, minor).
fn parse_version(v: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = v.split('.').collect();
    match parts.as_slice() {
        [major, minor, ..] => Some((major.parse().ok()?, minor.parse().ok()?)),
        [major] => Some((major.parse().ok()?, 0)),
        _ => None,
    }
}

/// Validate that a version string is in our supported catalog.
pub fn validate_k8s_version(version: &str) -> ClusterResult<()> {
    let versions = supported_versions();
    let found = versions.iter().find(|v| v.version == version);
    match found {
        Some(v) if v.is_supported => Ok(()),
        Some(_) => Err(ClusterError::UnsupportedVersion(format!(
            "{version} is end-of-life"
        ))),
        None => Err(ClusterError::UnsupportedVersion(version.to_string())),
    }
}

/// Validate an upgrade path: only one minor version at a time.
pub fn validate_upgrade(from: &str, to: &str) -> ClusterResult<()> {
    let (from_major, from_minor) = parse_version(from).ok_or_else(|| {
        ClusterError::InvalidUpgrade {
            from: from.to_string(),
            to: to.to_string(),
            reason: "invalid source version".into(),
        }
    })?;
    let (to_major, to_minor) = parse_version(to).ok_or_else(|| {
        ClusterError::InvalidUpgrade {
            from: from.to_string(),
            to: to.to_string(),
            reason: "invalid target version".into(),
        }
    })?;

    if to_major < from_major || (to_major == from_major && to_minor < from_minor) {
        return Err(ClusterError::InvalidUpgrade {
            from: from.to_string(),
            to: to.to_string(),
            reason: "cannot downgrade".into(),
        });
    }

    if to_major == from_major && to_minor == from_minor {
        return Err(ClusterError::InvalidUpgrade {
            from: from.to_string(),
            to: to.to_string(),
            reason: "already at target version".into(),
        });
    }

    if to_major == from_major && to_minor > from_minor + 1 {
        return Err(ClusterError::InvalidUpgrade {
            from: from.to_string(),
            to: to.to_string(),
            reason: "can only upgrade one minor version at a time".into(),
        });
    }

    validate_k8s_version(to)?;
    Ok(())
}

/// Get the latest supported version.
pub fn latest_version() -> &'static str {
    "1.32"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_version_ok() {
        assert!(validate_k8s_version("1.29").is_ok());
        assert!(validate_k8s_version("1.30").is_ok());
        assert!(validate_k8s_version("1.32").is_ok());
    }

    #[test]
    fn unsupported_version_fails() {
        assert!(validate_k8s_version("1.25").is_err());
        assert!(validate_k8s_version("1.27").is_err()); // EOL
        assert!(validate_k8s_version("2.0").is_err());
    }

    #[test]
    fn valid_upgrade_path() {
        assert!(validate_upgrade("1.29", "1.30").is_ok());
        assert!(validate_upgrade("1.31", "1.32").is_ok());
    }

    #[test]
    fn skip_minor_version_fails() {
        assert!(validate_upgrade("1.29", "1.31").is_err());
    }

    #[test]
    fn downgrade_fails() {
        assert!(validate_upgrade("1.30", "1.29").is_err());
    }

    #[test]
    fn same_version_fails() {
        assert!(validate_upgrade("1.30", "1.30").is_err());
    }
}
