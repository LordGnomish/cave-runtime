// SPDX-License-Identifier: AGPL-3.0-or-later
//! Multi-architecture image support — OCI image index / Docker manifest list.
//!
//! When a registry stores a single tag for multiple platforms it serves a
//! "manifest list" (Docker) or "image index" (OCI) instead of a leaf
//! manifest. Each entry pins one (architecture, os, [variant]) tuple to a
//! concrete leaf-manifest digest.
//!
//! Upstream:
//! - containerd: `images/converter/multi-arch.go`
//! - oci-spec:   <https://github.com/opencontainers/image-spec/blob/main/image-index.md>

use serde::{Deserialize, Serialize};

/// `Platform` mirrors `runtime-spec` and the OCI image index spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    /// CPU variant — e.g. `v8` for arm64, `v6` / `v7` for arm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Required minimum OS version (Windows-only in practice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// Required OS feature flags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub os_features: Vec<String>,
}

impl Platform {
    pub fn linux_amd64() -> Self {
        Self { architecture: "amd64".into(), os: "linux".into(), ..Default::default() }
    }
    pub fn linux_arm64() -> Self {
        Self {
            architecture: "arm64".into(),
            os: "linux".into(),
            variant: Some("v8".into()),
            ..Default::default()
        }
    }
    pub fn windows_amd64() -> Self {
        Self { architecture: "amd64".into(), os: "windows".into(), ..Default::default() }
    }
    pub fn current_host() -> Self {
        // Runtime-detected platform of the cave-cri process.
        Self {
            architecture: std::env::consts::ARCH.to_string(),
            os: std::env::consts::OS.to_string(),
            variant: None,
            os_version: None,
            os_features: vec![],
        }
    }
}

/// One manifest entry within an image index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestListEntry {
    pub digest: String,
    pub size: u64,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub platform: Platform,
}

/// OCI image index / Docker manifest list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestList {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub manifests: Vec<ManifestListEntry>,
}

impl ManifestList {
    /// Pick the leaf manifest matching `target`. Matching rules mirror
    /// containerd's `platforms.OnlyStrict` — require equal architecture
    /// and os, prefer an exact variant, otherwise accept an entry with no
    /// variant set.
    pub fn select(&self, target: &Platform) -> Option<&ManifestListEntry> {
        let arch_match: Vec<&ManifestListEntry> = self
            .manifests
            .iter()
            .filter(|m| {
                m.platform.architecture == target.architecture
                    && m.platform.os == target.os
            })
            .collect();
        // Exact variant first.
        if let Some(target_variant) = target.variant.as_ref() {
            if let Some(m) = arch_match.iter().find(|m| m.platform.variant.as_ref() == Some(target_variant)) {
                return Some(*m);
            }
        }
        // Then entries with no variant.
        if let Some(m) = arch_match.iter().find(|m| m.platform.variant.is_none()) {
            return Some(*m);
        }
        // Last resort: any arch/os match.
        arch_match.into_iter().next()
    }

    /// True if the list is a multi-arch image index (more than one entry).
    pub fn is_multi_arch(&self) -> bool {
        self.manifests.len() > 1
    }

    /// Distinct platforms advertised.
    pub fn platforms(&self) -> Vec<&Platform> {
        self.manifests.iter().map(|m| &m.platform).collect()
    }
}

/// Recognised manifest-list media types.
pub const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub const DOCKER_MANIFEST_LIST_MEDIA_TYPE: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";

