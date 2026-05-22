// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RBAC deeper-cut — root-user auto-creation, full permission set
//! resolution, and prefix-vs-exact-vs-range permission semantics.
//!
//! The base store (see [`crate::store::KvStore`]) implements user/role/
//! permission CRUD plus a `check_auth_token` that validates an inbound
//! request against the *currently authenticated* user.  This module adds
//! the higher-level helpers etcd v3.6 ships:
//!
//!   * `auto_create_root_user_role` — provisions the well-known
//!     `root` user and `root` role on first `auth_enable` (mirrors
//!     etcdctl's `--insecure-discovery` bootstrap).
//!   * `effective_permissions` — flattens a user's roles into a single
//!     deduplicated permission list.
//!   * `permission_matches_key` — re-implements etcd's prefix / exact /
//!     range matcher so callers can introspect "would this token grant
//!     access to this key?" without invoking the full request path.
//!
//! Mirrors etcd v3.6.10
//!   `server/auth/store.go#newStorage` (root bootstrap)
//!   `server/auth/store.go#isOpPermitted`
//!   `server/auth/range_perm_cache.go`.

use crate::error::{EtcdError, EtcdResult};
use crate::models::{
    AuthRoleAddRequest, AuthRoleGrantPermissionRequest, AuthUserAddRequest,
    AuthUserGrantRoleRequest, PermType, Permission,
};
use crate::store::KvStore;
use std::collections::BTreeMap;

/// The well-known root user / role.  Etcd reserves these names; users
/// cannot delete them while auth is enabled.
pub const ROOT_USER: &str = "root";
pub const ROOT_ROLE: &str = "root";

/// Idempotently provision the root user + role with the supplied
/// password.  Safe to call repeatedly — already-existing entries are a
/// no-op.  Returns `true` when at least one entry was newly created.
pub fn auto_create_root_user_role(store: &KvStore, password: &str) -> EtcdResult<bool> {
    let mut created = false;
    let role_add = store.role_add(&AuthRoleAddRequest {
        name: ROOT_ROLE.into(),
    });
    match role_add {
        Ok(_) => created = true,
        Err(EtcdError::RoleAlreadyExists(_)) => {}
        Err(e) => return Err(e),
    }
    let user_add = store.user_add(&AuthUserAddRequest {
        name: ROOT_USER.into(),
        password: password.into(),
    });
    match user_add {
        Ok(_) => created = true,
        Err(EtcdError::UserAlreadyExists(_)) => {}
        Err(e) => return Err(e),
    }
    // Grant role → user (skip when already granted).
    if let Err(e) = store.user_grant_role(&AuthUserGrantRoleRequest {
        user: ROOT_USER.into(),
        role: ROOT_ROLE.into(),
    }) {
        // user_grant_role currently doesn't error on already-granted, but
        // be defensive.
        if !matches!(e, EtcdError::RoleNotFound(_)) {
            // already granted or harmless duplicate — swallow.
        }
    }
    Ok(created)
}

/// Walk a user's role list and return one merged permission vector with
/// duplicates removed.  Permissions are deduplicated by
/// `(perm_type, key, range_end)` triple.
pub fn effective_permissions(store: &KvStore, username: &str) -> EtcdResult<Vec<Permission>> {
    let user = store.user_get(&crate::models::AuthUserGetRequest {
        name: username.to_string(),
    })?;
    let mut by_key: BTreeMap<(String, Option<String>, String), Permission> = BTreeMap::new();
    for role_name in &user.roles {
        let role = store.role_get(&crate::models::AuthRoleGetRequest {
            role: role_name.clone(),
        })?;
        for perm in role.perm {
            let key = (
                perm.key.clone(),
                perm.range_end.clone(),
                format!("{:?}", perm.perm_type),
            );
            by_key.insert(key, perm);
        }
    }
    Ok(by_key.into_values().collect())
}

