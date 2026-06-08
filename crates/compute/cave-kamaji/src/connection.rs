// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Datastore driver abstraction — the back-end Kamaji shares across tenant
//! control planes, with Kine support for the SQL drivers.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   api/v1alpha1/datastore_types.go             — `Driver` enum
//!   internal/datastore/postgresql.go            — Postgres DDL + kine table
//!   internal/datastore/mysql.go                 — MySQL DDL
//!   internal/datastore/datastore.go (Connection interface)
//!
//! Upstream drives a live `*pg.DB` / `*sql.DB`. The Cave port keeps the exact
//! DDL the controllers emit (so we can reproduce, diff, and test the
//! statements deterministically) and provides an in-memory [`FakeConnection`]
//! that the datastore-setup state machine ([`crate::ds_setup`]) drives the
//! same way upstream drives a real connection. The live TCP/TLS plumbing is
//! owned by cave-rdbms (Postgres/MySQL) and cave-etcd (shared etcd).

use std::collections::BTreeSet;

/// The shared-datastore driver — `Driver` in datastore_types.go.
///
/// `+kubebuilder:validation:Enum=etcd;MySQL;PostgreSQL;NATS`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Driver {
    Etcd,
    MySql,
    PostgreSql,
    Nats,
}

impl Driver {
    /// Wire value as serialized in the CRD (`spec.driver`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Driver::Etcd => "etcd",
            Driver::MySql => "MySQL",
            Driver::PostgreSql => "PostgreSQL",
            Driver::Nats => "NATS",
        }
    }

    /// Parse from the CRD wire value.
    pub fn from_wire(s: &str) -> Option<Driver> {
        match s {
            "etcd" => Some(Driver::Etcd),
            "MySQL" => Some(Driver::MySql),
            "PostgreSQL" => Some(Driver::PostgreSql),
            "NATS" => Some(Driver::Nats),
            _ => None,
        }
    }

    /// True for the SQL/streaming drivers that route through Kine; etcd is
    /// the native back-end and talks to the apiserver directly. Mirrors the
    /// `--etcd-servers=http://127.0.0.1:2379` (Kine sidecar) switch in
    /// builders/controlplane/deployment.go `buildKubeAPIServerCommand`.
    pub fn is_kine(&self) -> bool {
        !matches!(self, Driver::Etcd)
    }

    /// Whether a single DataStore of this driver can be shared by more than
    /// one TenantControlPlane. datastore_multitenancy.go rejects a second
    /// reclaiming TCP for NATS only.
    pub fn supports_multitenancy(&self) -> bool {
        !matches!(self, Driver::Nats)
    }
}

/// PostgreSQL DDL renderers — faithful to the `postgresql*Statement` consts
/// in internal/datastore/postgresql.go. The password in `create_user` is a
/// bound parameter upstream (`PASSWORD ?`), so it is not interpolated here.
pub mod postgres {
    pub fn create_db(db: &str) -> String {
        format!("CREATE DATABASE {db}")
    }
    pub fn create_user(user: &str) -> String {
        format!("CREATE ROLE {user} LOGIN PASSWORD ?")
    }
    pub fn grant_privileges(db: &str, user: &str) -> String {
        format!("GRANT ALL PRIVILEGES ON DATABASE {db} TO {user}")
    }
    pub fn change_owner(db: &str, user: &str) -> String {
        format!("ALTER DATABASE {db} OWNER TO {user}")
    }
    pub fn revoke_privileges(db: &str, user: &str) -> String {
        format!("REVOKE ALL PRIVILEGES ON DATABASE {db} FROM {user}")
    }
    pub fn drop_role(user: &str) -> String {
        format!("DROP ROLE {user}")
    }
    pub fn drop_db(db: &str) -> String {
        format!("DROP DATABASE {db} WITH (FORCE)")
    }
}

