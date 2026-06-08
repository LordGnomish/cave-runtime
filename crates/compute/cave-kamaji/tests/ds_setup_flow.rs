// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for the datastore-setup state machine + per-tenant
//! resource-isolation naming.
//!
//! Faithful port targets (Kamaji v1.0.0):
//!   internal/resources/datastore/datastore_setup.go        — createDB/createUser/grant
//!   internal/resources/datastore/datastore_storage_config.go — schema/user coalesce
//!   internal/resources/datastore/datastore_multitenancy.go — NATS rejection
//!   internal/resources/utils/utils.go                      — UpdateOperationResult

use cave_kamaji::connection::{Connection, Driver, FakeConnection};
use cave_kamaji::ds_setup::{
    OperationResult, SetupResource, StorageSetupStatus, multitenancy_check, run_setup,
    run_teardown, should_status_be_updated, tenant_schema, update_operation_result,
};

// ── Per-tenant naming (storage_config.go coalesceFn) ────────────────────────

#[test]
fn tenant_schema_joins_namespace_and_name() {
    assert_eq!(tenant_schema("tenants", "alpha"), "tenants_alpha");
}

#[test]
fn tenant_schema_replaces_dashes_with_underscores() {
    // issue #328: PostgreSQL rejects '-' in identifiers.
    assert_eq!(tenant_schema("my-ns", "my-tcp"), "my_ns_my_tcp");
}

// ── OperationResult precedence (utils.UpdateOperationResult) ────────────────

#[test]
fn operation_result_created_dominates() {
    assert_eq!(
        update_operation_result(OperationResult::None, OperationResult::Created),
        OperationResult::Created
    );
    assert_eq!(
        update_operation_result(OperationResult::Updated, OperationResult::Created),
        OperationResult::Created
    );
}

#[test]
fn operation_result_updated_beats_none() {
    assert_eq!(
        update_operation_result(OperationResult::None, OperationResult::Updated),
        OperationResult::Updated
    );
    assert_eq!(
        update_operation_result(OperationResult::None, OperationResult::None),
        OperationResult::None
    );
}

// ── Setup CreateOrUpdate flow ───────────────────────────────────────────────

fn resource() -> SetupResource {
    SetupResource {
        schema: "tenants_alpha".into(),
        user: "tenants_alpha".into(),
        password: "s3cret".into(),
    }
}

#[test]
fn fresh_setup_creates_db_user_and_grant_in_order() {
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    let res = run_setup(&mut conn, &resource()).unwrap();
    assert_eq!(res, OperationResult::Created);

    let log = conn.statement_log();
    // exact upstream ordering: createDB -> createUser -> grant
    let db = log.iter().position(|s| s.starts_with("CREATE DATABASE")).unwrap();
    let user = log.iter().position(|s| s.starts_with("CREATE ROLE")).unwrap();
    let grant = log.iter().position(|s| s.starts_with("GRANT")).unwrap();
    assert!(db < user && user < grant, "db<user<grant: {log:?}");
}

#[test]
fn rerunning_setup_is_idempotent_and_reports_none() {
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    run_setup(&mut conn, &resource()).unwrap();
    let before = conn.statement_log().len();
    let res = run_setup(&mut conn, &resource()).unwrap();
    assert_eq!(res, OperationResult::None);
    assert_eq!(
        conn.statement_log().len(),
        before,
        "no new DDL on a converged datastore"
    );
}

#[test]
fn teardown_revokes_then_drops_db_then_drops_user() {
    let mut conn = FakeConnection::new(Driver::PostgreSql);
    run_setup(&mut conn, &resource()).unwrap();
    run_teardown(&mut conn, &resource()).unwrap();

    assert!(!conn.db_exists("tenants_alpha").unwrap());
    assert!(!conn.user_exists("tenants_alpha").unwrap());
    let log = conn.statement_log();
    let revoke = log.iter().rposition(|s| s.starts_with("REVOKE")).unwrap();
    let dropdb = log.iter().rposition(|s| s.starts_with("DROP DATABASE")).unwrap();
    let dropuser = log.iter().rposition(|s| s.starts_with("DROP ROLE")).unwrap();
    assert!(revoke < dropdb && dropdb < dropuser, "revoke<dropdb<dropuser: {log:?}");
}

// ── ShouldStatusBeUpdated (datastore_setup.go) ──────────────────────────────

#[test]
fn status_update_triggers_on_driver_or_checksum_or_user_or_schema_drift() {
    let res = resource();
    // converged: same driver, checksum matches config, same user+schema -> no update
    let converged = StorageSetupStatus {
        driver: "PostgreSQL".into(),
        checksum: "abc".into(),
        user: res.user.clone(),
        schema: res.schema.clone(),
    };
    assert!(!should_status_be_updated(&converged, &res, "PostgreSQL", "abc"));

    // driver drift
    assert!(should_status_be_updated(&converged, &res, "MySQL", "abc"));
    // config checksum drift
    assert!(should_status_be_updated(&converged, &res, "PostgreSQL", "xyz"));
    // user drift
    let mut s = converged.clone();
    s.user = "other".into();
    assert!(should_status_be_updated(&s, &res, "PostgreSQL", "abc"));
}

// ── MultiTenancy NATS rejection (datastore_multitenancy.go) ─────────────────

#[test]
fn non_nats_driver_never_blocks_sharing() {
    let used = vec!["nsA/tcpA".to_string()];
    assert!(multitenancy_check(Driver::PostgreSql, &used, "nsB/tcpB").is_ok());
}

#[test]
fn nats_allows_reclaim_by_existing_owner_or_when_unused() {
    // already the owner
    let used = vec!["nsA/tcpA".to_string()];
    assert!(multitenancy_check(Driver::Nats, &used, "nsA/tcpA").is_ok());
    // unused datastore
    assert!(multitenancy_check(Driver::Nats, &[], "nsA/tcpA").is_ok());
}

#[test]
fn nats_rejects_a_second_tenant() {
    let used = vec!["nsA/tcpA".to_string()];
    assert!(multitenancy_check(Driver::Nats, &used, "nsB/tcpB").is_err());
}
