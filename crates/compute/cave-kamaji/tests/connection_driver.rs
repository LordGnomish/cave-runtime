// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD spec for the datastore driver abstraction (kine support).
//!
//! Faithful port targets (Kamaji v1.0.0):
//!   api/v1alpha1/datastore_types.go            — Driver enum
//!   internal/datastore/postgresql.go           — Postgres DDL + kine table
//!   internal/datastore/mysql.go                — MySQL DDL
//!   internal/resources/datastore/datastore_setup.go — createDB/createUser/grant flow

use cave_kamaji::connection::{
    Connection, DatastoreError, Driver, FakeConnection, kine_table_ddl, mysql, postgres, run_migrate,
};

// ── Driver enum (datastore_types.go) ────────────────────────────────────────

#[test]
fn driver_wire_values_match_upstream() {
    assert_eq!(Driver::Etcd.as_str(), "etcd");
    assert_eq!(Driver::MySql.as_str(), "MySQL");
    assert_eq!(Driver::PostgreSql.as_str(), "PostgreSQL");
    assert_eq!(Driver::Nats.as_str(), "NATS");
}

#[test]
fn driver_round_trips_from_str() {
    for d in [Driver::Etcd, Driver::MySql, Driver::PostgreSql, Driver::Nats] {
        assert_eq!(Driver::from_wire(d.as_str()), Some(d));
    }
    assert_eq!(Driver::from_wire("bogus"), None);
}

#[test]
fn kine_drivers_classified() {
    // etcd is native; MySQL/PostgreSQL/NATS run through Kine.
    assert!(!Driver::Etcd.is_kine());
    assert!(Driver::MySql.is_kine());
    assert!(Driver::PostgreSql.is_kine());
    assert!(Driver::Nats.is_kine());
}

#[test]
fn only_nats_lacks_multitenancy() {
    // datastore_multitenancy.go: NATS doesn't support multi-tenancy.
    assert!(Driver::Etcd.supports_multitenancy());
    assert!(Driver::MySql.supports_multitenancy());
    assert!(Driver::PostgreSql.supports_multitenancy());
    assert!(!Driver::Nats.supports_multitenancy());
}

// ── Postgres DDL renderer (postgresql.go constants) ─────────────────────────

#[test]
fn postgres_ddl_matches_upstream_statements() {
    assert_eq!(postgres::create_db("t_a"), "CREATE DATABASE t_a");
    // CREATE ROLE %s LOGIN PASSWORD ?  — password bound as a parameter upstream.
    assert_eq!(postgres::create_user("t_a"), "CREATE ROLE t_a LOGIN PASSWORD ?");
    assert_eq!(
        postgres::grant_privileges("t_a", "t_a"),
        "GRANT ALL PRIVILEGES ON DATABASE t_a TO t_a"
    );
    assert_eq!(
        postgres::change_owner("t_a", "t_a"),
        "ALTER DATABASE t_a OWNER TO t_a"
    );
    assert_eq!(
        postgres::revoke_privileges("t_a", "t_a"),
        "REVOKE ALL PRIVILEGES ON DATABASE t_a FROM t_a"
    );
    assert_eq!(postgres::drop_role("t_a"), "DROP ROLE t_a");
    assert_eq!(postgres::drop_db("t_a"), "DROP DATABASE t_a WITH (FORCE)");
}

#[test]
fn kine_table_ddl_is_postgres_schema_with_indexes() {
    let ddl = kine_table_ddl();
    // CREATE TABLE + TRUNCATE + 4 plain indexes + 1 unique index = 7 statements.
    assert_eq!(ddl.len(), 7);
    assert!(ddl[0].contains("CREATE TABLE IF NOT EXISTS kine"));
    assert!(ddl[0].contains("id SERIAL PRIMARY KEY"));
    assert!(ddl[0].contains("value bytea"));
    assert_eq!(ddl[1], "TRUNCATE TABLE kine");
    assert!(ddl.iter().any(|s| s.contains("kine_name_index")));
    assert!(
        ddl.iter()
            .any(|s| s.contains("CREATE UNIQUE INDEX IF NOT EXISTS kine_name_prev_revision_uindex"))
    );
}

// ── MySQL DDL renderer (mysql.go constants) ─────────────────────────────────

#[test]
fn mysql_ddl_matches_upstream_statements() {
    assert_eq!(mysql::create_db("t_a"), "CREATE DATABASE IF NOT EXISTS t_a");
    assert_eq!(
        mysql::create_user("t_a", "secret"),
        "CREATE USER `t_a`@`%` IDENTIFIED BY 'secret'"
    );
    // Upstream GrantPrivileges(user, dbName) fills (%s=user, %s=dbName) — replicate arg order.
    assert_eq!(
        mysql::grant_privileges("t_a", "t_a"),
        "GRANT ALL PRIVILEGES ON `t_a`.* TO `t_a`@`%`"
    );
    assert_eq!(mysql::drop_db("t_a"), "DROP DATABASE IF EXISTS `t_a`");
    assert_eq!(mysql::drop_user("t_a"), "DROP USER IF EXISTS `t_a`");
    assert_eq!(
        mysql::revoke_privileges("t_a", "t_a"),
        "REVOKE ALL PRIVILEGES ON `t_a`.* FROM `t_a`"
    );
}