/// MySQL DDL renderers — faithful to the `mysql*Statement` consts in
/// internal/datastore/mysql.go. Note `grant_privileges`/`revoke_privileges`
/// replicate the upstream call-site arg order `(user, dbName)`; in Kamaji the
/// per-tenant user and schema share the same `ns_name` value so the order is
/// invisible in practice.
pub mod mysql {
    pub fn create_db(db: &str) -> String {
        format!("CREATE DATABASE IF NOT EXISTS {db}")
    }
    pub fn create_user(user: &str, password: &str) -> String {
        format!("CREATE USER `{user}`@`%` IDENTIFIED BY '{password}'")
    }
    pub fn grant_privileges(user: &str, db: &str) -> String {
        format!("GRANT ALL PRIVILEGES ON `{user}`.* TO `{db}`@`%`")
    }
    pub fn drop_db(db: &str) -> String {
        format!("DROP DATABASE IF EXISTS `{db}`")
    }
    pub fn drop_user(user: &str) -> String {
        format!("DROP USER IF EXISTS `{user}`")
    }
    pub fn revoke_privileges(db: &str, user: &str) -> String {
        format!("REVOKE ALL PRIVILEGES ON `{db}`.* FROM `{user}`")
    }
}

/// The Kine table schema + indexes a SQL datastore needs, exactly as emitted
/// by `(*PostgreSQLConnection).Migrate` in postgresql.go (CREATE TABLE +
/// TRUNCATE + four plain indexes + one unique index).
pub fn kine_table_ddl() -> Vec<String> {
    vec![
        "CREATE TABLE IF NOT EXISTS kine (\n\tid SERIAL PRIMARY KEY,\n\tname VARCHAR(630),\n\tcreated INTEGER,\n\tdeleted INTEGER,\n\tcreate_revision INTEGER,\n\tprev_revision INTEGER,\n\tlease INTEGER,\n\tvalue bytea,\n\told_value bytea\n)".to_string(),
        "TRUNCATE TABLE kine".to_string(),
        "CREATE INDEX IF NOT EXISTS kine_name_index ON kine (name)".to_string(),
        "CREATE INDEX IF NOT EXISTS kine_name_id_index ON kine (name,id)".to_string(),
        "CREATE INDEX IF NOT EXISTS kine_id_deleted_index ON kine (id,deleted)".to_string(),
        "CREATE INDEX IF NOT EXISTS kine_prev_revision_index ON kine (prev_revision)".to_string(),
        "CREATE UNIQUE INDEX IF NOT EXISTS kine_name_prev_revision_uindex ON kine (name, prev_revision)".to_string(),
    ]
}

/// Errors surfaced by a [`Connection`] — mirrors the `internal/datastore/errors`
/// taxonomy (only the variants the Cave port exercises).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DatastoreError {
    #[error("driver {0} does not support programmatic datastore management")]
    UnsupportedDriver(&'static str),
    #[error("datastore connection check failed: {0}")]
    CheckFailed(String),
}

/// The management surface the datastore-setup controller drives — a faithful
/// projection of the `datastore.Connection` interface in datastore.go.
pub trait Connection {
    fn driver(&self) -> Driver;
    fn db_exists(&self, db: &str) -> Result<bool, DatastoreError>;
    fn create_db(&mut self, db: &str) -> Result<(), DatastoreError>;
    fn delete_db(&mut self, db: &str) -> Result<(), DatastoreError>;
    fn user_exists(&self, user: &str) -> Result<bool, DatastoreError>;
    fn create_user(&mut self, user: &str, password: &str) -> Result<(), DatastoreError>;
    fn delete_user(&mut self, user: &str) -> Result<(), DatastoreError>;
    fn grant_privileges_exists(&self, user: &str, db: &str) -> Result<bool, DatastoreError>;
    fn grant_privileges(&mut self, user: &str, db: &str) -> Result<(), DatastoreError>;
    fn revoke_privileges(&mut self, user: &str, db: &str) -> Result<(), DatastoreError>;
    /// Execute a raw DDL statement against the back-end — the escape hatch the
    /// Kine schema migration drives (`CREATE TABLE kine` + its indexes).
    fn execute_ddl(&mut self, stmt: &str) -> Result<(), DatastoreError>;
}

/// `(*Connection).Migrate` — run the Kine table migration for the SQL/streaming
/// drivers that route the apiserver through a Kine sidecar. The etcd driver is
/// the native back-end and needs no schema, so it is a no-op. Returns `true`
/// when the Kine schema DDL was executed.
///
/// Upstream reference: internal/datastore/{postgresql,mysql}.go `Migrate`,
/// invoked from `getDataStoreMigratingResources` (the `datastore-migrate` step).
pub fn run_migrate(conn: &mut dyn Connection) -> Result<bool, DatastoreError> {
    if !conn.driver().is_kine() {
        return Ok(false);
    }
    for stmt in kine_table_ddl() {
        conn.execute_ddl(&stmt)?;
    }
    Ok(true)
}