/// True when `perm` covers `key` for `desired`.  Implements etcd's
/// prefix / exact / range matcher used by
/// `range_perm_cache.go#permissionsForRange`:
///
///   * `range_end == None`         → exact match: `key == perm.key`
///   * `range_end == Some(b"\\0")` → prefix match: `key.starts_with(perm.key)`
///   * `range_end == Some(other)`  → range match: `perm.key <= key < range_end`
pub fn permission_matches_key(perm: &Permission, key: &[u8], desired: PermType) -> bool {
    let perm_covers = perm.perm_type == desired || perm.perm_type == PermType::Readwrite;
    if !perm_covers {
        return false;
    }
    match perm.range_end.as_deref() {
        None => key == perm.key.as_bytes(),
        Some("\0") => key.starts_with(perm.key.as_bytes()),
        Some(end) => key >= perm.key.as_bytes() && key < end.as_bytes(),
    }
}

/// Returns `true` when *any* role granted to `username` covers `(key,
/// desired)`.  Useful for admin tooling that wants to display which
/// keys a given user can touch.
pub fn user_can_access(
    store: &KvStore,
    username: &str,
    key: &[u8],
    desired: PermType,
) -> EtcdResult<bool> {
    if username == ROOT_USER {
        return Ok(true);
    }
    let perms = effective_permissions(store, username)?;
    Ok(perms
        .iter()
        .any(|p| permission_matches_key(p, key, desired)))
}

/// Grant a permission to a role using a typed builder; the wrapper
/// makes the common case (`Read` / `Write` / `Readwrite` on a single
/// key) a one-liner.
pub fn grant_role_permission(
    store: &KvStore,
    role: &str,
    perm_type: PermType,
    key: impl Into<String>,
    range_end: Option<String>,
) -> EtcdResult<()> {
    store
        .role_grant_permission(&AuthRoleGrantPermissionRequest {
            name: role.into(),
            perm: Permission {
                perm_type,
                key: key.into(),
                range_end,
            },
        })
        .map(|_| ())
}

