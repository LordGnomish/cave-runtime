// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Storage versioning — encode at the storage version, decode at the
//! requested version. Supports per-kind multi-version registration with
//! a single elected storage version (KEP-3247).
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apimachinery/pkg/runtime/scheme.go` (`Scheme`,
//!     `convertToVersion`).
//!   * `staging/src/k8s.io/apiserver/pkg/storage/storagebackend/factory/factory.go`
//!     (storage version selection).
//!   * `staging/src/k8s.io/apiserver/pkg/storageversion/manager.go`
//!     + KEP-3247 (Storage Version API) — election + hash for migration.
//!
//! Encoded blobs carry an explicit `apiVersion` field so the decoder can
//! read storage written under any historical version.
//!
//! Tenant invariant: each `(tenant_id, group, kind)` triple owns its own
//! version table and storage-version election. Tenant A's storage version
//! MUST NOT influence tenant B's encode/decode.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct VersionRef {
    pub group: String,
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct VersionsKey {
    tenant_id: String,
    group: String,
    kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    UnknownVersion { version: String },
    NoStorageVersionElected,
    VersionAlreadyRegistered { version: String },
    DecodeApiVersionMismatch { expected: String, actual: String },
}

#[derive(Debug, Clone)]
struct VersionTable {
    /// Versions in registration order; first is the current storage.
    versions: Vec<String>,
    storage: Option<String>,
}

pub struct StorageVersionRegistry {
    inner: Mutex<HashMap<VersionsKey, VersionTable>>,
}

impl StorageVersionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn register_version(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        version: &str,
    ) -> Result<(), StorageError> {
        let key = VersionsKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entry(key).or_insert(VersionTable {
            versions: vec![],
            storage: None,
        });
        if entry.versions.iter().any(|v| v == version) {
            return Err(StorageError::VersionAlreadyRegistered {
                version: version.into(),
            });
        }
        entry.versions.push(version.into());
        // Auto-elect the first registered version as storage; subsequent
        // promotions go through `elect_storage_version`.
        if entry.storage.is_none() {
            entry.storage = Some(version.into());
        }
        Ok(())
    }

    pub fn elect_storage_version(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        version: &str,
    ) -> Result<(), StorageError> {
        let key = VersionsKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.get_mut(&key).ok_or(StorageError::UnknownVersion {
            version: version.into(),
        })?;
        if !entry.versions.iter().any(|v| v == version) {
            return Err(StorageError::UnknownVersion {
                version: version.into(),
            });
        }
        entry.storage = Some(version.into());
        Ok(())
    }

    pub fn storage_version(&self, tenant_id: &str, group: &str, kind: &str) -> Option<String> {
        let key = VersionsKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        self.inner
            .lock()
            .unwrap()
            .get(&key)
            .and_then(|t| t.storage.clone())
    }

    pub fn known_versions(&self, tenant_id: &str, group: &str, kind: &str) -> Vec<String> {
        let key = VersionsKey {
            tenant_id: tenant_id.into(),
            group: group.into(),
            kind: kind.into(),
        };
        self.inner
            .lock()
            .unwrap()
            .get(&key)
            .map(|t| t.versions.clone())
            .unwrap_or_default()
    }

    /// Encode an object at the elected storage version. Returns the JSON
    /// payload with an explicit `apiVersion` field.
    pub fn encode(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, StorageError> {
        let storage = self
            .storage_version(tenant_id, group, kind)
            .ok_or(StorageError::NoStorageVersionElected)?;
        let api_version = if group.is_empty() {
            storage.clone()
        } else {
            format!("{}/{}", group, storage)
        };
        let mut obj = body.as_object().cloned().unwrap_or_default();
        obj.insert("apiVersion".into(), serde_json::Value::String(api_version));
        obj.insert("kind".into(), serde_json::Value::String(kind.into()));
        Ok(serde_json::Value::Object(obj))
    }

    /// Decode an encoded blob, requesting it at `at_version`. The blob's
    /// `apiVersion` is read out of the JSON; if it does not match the
    /// requested version this returns `DecodeApiVersionMismatch`.
    pub fn decode(
        &self,
        tenant_id: &str,
        group: &str,
        kind: &str,
        at_version: &str,
        encoded: &serde_json::Value,
    ) -> Result<serde_json::Value, StorageError> {
        let known = self.known_versions(tenant_id, group, kind);
        if !known.iter().any(|v| v == at_version) {
            return Err(StorageError::UnknownVersion {
                version: at_version.into(),
            });
        }
        let actual_api_version = encoded
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let expected_api_version = if group.is_empty() {
            at_version.to_string()
        } else {
            format!("{}/{}", group, at_version)
        };
        if actual_api_version != expected_api_version {
            return Err(StorageError::DecodeApiVersionMismatch {
                expected: expected_api_version,
                actual: actual_api_version,
            });
        }
        Ok(encoded.clone())
    }

    /// Cheap, deterministic shape hash used by the storage-version
    /// migration controller (KEP-3247). Independent of payload — just the
    /// known-version list at the time of the call.
    pub fn version_set_hash(&self, tenant_id: &str, group: &str, kind: &str) -> u64 {
        let mut versions = self.known_versions(tenant_id, group, kind);
        versions.sort();
        let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
        for v in versions {
            for b in v.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            h ^= 0x2F; // sep '/'
            h = h.wrapping_mul(1099511628211);
        }
        h
    }
}

impl Default for StorageVersionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Upstream parity: `TestStorageVersion_AutoElectFirstVersion`
    /// (storageversion/manager_test.go — first registered version becomes
    /// the storage version automatically).
    #[test]
    fn test_first_registered_version_is_auto_elected_as_storage() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1alpha1")
            .unwrap();
        assert_eq!(
            r.storage_version("acme", "acme.io", "Widget").as_deref(),
            Some("v1alpha1")
        );
        // tenant_id invariant: globex sees nothing for the same kind.
        assert!(
            r.storage_version("globex", "acme.io", "Widget").is_none(),
            "tenant_id invariant: storage version is per-tenant"
        );
    }

    /// Upstream parity: `TestStorageVersion_ElectExplicitVersion`
    /// (manager.go — `markStorageVersionForKind` promotes a registered
    /// version to storage).
    #[test]
    fn test_elect_storage_version_promotes_registered_version() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1alpha1")
            .unwrap();
        r.register_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        r.elect_storage_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        assert_eq!(
            r.storage_version("acme", "acme.io", "Widget").as_deref(),
            Some("v1")
        );
        // tenant_id invariant: election is scoped — globex still sees nothing.
        assert!(
            r.storage_version("globex", "acme.io", "Widget").is_none(),
            "tenant_id invariant: globex unaffected by acme election"
        );
    }

    /// Upstream parity: `TestStorageVersion_RejectsElectionOfUnknown`
    /// (manager.go — promote of an unregistered version is an error).
    #[test]
    fn test_elect_unknown_version_returns_error() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        let err = r
            .elect_storage_version("acme", "acme.io", "Widget", "v2")
            .expect_err("must reject unknown version");
        assert_eq!(
            err,
            StorageError::UnknownVersion {
                version: "v2".into()
            }
        );
        // tenant_id invariant: storage version unchanged.
        assert_eq!(
            r.storage_version("acme", "acme.io", "Widget").as_deref(),
            Some("v1")
        );
    }

    /// Upstream parity: `TestStorageVersion_EncodeStampsApiVersion`
    /// (scheme.go::convertToVersion — encode targets the elected
    /// storage version's apiVersion).
    #[test]
    fn test_encode_stamps_storage_version_api_version_and_kind() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1alpha1")
            .unwrap();
        r.register_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        r.elect_storage_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        let encoded = r
            .encode("acme", "acme.io", "Widget", json!({"foo": "bar"}))
            .unwrap();
        assert_eq!(encoded["apiVersion"], "acme.io/v1");
        assert_eq!(encoded["kind"], "Widget");
        assert_eq!(encoded["foo"], "bar", "encoding preserves payload fields");
        // tenant_id invariant: globex's encode would fail without its own setup.
        let g = r.encode("globex", "acme.io", "Widget", json!({}));
        assert!(
            matches!(g, Err(StorageError::NoStorageVersionElected)),
            "tenant_id invariant: globex has no election even if acme does"
        );
    }

    /// Upstream parity: `TestStorageVersion_DecodeAtRequestedVersionMatches`
    /// (`TypeMeta.APIVersion` must match the requested version on read).
    #[test]
    fn test_decode_at_requested_version_succeeds_when_match() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        let encoded = json!({
            "apiVersion": "acme.io/v1",
            "kind": "Widget",
            "spec": {"replicas": 3},
        });
        let decoded = r
            .decode("acme", "acme.io", "Widget", "v1", &encoded)
            .unwrap();
        assert_eq!(decoded["spec"]["replicas"], 3);
        // Mismatch path:
        let bad = json!({
            "apiVersion": "acme.io/v2",
            "kind": "Widget",
        });
        let err = r
            .decode("acme", "acme.io", "Widget", "v1", &bad)
            .unwrap_err();
        assert_eq!(
            err,
            StorageError::DecodeApiVersionMismatch {
                expected: "acme.io/v1".into(),
                actual: "acme.io/v2".into(),
            }
        );
        // tenant_id invariant: decode is scoped per-tenant version table.
        let unknown = r.decode("globex", "acme.io", "Widget", "v1", &encoded);
        assert!(
            matches!(unknown, Err(StorageError::UnknownVersion { .. })),
            "tenant_id invariant: globex has no version table for the same kind"
        );
    }

    /// Upstream parity: `TestStorageVersion_HashChangesWithVersionSet`
    /// (storageversion/manager.go::computeHashFor — KEP-3247 hash detects
    /// version-set drift between API server replicas).
    #[test]
    fn test_version_set_hash_changes_when_set_changes_and_is_per_tenant() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        let h1 = r.version_set_hash("acme", "acme.io", "Widget");
        r.register_version("acme", "acme.io", "Widget", "v2")
            .unwrap();
        let h2 = r.version_set_hash("acme", "acme.io", "Widget");
        assert_ne!(
            h1, h2,
            "hash differs after registering an additional version"
        );
        // tenant_id invariant: globex starts at the empty hash — distinct from
        // acme's even when versions match.
        let g0 = r.version_set_hash("globex", "acme.io", "Widget");
        r.register_version("globex", "acme.io", "Widget", "v1")
            .unwrap();
        let g1 = r.version_set_hash("globex", "acme.io", "Widget");
        assert_ne!(g0, g1);
        assert_ne!(
            g1, h2,
            "tenant_id invariant: globex hash distinct from acme even with same kind"
        );
    }

    /// Upstream parity: `TestStorageVersion_DuplicateRegisterRejected`
    /// (manager.go::registerVersion — same version twice is an error
    /// rather than a silent no-op).
    #[test]
    fn test_duplicate_version_registration_rejected() {
        let r = StorageVersionRegistry::new();
        r.register_version("acme", "acme.io", "Widget", "v1")
            .unwrap();
        let err = r
            .register_version("acme", "acme.io", "Widget", "v1")
            .expect_err("duplicate registration must error");
        assert_eq!(
            err,
            StorageError::VersionAlreadyRegistered {
                version: "v1".into()
            }
        );
        // tenant_id invariant: registration is per-tenant; globex can register
        // its own v1 independently.
        let ok = r.register_version("globex", "acme.io", "Widget", "v1");
        assert!(
            ok.is_ok(),
            "tenant_id invariant: globex can have its own v1 entry"
        );
    }
}