/// True if `media_type` is one of the known multi-arch index types.
pub fn is_index_media_type(media_type: &str) -> bool {
    media_type == OCI_INDEX_MEDIA_TYPE || media_type == DOCKER_MANIFEST_LIST_MEDIA_TYPE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(digest: &str, arch: &str, os: &str, variant: Option<&str>) -> ManifestListEntry {
        ManifestListEntry {
            digest: digest.into(),
            size: 1024,
            media_type: "application/vnd.oci.image.manifest.v1+json".into(),
            platform: Platform {
                architecture: arch.into(),
                os: os.into(),
                variant: variant.map(|v| v.to_string()),
                ..Default::default()
            },
        }
    }

    fn sample_list() -> ManifestList {
        ManifestList {
            schema_version: 2,
            media_type: OCI_INDEX_MEDIA_TYPE.into(),
            manifests: vec![
                entry("sha256:amd", "amd64", "linux", None),
                entry("sha256:arm", "arm64", "linux", Some("v8")),
                entry("sha256:win", "amd64", "windows", None),
            ],
        }
    }

    // ── Platform helpers ────────────────────────────────────────────────────

    #[test]
    fn platform_linux_amd64_has_no_variant() {
        let p = Platform::linux_amd64();
        assert_eq!(p.architecture, "amd64");
        assert_eq!(p.os, "linux");
        assert!(p.variant.is_none());
    }

    #[test]
    fn platform_linux_arm64_uses_v8_variant() {
        let p = Platform::linux_arm64();
        assert_eq!(p.architecture, "arm64");
        assert_eq!(p.variant.as_deref(), Some("v8"));
    }

    #[test]
    fn platform_current_host_is_populated() {
        let p = Platform::current_host();
        assert!(!p.architecture.is_empty());
        assert!(!p.os.is_empty());
    }

    // ── select ──────────────────────────────────────────────────────────────

    #[test]
    fn select_amd64_linux_picks_amd_entry() {
        let list = sample_list();
        let m = list.select(&Platform::linux_amd64()).unwrap();
        assert_eq!(m.digest, "sha256:amd");
    }

    #[test]
    fn select_arm64_v8_picks_arm_entry_with_variant() {
        let list = sample_list();
        let m = list.select(&Platform::linux_arm64()).unwrap();
        assert_eq!(m.digest, "sha256:arm");
    }

    #[test]
    fn select_windows_amd64_picks_windows_entry() {
        let list = sample_list();
        let m = list.select(&Platform::windows_amd64()).unwrap();
        assert_eq!(m.digest, "sha256:win");
    }

    #[test]
    fn select_unknown_platform_returns_none() {
        let list = sample_list();
        let p = Platform { architecture: "riscv64".into(), os: "linux".into(), ..Default::default() };
        assert!(list.select(&p).is_none());
    }

    #[test]
    fn select_prefers_exact_variant_over_unversioned() {
        let list = ManifestList {
            schema_version: 2,
            media_type: OCI_INDEX_MEDIA_TYPE.into(),
            manifests: vec![
                entry("sha256:bare", "arm", "linux", None),
                entry("sha256:v6", "arm", "linux", Some("v6")),
                entry("sha256:v7", "arm", "linux", Some("v7")),
            ],
        };
        let target = Platform { architecture: "arm".into(), os: "linux".into(), variant: Some("v7".into()), ..Default::default() };
        let m = list.select(&target).unwrap();
        assert_eq!(m.digest, "sha256:v7");
    }

    #[test]
    fn select_falls_back_to_unversioned_when_variant_missing() {
        let list = ManifestList {
            schema_version: 2,
            media_type: OCI_INDEX_MEDIA_TYPE.into(),
            manifests: vec![entry("sha256:bare", "arm", "linux", None)],
        };
        let target = Platform { architecture: "arm".into(), os: "linux".into(), variant: Some("v7".into()), ..Default::default() };
        let m = list.select(&target).unwrap();
        assert_eq!(m.digest, "sha256:bare");
    }

    // ── is_multi_arch + platforms ──────────────────────────────────────────

    #[test]
    fn is_multi_arch_true_for_multiple_entries() {
        assert!(sample_list().is_multi_arch());
    }

    #[test]
    fn is_multi_arch_false_for_single_entry() {
        let list = ManifestList {
            schema_version: 2,
            media_type: OCI_INDEX_MEDIA_TYPE.into(),
            manifests: vec![entry("sha256:only", "amd64", "linux", None)],
        };
        assert!(!list.is_multi_arch());
    }

    #[test]
    fn platforms_lists_all_distinct_platforms() {
        let list = sample_list();
        let platforms = list.platforms();
        assert_eq!(platforms.len(), 3);
    }

    // ── media types ─────────────────────────────────────────────────────────

    #[test]
    fn is_index_media_type_recognises_oci_and_docker() {
        assert!(is_index_media_type(OCI_INDEX_MEDIA_TYPE));
        assert!(is_index_media_type(DOCKER_MANIFEST_LIST_MEDIA_TYPE));
        assert!(!is_index_media_type("application/vnd.oci.image.manifest.v1+json"));
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn manifest_list_roundtrips_through_json() {
        let list = sample_list();
        let json = serde_json::to_string(&list).unwrap();
        let back: ManifestList = serde_json::from_str(&json).unwrap();
        assert_eq!(list, back);
    }

    #[test]
    fn platform_omits_empty_optional_fields_in_json() {
        let p = Platform::linux_amd64();
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("variant"));
        assert!(!json.contains("os_version"));
        assert!(!json.contains("os_features"));
    }
}