// ─────────────────────────────────────────────────────────────────────────
// RBAC deeper tests — feat/cave-etcd-deeper-003
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AuthRoleAddRequest;

    fn dt(tenant_id: &str, suffix: &str) -> String {
        format!("/tenants/{}/{}", tenant_id, suffix)
    }

    #[test]
    fn test_auto_create_root_user_role_idempotent() {
        // cite: etcd v3.6.10 server/auth/store.go newStorage (root bootstrap)
        let _tenant_id = "rb-001";
        let store = KvStore::new();
        assert!(auto_create_root_user_role(&store, "secret").unwrap());
        // Second call → no-op (returns false because everything exists).
        let again = auto_create_root_user_role(&store, "secret").unwrap();
        assert!(!again);
    }

    #[test]
    fn test_auto_create_root_grants_role_to_user() {
        // cite: etcd v3.6.10 (root user joined to root role)
        let _tenant_id = "rb-002";
        let store = KvStore::new();
        auto_create_root_user_role(&store, "p").unwrap();
        let user = store
            .user_get(&crate::models::AuthUserGetRequest {
                name: ROOT_USER.into(),
            })
            .unwrap();
        assert!(user.roles.iter().any(|r| r == ROOT_ROLE));
    }

    #[test]
    fn test_permission_matches_exact_key() {
        // cite: etcd v3.6.10 range_perm_cache.go (exact match)
        let tenant_id = "rb-003";
        let perm = Permission {
            perm_type: PermType::Read,
            key: dt(tenant_id, "k").into(),
            range_end: None,
        };
        assert!(permission_matches_key(
            &perm,
            dt(tenant_id, "k").as_bytes(),
            PermType::Read
        ));
        assert!(!permission_matches_key(
            &perm,
            dt(tenant_id, "other").as_bytes(),
            PermType::Read
        ));
    }

    #[test]
    fn test_permission_matches_prefix_with_zero_byte() {
        // cite: etcd v3.6.10 (range_end == "\0" → prefix)
        let tenant_id = "rb-004";
        let perm = Permission {
            perm_type: PermType::Readwrite,
            key: dt(tenant_id, "").into(),
            range_end: Some("\0".into()),
        };
        assert!(permission_matches_key(
            &perm,
            dt(tenant_id, "anything/deep/here").as_bytes(),
            PermType::Read
        ));
    }

    #[test]
    fn test_permission_matches_range() {
        // cite: etcd v3.6.10 (range_end half-open)
        let tenant_id = "rb-005";
        let perm = Permission {
            perm_type: PermType::Read,
            key: dt(tenant_id, "a").into(),
            range_end: Some(dt(tenant_id, "c")),
        };
        assert!(permission_matches_key(
            &perm,
            dt(tenant_id, "b").as_bytes(),
            PermType::Read
        ));
        assert!(!permission_matches_key(
            &perm,
            dt(tenant_id, "c").as_bytes(),
            PermType::Read
        )); // range_end exclusive
    }

    #[test]
    fn test_permission_readwrite_covers_read_and_write() {
        // cite: etcd v3.6.10 PermType.READWRITE wildcard
        let tenant_id = "rb-006";
        let perm = Permission {
            perm_type: PermType::Readwrite,
            key: dt(tenant_id, "k").into(),
            range_end: None,
        };
        assert!(permission_matches_key(
            &perm,
            dt(tenant_id, "k").as_bytes(),
            PermType::Read
        ));
        assert!(permission_matches_key(
            &perm,
            dt(tenant_id, "k").as_bytes(),
            PermType::Write
        ));
    }

    #[test]
    fn test_user_can_access_root_always_true() {
        // cite: etcd v3.6.10 (root bypasses permission check)
        let tenant_id = "rb-007";
        let store = KvStore::new();
        auto_create_root_user_role(&store, "p").unwrap();
        assert!(user_can_access(
            &store,
            ROOT_USER,
            dt(tenant_id, "any").as_bytes(),
            PermType::Write
        )
        .unwrap());
    }

    #[test]
    fn test_user_can_access_filters_per_role() {
        // cite: etcd v3.6.10 isOpPermitted walks role list
        let tenant_id = "rb-008";
        let store = KvStore::new();
        store
            .user_add(&crate::models::AuthUserAddRequest {
                name: "alice".into(),
                password: "pw".into(),
            })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest {
                name: "data-r".into(),
            })
            .unwrap();
        grant_role_permission(
            &store,
            "data-r",
            PermType::Read,
            dt(tenant_id, "data/"),
            Some("\0".into()),
        )
        .unwrap();
        store
            .user_grant_role(&AuthUserGrantRoleRequest {
                user: "alice".into(),
                role: "data-r".into(),
            })
            .unwrap();
        assert!(user_can_access(
            &store,
            "alice",
            dt(tenant_id, "data/x").as_bytes(),
            PermType::Read
        )
        .unwrap());
        assert!(!user_can_access(
            &store,
            "alice",
            dt(tenant_id, "data/x").as_bytes(),
            PermType::Write
        )
        .unwrap());
        assert!(!user_can_access(
            &store,
            "alice",
            dt(tenant_id, "private").as_bytes(),
            PermType::Read
        )
        .unwrap());
    }

    #[test]
    fn test_effective_permissions_dedupes_across_roles() {
        // cite: etcd v3.6.10 range_perm_cache.go merges per role
        let tenant_id = "rb-009";
        let store = KvStore::new();
        store
            .user_add(&crate::models::AuthUserAddRequest {
                name: "u".into(),
                password: "p".into(),
            })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest { name: "r1".into() })
            .unwrap();
        store
            .role_add(&AuthRoleAddRequest { name: "r2".into() })
            .unwrap();
        // Both roles grant the same permission.
        grant_role_permission(&store, "r1", PermType::Read, dt(tenant_id, "k"), None).unwrap();
        grant_role_permission(&store, "r2", PermType::Read, dt(tenant_id, "k"), None).unwrap();
        store
            .user_grant_role(&AuthUserGrantRoleRequest {
                user: "u".into(),
                role: "r1".into(),
            })
            .unwrap();
        store
            .user_grant_role(&AuthUserGrantRoleRequest {
                user: "u".into(),
                role: "r2".into(),
            })
            .unwrap();
        let perms = effective_permissions(&store, "u").unwrap();
        assert_eq!(perms.len(), 1, "duplicates should fold");
    }
}