// ── In-memory Connection backend (drives the Setup flow) ────────────────────

#[test]
fn fake_connection_create_is_idempotent() {
    let mut c = FakeConnection::new(Driver::PostgreSql);
    assert!(!c.db_exists("t_a").unwrap());
    c.create_db("t_a").unwrap();
    assert!(c.db_exists("t_a").unwrap());
    // creating again must not error and must not double-record.
    c.create_db("t_a").unwrap();
    assert_eq!(c.databases().len(), 1);
}

#[test]
fn fake_connection_user_and_grant_lifecycle() {
    let mut c = FakeConnection::new(Driver::PostgreSql);
    c.create_db("t_a").unwrap();
    assert!(!c.user_exists("t_a").unwrap());
    c.create_user("t_a", "pw").unwrap();
    assert!(c.user_exists("t_a").unwrap());

    assert!(!c.grant_privileges_exists("t_a", "t_a").unwrap());
    c.grant_privileges("t_a", "t_a").unwrap();
    assert!(c.grant_privileges_exists("t_a", "t_a").unwrap());

    // delete path: revoke -> delete db -> delete user
    c.revoke_privileges("t_a", "t_a").unwrap();
    assert!(!c.grant_privileges_exists("t_a", "t_a").unwrap());
    c.delete_db("t_a").unwrap();
    assert!(!c.db_exists("t_a").unwrap());
    c.delete_user("t_a").unwrap();
    assert!(!c.user_exists("t_a").unwrap());
}

#[test]
fn fake_connection_records_rendered_sql() {
    let mut c = FakeConnection::new(Driver::PostgreSql);
    c.create_db("t_a").unwrap();
    c.create_user("t_a", "pw").unwrap();
    c.grant_privileges("t_a", "t_a").unwrap();
    let log = c.statement_log();
    assert!(log.iter().any(|s| s == "CREATE DATABASE t_a"));
    assert!(log.iter().any(|s| s == "CREATE ROLE t_a LOGIN PASSWORD ?"));
    assert!(
        log.iter()
            .any(|s| s == "GRANT ALL PRIVILEGES ON DATABASE t_a TO t_a")
    );
}

#[test]
fn fake_connection_mysql_renders_mysql_dialect() {
    let mut c = FakeConnection::new(Driver::MySql);
    c.create_db("t_a").unwrap();
    assert!(
        c.statement_log()
            .iter()
            .any(|s| s == "CREATE DATABASE IF NOT EXISTS t_a")
    );
}

#[test]
fn nats_driver_rejects_sql_mutations() {
    // NATS has no programmatic user/db management — mutations are unsupported.
    let mut c = FakeConnection::new(Driver::Nats);
    assert!(matches!(
        c.create_db("t_a"),
        Err(DatastoreError::UnsupportedDriver(_))
    ));
}

// ── Kine schema migration (datastore-migrate / Connection.Migrate) ───────────

#[test]
fn kine_migration_materialises_the_kine_table_for_sql_drivers() {
    let mut c = FakeConnection::new(Driver::PostgreSql);
    let migrated = run_migrate(&mut c).unwrap();
    assert!(migrated, "SQL driver runs the Kine schema migration");
    // The full Kine DDL (CREATE TABLE + indexes) is executed verbatim, in order.
    assert_eq!(c.statement_log(), kine_table_ddl().as_slice());
    assert!(
        c.statement_log()
            .iter()
            .any(|s| s.starts_with("CREATE TABLE IF NOT EXISTS kine")),
        "the kine table is created"
    );
    assert!(
        c.statement_log()
            .iter()
            .any(|s| s.contains("kine_name_prev_revision_uindex")),
        "the unique index is created"
    );
}

#[test]
fn etcd_driver_runs_no_kine_migration() {
    let mut c = FakeConnection::new(Driver::Etcd);
    let migrated = run_migrate(&mut c).unwrap();
    assert!(!migrated, "etcd is native — no Kine schema");
    assert!(c.statement_log().is_empty());
}

#[test]
fn execute_ddl_is_rejected_on_nats() {
    let mut c = FakeConnection::new(Driver::Nats);
    assert!(matches!(
        c.execute_ddl("CREATE TABLE kine ()"),
        Err(DatastoreError::UnsupportedDriver(_))
    ));
}
