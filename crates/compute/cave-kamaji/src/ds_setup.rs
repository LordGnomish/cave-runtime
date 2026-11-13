// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Datastore-setup state machine + per-tenant resource-isolation naming.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/resources/datastore/datastore_setup.go         — Setup resource
//!   internal/resources/datastore/datastore_storage_config.go — coalesce naming
//!   internal/resources/datastore/datastore_multitenancy.go  — NATS rejection
//!   internal/resources/utils/utils.go                       — UpdateOperationResult
//!
//! When a TenantControlPlane binds a SQL DataStore, Kamaji carves an isolated
//! schema + role + grant out of the shared back-end — one per tenant — so two
//! tenants never share a database. This module ports that decision logic:
//! the deterministic per-tenant naming, the idempotent create/teardown order,
//! the status-drift predicate, and the single-tenant NATS boundary. The actual
//! SQL is executed through a [`Connection`] (see [`crate::connection`]).

use crate::connection::{Connection, DatastoreError, Driver};

/// Mirror of `controllerutil.OperationResult` (the subset Kamaji datastore-setup
/// produces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationResult {
    None,
    Created,
    Updated,
    UpdatedStatus,
    UpdatedStatusOnly,
}

/// `utils.UpdateOperationResult` — collapses two results to the most
/// significant. Precedence: Created > Updated > UpdatedStatus >
/// UpdatedStatusOnly > None.
pub fn update_operation_result(current: OperationResult, op: OperationResult) -> OperationResult {
    use OperationResult::*;
    if current == Created || op == Created {
        return Created;
    }
    if current == Updated || op == Updated {
        return Updated;
    }
    if current == UpdatedStatus || op == UpdatedStatus {
        return UpdatedStatus;
    }
    if current == UpdatedStatusOnly || op == UpdatedStatusOnly {
        return UpdatedStatusOnly;
    }
    None
}

/// Per-tenant schema/user name — `storage_config.go` coalesce default:
/// `"{namespace}_{name}"` with every `-` replaced by `_` (PostgreSQL rejects
/// dashes in identifiers, issue #328). This is the core resource-isolation
/// boundary: each tenant's data lives under a name derived solely from its
/// namespaced identity, so names never collide across tenants.
pub fn tenant_schema(namespace: &str, name: &str) -> String {
    format!("{namespace}_{name}").replace('-', "_")
}

/// The DataStore Secret payload the setup controller consumes
/// (`SetupResource` in datastore_setup.go: DB_SCHEMA / DB_USER / DB_PASSWORD).
#[derive(Debug, Clone)]
pub struct SetupResource {
    pub schema: String,
    pub user: String,
    pub password: String,
}

/// Mirror of `tcp.Status.Storage.Setup` — what the controller persists after a
/// successful setup, used to detect drift.
#[derive(Debug, Clone)]
pub struct StorageSetupStatus {
    pub driver: String,
    pub checksum: String,
    pub user: String,
    pub schema: String,
}

/// `(*Setup).ShouldStatusBeUpdated` — the datastore status must be rewritten
/// when the driver changed, the setup checksum no longer matches the current
/// config checksum, or the user/schema drifted from the resolved resource.
pub fn should_status_be_updated(
    status: &StorageSetupStatus,
    resource: &SetupResource,
    current_driver: &str,
    config_checksum: &str,
) -> bool {
    status.driver != current_driver
        || status.checksum != config_checksum
        || status.user != resource.user
        || status.schema != resource.schema
}

/// `(*Setup).CreateOrUpdate` — idempotently materialise the tenant's database,
/// role, and grant, in that exact order, accumulating the operation result.
pub fn run_setup(
    conn: &mut dyn Connection,
    res: &SetupResource,
) -> Result<OperationResult, DatastoreError> {
    let mut result = OperationResult::None;

    // createDB
    if !conn.db_exists(&res.schema)? {
        conn.create_db(&res.schema)?;
        result = update_operation_result(result, OperationResult::Created);
    }
    // createUser
    if !conn.user_exists(&res.user)? {
        conn.create_user(&res.user, &res.password)?;
        result = update_operation_result(result, OperationResult::Created);
    }
    // createGrantPrivileges
    if !conn.grant_privileges_exists(&res.user, &res.schema)? {
        conn.grant_privileges(&res.user, &res.schema)?;
        result = update_operation_result(result, OperationResult::Created);
    }

    Ok(result)
}

/// `(*Setup).Delete` — tear the tenant's datastore down in the upstream order:
/// revoke privileges, drop the database, drop the user.
pub fn run_teardown(conn: &mut dyn Connection, res: &SetupResource) -> Result<(), DatastoreError> {
    if conn.grant_privileges_exists(&res.user, &res.schema)? {
        conn.revoke_privileges(&res.user, &res.schema)?;
    }
    if conn.db_exists(&res.schema)? {
        conn.delete_db(&res.schema)?;
    }
    if conn.user_exists(&res.user)? {
        conn.delete_user(&res.user)?;
    }
    Ok(())
}

/// `(*MultiTenancy).CreateOrUpdate` — enforce the single-tenant boundary for
/// NATS, which has no programmatic multi-tenant isolation. Every other driver
/// is freely shareable. `tcp_key` is the reclaiming tenant's `namespace/name`;
/// `used_by` is the DataStore's `status.usedBy` list.
pub fn multitenancy_check(driver: Driver, used_by: &[String], tcp_key: &str) -> Result<(), String> {
    if driver != Driver::Nats {
        return Ok(());
    }
    if used_by.iter().any(|k| k == tcp_key) {
        // already the owner
        return Ok(());
    }
    if used_by.is_empty() {
        // free to claim
        return Ok(());
    }
    Err("NATS doesn't support multi-tenancy, the current datastore is already in use".to_string())
}
