// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Catalog permission constants — port from @backstage/catalog-backend
//!
//! Upstream: backstage/plugins/catalog-backend/src/permissions.ts

use crate::models::{Permission, PermissionAction, PermissionAttributes};

// Upstream: catalog.entity.read
pub const CATALOG_ENTITY_READ: &str = "catalog.entity.read";
// Upstream: catalog.entity.create
pub const CATALOG_ENTITY_CREATE: &str = "catalog.entity.create";
// Upstream: catalog.entity.refresh
pub const CATALOG_ENTITY_REFRESH: &str = "catalog.entity.refresh";
// Upstream: catalog.entity.update
pub const CATALOG_ENTITY_UPDATE: &str = "catalog.entity.update";
// Upstream: catalog.entity.delete
pub const CATALOG_ENTITY_DELETE: &str = "catalog.entity.delete";
// Upstream: catalog.location.read
pub const CATALOG_LOCATION_READ: &str = "catalog.location.read";
// Upstream: catalog.location.create
pub const CATALOG_LOCATION_CREATE: &str = "catalog.location.create";
// Upstream: catalog.location.delete
pub const CATALOG_LOCATION_DELETE: &str = "catalog.location.delete";

/// Helper to create Permission for catalog.entity.read
pub fn catalog_entity_read_permission() -> Permission {
    Permission {
        name: CATALOG_ENTITY_READ.to_string(),
        attributes: PermissionAttributes {
            action: Some(PermissionAction::Read),
        },
    }
}

/// Helper to create Permission for catalog.entity.create
pub fn catalog_entity_create_permission() -> Permission {
    Permission {
        name: CATALOG_ENTITY_CREATE.to_string(),
        attributes: PermissionAttributes {
            action: Some(PermissionAction::Create),
        },
    }
}

/// Helper to create Permission for catalog.entity.delete
pub fn catalog_entity_delete_permission() -> Permission {
    Permission {
        name: CATALOG_ENTITY_DELETE.to_string(),
        attributes: PermissionAttributes {
            action: Some(PermissionAction::Delete),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_permission_names_match_upstream() {
        assert_eq!(CATALOG_ENTITY_READ, "catalog.entity.read");
        assert_eq!(CATALOG_ENTITY_CREATE, "catalog.entity.create");
        assert_eq!(CATALOG_ENTITY_REFRESH, "catalog.entity.refresh");
        assert_eq!(CATALOG_ENTITY_UPDATE, "catalog.entity.update");
        assert_eq!(CATALOG_ENTITY_DELETE, "catalog.entity.delete");
        assert_eq!(CATALOG_LOCATION_READ, "catalog.location.read");
        assert_eq!(CATALOG_LOCATION_CREATE, "catalog.location.create");
        assert_eq!(CATALOG_LOCATION_DELETE, "catalog.location.delete");
    }

    #[test]
    fn catalog_entity_read_has_read_action() {
        let perm = catalog_entity_read_permission();
        assert_eq!(perm.name, CATALOG_ENTITY_READ);
        assert_eq!(perm.attributes.action, Some(PermissionAction::Read));
    }

    #[test]
    fn catalog_entity_create_has_create_action() {
        let perm = catalog_entity_create_permission();
        assert_eq!(perm.name, CATALOG_ENTITY_CREATE);
        assert_eq!(perm.attributes.action, Some(PermissionAction::Create));
    }

    #[test]
    fn catalog_entity_delete_has_delete_action() {
        let perm = catalog_entity_delete_permission();
        assert_eq!(perm.name, CATALOG_ENTITY_DELETE);
        assert_eq!(perm.attributes.action, Some(PermissionAction::Delete));
    }
}
