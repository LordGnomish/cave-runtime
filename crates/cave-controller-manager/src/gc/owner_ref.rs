// SPDX-License-Identifier: AGPL-3.0-or-later
//! OwnerReference validation — `pkg/apis/meta/v1/types.go::OwnerReference`
//! plus `pkg/controller/garbagecollector/operations.go` consistency checks.

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One entry of `metadata.ownerReferences[]`. Mirrors `metav1.OwnerReference`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerReference {
    pub uid: String,
    pub name: String,
    pub kind: String,
    pub api_version: String,
    /// At most one controller per object. `controller: true` means the GC
    /// considers this owner the *primary* owner.
    pub controller: bool,
    /// In Foreground mode, owner deletion blocks until this dependent is gone.
    pub block_owner_deletion: bool,
}

impl OwnerReference {
    pub fn new(uid: impl Into<String>, name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            uid: uid.into(),
            name: name.into(),
            kind: kind.into(),
            api_version: "v1".into(),
            controller: false,
            block_owner_deletion: false,
        }
    }

    pub fn as_controller(mut self) -> Self {
        self.controller = true;
        self
    }

    pub fn blocking(mut self) -> Self {
        self.block_owner_deletion = true;
        self
    }
}

/// Validate the `ownerReferences[]` slice on a single object.
///
/// Mirrors:
/// * `pkg/apis/core/validation/validation.go::ValidateOwnerReferences`
/// * the GC's per-node consistency check in `dependent_graph.go::insertNode`.
///
/// Rules enforced:
///
/// * At most one entry may have `controller = true`.
/// * UIDs must be unique within the slice.
/// * UIDs and names must be non-empty.
pub fn validate_owner_refs(refs: &[OwnerReference]) -> Result<(), ControllerError> {
    let mut seen_uids = HashSet::new();
    let mut controller_count = 0u32;
    for r in refs {
        if r.uid.is_empty() {
            return Err(ControllerError::InvalidSpec {
                kind: "OwnerReference",
                reason: "uid must be non-empty".into(),
            });
        }
        if r.name.is_empty() {
            return Err(ControllerError::InvalidSpec {
                kind: "OwnerReference",
                reason: "name must be non-empty".into(),
            });
        }
        if !seen_uids.insert(r.uid.clone()) {
            return Err(ControllerError::InvalidSpec {
                kind: "OwnerReference",
                reason: format!("duplicate uid {}", r.uid),
            });
        }
        if r.controller {
            controller_count += 1;
            if controller_count > 1 {
                return Err(ControllerError::InvalidSpec {
                    kind: "OwnerReference",
                    reason: "at most one ownerReference can be controller".into(),
                });
            }
        }
    }
    Ok(())
}

/// Returns the controller owner, if any.
pub fn controller_of(refs: &[OwnerReference]) -> Option<&OwnerReference> {
    refs.iter().find(|r| r.controller)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/garbagecollector/operations.go",
    "OwnerReference",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn r(uid: &str) -> OwnerReference {
        OwnerReference::new(uid, format!("obj-{uid}"), "Pod")
    }

    #[test]
    fn empty_owner_refs_validate_ok() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/operations.go",
            "validateOwnerReferences",
            "tenant-gc-or-empty"
        );
        assert!(validate_owner_refs(&[]).is_ok());
    }

    #[test]
    fn single_owner_ref_validates() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/operations.go",
            "validateOwnerReferences",
            "tenant-gc-or-single"
        );
        assert!(validate_owner_refs(&[r("uid-1")]).is_ok());
    }

    #[test]
    fn duplicate_uid_is_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/operations.go",
            "validateOwnerReferences",
            "tenant-gc-or-dup-uid"
        );
        assert!(validate_owner_refs(&[r("u1"), r("u1")]).is_err());
    }

    #[test]
    fn empty_uid_is_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/core/validation/validation.go",
            "ValidateOwnerReferences",
            "tenant-gc-or-empty-uid"
        );
        assert!(validate_owner_refs(&[r("")]).is_err());
    }

    #[test]
    fn empty_name_is_rejected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/core/validation/validation.go",
            "ValidateOwnerReferences",
            "tenant-gc-or-empty-name"
        );
        let mut o = r("u1");
        o.name = String::new();
        assert!(validate_owner_refs(&[o]).is_err());
    }

    #[test]
    fn at_most_one_controller_owner() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/core/validation/validation.go",
            "ValidateOwnerReferences",
            "tenant-gc-or-multi-ctrl"
        );
        let refs = vec![r("u1").as_controller(), r("u2").as_controller()];
        assert!(validate_owner_refs(&refs).is_err());
    }

    #[test]
    fn one_controller_alongside_non_controllers_is_ok() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/core/validation/validation.go",
            "ValidateOwnerReferences",
            "tenant-gc-or-mixed"
        );
        let refs = vec![r("u1").as_controller(), r("u2"), r("u3")];
        assert!(validate_owner_refs(&refs).is_ok());
    }

    #[test]
    fn controller_of_returns_first_marked_controller() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/operations.go",
            "ControllerRef",
            "tenant-gc-or-ctrl-of"
        );
        let refs = vec![r("u1"), r("u2").as_controller(), r("u3")];
        assert_eq!(controller_of(&refs).unwrap().uid, "u2");
    }

    #[test]
    fn controller_of_none_when_no_controller() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/garbagecollector/operations.go",
            "ControllerRef",
            "tenant-gc-or-no-ctrl"
        );
        let refs = vec![r("u1"), r("u2")];
        assert!(controller_of(&refs).is_none());
    }

    #[test]
    fn block_owner_deletion_is_independent_of_controller() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/apis/core/validation/validation.go",
            "ValidateOwnerReferences",
            "tenant-gc-or-block-ind"
        );
        // Both flags can be set, only set, or unset — all valid.
        let refs = vec![
            r("u1").blocking(),
            r("u2").as_controller().blocking(),
            r("u3"),
        ];
        assert!(validate_owner_refs(&refs).is_ok());
    }
}