/// In-memory [`Connection`] that renders and records the same DDL the real
/// drivers emit. Used as the datastore-setup state machine's backend in tests
/// and in the REST `setup-preview` surface.
#[derive(Debug, Default)]
pub struct FakeConnection {
    driver: Option<Driver>,
    databases: BTreeSet<String>,
    users: BTreeSet<String>,
    grants: BTreeSet<(String, String)>,
    statements: Vec<String>,
}

impl FakeConnection {
    pub fn new(driver: Driver) -> Self {
        Self {
            driver: Some(driver),
            ..Default::default()
        }
    }

    /// Snapshot of databases that currently exist.
    pub fn databases(&self) -> Vec<String> {
        self.databases.iter().cloned().collect()
    }

    /// The ordered list of DDL statements this connection has executed.
    pub fn statement_log(&self) -> &[String] {
        &self.statements
    }

    fn drv(&self) -> Driver {
        self.driver.unwrap_or(Driver::PostgreSql)
    }

    /// NATS has no programmatic schema/user management (datastore_multitenancy.go
    /// + storage_config.go borrow the root credentials instead).
    fn guard_sql(&self) -> Result<(), DatastoreError> {
        if matches!(self.drv(), Driver::Nats) {
            return Err(DatastoreError::UnsupportedDriver("NATS"));
        }
        Ok(())
    }
}

impl Connection for FakeConnection {
    fn driver(&self) -> Driver {
        self.drv()
    }

    fn db_exists(&self, db: &str) -> Result<bool, DatastoreError> {
        Ok(self.databases.contains(db))
    }

    fn create_db(&mut self, db: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        let stmt = match self.drv() {
            Driver::MySql => mysql::create_db(db),
            _ => postgres::create_db(db),
        };
        self.statements.push(stmt);
        self.databases.insert(db.to_string());
        Ok(())
    }

    fn delete_db(&mut self, db: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        let stmt = match self.drv() {
            Driver::MySql => mysql::drop_db(db),
            _ => postgres::drop_db(db),
        };
        self.statements.push(stmt);
        self.databases.remove(db);
        Ok(())
    }

    fn user_exists(&self, user: &str) -> Result<bool, DatastoreError> {
        Ok(self.users.contains(user))
    }

    fn create_user(&mut self, user: &str, password: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        let stmt = match self.drv() {
            Driver::MySql => mysql::create_user(user, password),
            _ => postgres::create_user(user),
        };
        self.statements.push(stmt);
        self.users.insert(user.to_string());
        Ok(())
    }

    fn delete_user(&mut self, user: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        let stmt = match self.drv() {
            Driver::MySql => mysql::drop_user(user),
            _ => postgres::drop_role(user),
        };
        self.statements.push(stmt);
        self.users.remove(user);
        Ok(())
    }

    fn grant_privileges_exists(&self, user: &str, db: &str) -> Result<bool, DatastoreError> {
        Ok(self.grants.contains(&(user.to_string(), db.to_string())))
    }

    fn grant_privileges(&mut self, user: &str, db: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        let stmt = match self.drv() {
            Driver::MySql => mysql::grant_privileges(user, db),
            _ => postgres::grant_privileges(db, user),
        };
        self.statements.push(stmt);
        self.grants.insert((user.to_string(), db.to_string()));
        Ok(())
    }

    fn revoke_privileges(&mut self, user: &str, db: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        let stmt = match self.drv() {
            Driver::MySql => mysql::revoke_privileges(db, user),
            _ => postgres::revoke_privileges(db, user),
        };
        self.statements.push(stmt);
        self.grants.remove(&(user.to_string(), db.to_string()));
        Ok(())
    }

    fn execute_ddl(&mut self, stmt: &str) -> Result<(), DatastoreError> {
        self.guard_sql()?;
        self.statements.push(stmt.to_string());
        Ok(())
    }
}
