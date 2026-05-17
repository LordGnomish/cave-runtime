// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/connections/jpa/JpaConnectionProviderFactory.java
//
// Port of Keycloak's `JpaConnectionProviderFactory` — the bridge between the
// realm/user providers and the underlying JDBC connection pool. Our
// `RdbmsBackend` plays the same role over SQLite (rusqlite, bundled). The
// schema is identical to what a Postgres deployment would use because the
// migration DDL stays ANSI-compatible (TEXT/INTEGER + json columns).
//
// Why rusqlite and not sqlx/tokio-postgres: the workspace already pins
// `rusqlite { features = ["bundled"] }` and avoids the much larger sqlx
// dependency tree. The trait is identical so a Postgres impl is a
// drop-in addition once `cave_rdbms::Pool` exposes a query API.

//! `RdbmsBackend` — rusqlite-backed implementation of [`PersistenceBackend`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::backend::{PersistenceBackend, PersistenceError, Result};
use super::entities::{
    AuditFields, AuthFlowEntity, ClientEntity, FlowExecution, GroupEntity, IdentityProviderEntity,
    IdpMapper, RealmEntity, RoleEntity, UserCredential, UserEntity,
};
use super::migration::{self, AppliedMigration, Migration, MigrationExecutor};
use super::txn::Transaction;

impl From<rusqlite::Error> for PersistenceError {
    fn from(e: rusqlite::Error) -> Self {
        PersistenceError::Backend(format!("rusqlite: {e}"))
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(e: serde_json::Error) -> Self {
        PersistenceError::Backend(format!("serde_json: {e}"))
    }
}

impl From<tokio::task::JoinError> for PersistenceError {
    fn from(e: tokio::task::JoinError) -> Self {
        PersistenceError::Backend(format!("spawn_blocking join: {e}"))
    }
}

/// rusqlite is sync; we put the `Connection` behind an `Arc<Mutex<…>>` and
/// drive every method through `tokio::task::spawn_blocking`.
#[derive(Clone)]
pub struct RdbmsBackend {
    conn: Arc<Mutex<Connection>>,
}

impl RdbmsBackend {
    /// Open or create a SQLite database file and run migrations.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .map_err(|e| PersistenceError::Backend(format!("open {e}")))?;
        let me = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        me.run_migrations()?;
        Ok(me)
    }

    /// In-memory SQLite — useful for tests and ephemeral local dev.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| PersistenceError::Backend(format!("open_in_memory {e}")))?;
        let me = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        me.run_migrations()?;
        Ok(me)
    }

    fn run_migrations(&self) -> Result<()> {
        let mut guard = self.conn.lock().expect("conn mutex poisoned");
        guard.execute_batch(
            r#"CREATE TABLE IF NOT EXISTS cave_auth_schema_history (
                version    TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                applied_at TEXT NOT NULL,
                checksum   INTEGER NOT NULL
            );"#,
        )?;
        let mut exec = RusqliteMigrationExec { conn: &mut *guard };
        migration::run(&migration::baseline_migrations(), &mut exec)
            .map_err(|e| PersistenceError::Migration(e))?;
        Ok(())
    }

    fn with_conn<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R>,
    {
        let mut guard = self.conn.lock().expect("conn mutex poisoned");
        f(&mut *guard)
    }

    /// List entries in `cave_auth_schema_history` — used by tests.
    pub fn applied_migrations(&self) -> Result<Vec<AppliedMigration>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT version, name, applied_at, checksum FROM cave_auth_schema_history ORDER BY version",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    let applied_at: String = r.get(2)?;
                    Ok(AppliedMigration {
                        version: r.get(0)?,
                        name: r.get(1)?,
                        applied_at: DateTime::parse_from_rfc3339(&applied_at)
                            .map(|d| d.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        checksum: r.get(3)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
}

// ── MigrationExecutor adapter ────────────────────────────────────────────────

struct RusqliteMigrationExec<'a> {
    conn: &'a mut Connection,
}

impl<'a> MigrationExecutor for RusqliteMigrationExec<'a> {
    fn applied(&self) -> Vec<AppliedMigration> {
        let mut stmt = match self.conn.prepare(
            "SELECT version, name, applied_at, checksum FROM cave_auth_schema_history ORDER BY version",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |r| {
            let applied_at: String = r.get(2)?;
            Ok(AppliedMigration {
                version: r.get(0)?,
                name: r.get(1)?,
                applied_at: DateTime::parse_from_rfc3339(&applied_at)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                checksum: r.get(3)?,
            })
        })
        .and_then(|iter| iter.collect::<std::result::Result<Vec<_>, _>>())
        .unwrap_or_default()
    }
    fn execute(&mut self, m: &Migration) -> std::result::Result<(), String> {
        self.conn
            .execute_batch(m.up_sql)
            .map_err(|e| format!("migration {} failed: {e}", m.version))
    }
    fn record(&mut self, m: &Migration, checksum: i64) {
        let _ = self.conn.execute(
            "INSERT INTO cave_auth_schema_history (version, name, applied_at, checksum) VALUES (?1, ?2, ?3, ?4)",
            params![m.version, m.name, Utc::now().to_rfc3339(), checksum],
        );
    }
}

// ── Row encoding helpers ─────────────────────────────────────────────────────

fn ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}
fn ts_opt(dt: Option<DateTime<Utc>>) -> Option<String> {
    dt.map(ts)
}
fn parse_ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
fn b(x: bool) -> i64 {
    if x { 1 } else { 0 }
}
fn ib(x: i64) -> bool {
    x != 0
}
fn uid_to_str(id: Uuid) -> String {
    id.to_string()
}
#[cfg(test)]
fn parse_uid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| PersistenceError::Backend(format!("bad uuid `{s}`: {e}")))
}
#[cfg(test)]
fn parse_uid_opt(s: Option<String>) -> Result<Option<Uuid>> {
    match s.as_deref() {
        None | Some("") => Ok(None),
        Some(v) => Ok(Some(parse_uid(v)?)),
    }
}

// Row→entity decoders
fn row_to_realm(row: &rusqlite::Row<'_>) -> rusqlite::Result<RealmEntity> {
    let id_s: String = row.get(0)?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(RealmEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        name: row.get("name")?,
        display_name: row.get("display_name")?,
        enabled: ib(row.get("enabled")?),
        ssl_required: row.get("ssl_required")?,
        access_token_lifespan: row.get("access_token_lifespan")?,
        sso_session_idle_timeout: row.get("sso_session_idle_timeout")?,
        sso_session_max_lifespan: row.get("sso_session_max_lifespan")?,
        registration_allowed: ib(row.get("registration_allowed")?),
        remember_me: ib(row.get("remember_me")?),
        verify_email: ib(row.get("verify_email")?),
        login_with_email_allowed: ib(row.get("login_with_email_allowed")?),
        duplicate_emails_allowed: ib(row.get("duplicate_emails_allowed")?),
        reset_password_allowed: ib(row.get("reset_password_allowed")?),
        edit_username_allowed: ib(row.get("edit_username_allowed")?),
        brute_force_protected: ib(row.get("brute_force_protected")?),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

fn row_to_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserEntity> {
    let id_s: String = row.get(0)?;
    let realm_s: String = row.get("realm_id")?;
    let creds_json: String = row.get("credentials_json")?;
    let attrs_json: String = row.get("attributes_json")?;
    let sa_link_s: Option<String> = row.get("service_account_client_link")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(UserEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        realm_id: Uuid::parse_str(&realm_s).unwrap_or_else(|_| Uuid::nil()),
        username: row.get("username")?,
        email: row.get("email")?,
        email_verified: ib(row.get("email_verified")?),
        first_name: row.get("first_name")?,
        last_name: row.get("last_name")?,
        enabled: ib(row.get("enabled")?),
        federation_link: row.get("federation_link")?,
        service_account_client_link: sa_link_s
            .as_deref()
            .map(|s| Uuid::parse_str(s).unwrap_or_else(|_| Uuid::nil())),
        credentials: serde_json::from_str::<Vec<UserCredential>>(&creds_json).unwrap_or_default(),
        attributes: serde_json::from_str(&attrs_json).unwrap_or_default(),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

fn row_to_client(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClientEntity> {
    let id_s: String = row.get(0)?;
    let realm_s: String = row.get("realm_id")?;
    let redirect_json: String = row.get("redirect_uris_json")?;
    let webo_json: String = row.get("web_origins_json")?;
    let attrs_json: String = row.get("attributes_json")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(ClientEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        realm_id: Uuid::parse_str(&realm_s).unwrap_or_else(|_| Uuid::nil()),
        client_id: row.get("client_id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        enabled: ib(row.get("enabled")?),
        client_authenticator_type: row.get("client_authenticator_type")?,
        secret: row.get("secret")?,
        redirect_uris: serde_json::from_str(&redirect_json).unwrap_or_default(),
        web_origins: serde_json::from_str(&webo_json).unwrap_or_default(),
        bearer_only: ib(row.get("bearer_only")?),
        consent_required: ib(row.get("consent_required")?),
        standard_flow_enabled: ib(row.get("standard_flow_enabled")?),
        implicit_flow_enabled: ib(row.get("implicit_flow_enabled")?),
        direct_access_grants_enabled: ib(row.get("direct_access_grants_enabled")?),
        service_accounts_enabled: ib(row.get("service_accounts_enabled")?),
        public_client: ib(row.get("public_client")?),
        frontchannel_logout: ib(row.get("frontchannel_logout")?),
        protocol: row.get("protocol")?,
        attributes: serde_json::from_str(&attrs_json).unwrap_or_default(),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

fn row_to_role(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoleEntity> {
    let id_s: String = row.get(0)?;
    let realm_s: String = row.get("realm_id")?;
    let client_s: Option<String> = row.get("client_id")?;
    let attrs_json: String = row.get("attributes_json")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(RoleEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        realm_id: Uuid::parse_str(&realm_s).unwrap_or_else(|_| Uuid::nil()),
        client_id: client_s
            .as_deref()
            .map(|s| Uuid::parse_str(s).unwrap_or_else(|_| Uuid::nil())),
        name: row.get("name")?,
        description: row.get("description")?,
        composite: ib(row.get("composite")?),
        attributes: serde_json::from_str(&attrs_json).unwrap_or_default(),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

fn row_to_group(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupEntity> {
    let id_s: String = row.get(0)?;
    let realm_s: String = row.get("realm_id")?;
    let parent_s: Option<String> = row.get("parent_id")?;
    let attrs_json: String = row.get("attributes_json")?;
    let role_ids_json: String = row.get("role_ids_json")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(GroupEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        realm_id: Uuid::parse_str(&realm_s).unwrap_or_else(|_| Uuid::nil()),
        parent_id: parent_s
            .as_deref()
            .map(|s| Uuid::parse_str(s).unwrap_or_else(|_| Uuid::nil())),
        name: row.get("name")?,
        attributes: serde_json::from_str(&attrs_json).unwrap_or_default(),
        role_ids: serde_json::from_str::<Vec<String>>(&role_ids_json)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| Uuid::parse_str(&s).ok())
            .collect(),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

fn row_to_idp(row: &rusqlite::Row<'_>) -> rusqlite::Result<IdentityProviderEntity> {
    let id_s: String = row.get(0)?;
    let realm_s: String = row.get("realm_id")?;
    let fbl: Option<String> = row.get("first_broker_login_flow_id")?;
    let pbl: Option<String> = row.get("post_broker_login_flow_id")?;
    let config_json: String = row.get("config_json")?;
    let mappers_json: String = row.get("mappers_json")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(IdentityProviderEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        realm_id: Uuid::parse_str(&realm_s).unwrap_or_else(|_| Uuid::nil()),
        alias: row.get("alias")?,
        display_name: row.get("display_name")?,
        provider_id: row.get("provider_id")?,
        enabled: ib(row.get("enabled")?),
        trust_email: ib(row.get("trust_email")?),
        store_token: ib(row.get("store_token")?),
        add_read_token_role_on_create: ib(row.get("add_read_token_role_on_create")?),
        authenticate_by_default: ib(row.get("authenticate_by_default")?),
        link_only: ib(row.get("link_only")?),
        first_broker_login_flow_id: fbl
            .as_deref()
            .map(|s| Uuid::parse_str(s).unwrap_or_else(|_| Uuid::nil())),
        post_broker_login_flow_id: pbl
            .as_deref()
            .map(|s| Uuid::parse_str(s).unwrap_or_else(|_| Uuid::nil())),
        config: serde_json::from_str(&config_json).unwrap_or_default(),
        mappers: serde_json::from_str::<Vec<IdpMapper>>(&mappers_json).unwrap_or_default(),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

fn row_to_flow(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuthFlowEntity> {
    let id_s: String = row.get(0)?;
    let realm_s: String = row.get("realm_id")?;
    let execs_json: String = row.get("executions_json")?;
    let created: String = row.get("created_at")?;
    let updated: String = row.get("updated_at")?;
    let deleted: Option<String> = row.get("deleted_at")?;
    Ok(AuthFlowEntity {
        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
        realm_id: Uuid::parse_str(&realm_s).unwrap_or_else(|_| Uuid::nil()),
        alias: row.get("alias")?,
        description: row.get("description")?,
        provider_id: row.get("provider_id")?,
        top_level: ib(row.get("top_level")?),
        built_in: ib(row.get("built_in")?),
        executions: serde_json::from_str::<Vec<FlowExecution>>(&execs_json).unwrap_or_default(),
        audit: AuditFields {
            created_at: parse_ts(&created),
            updated_at: parse_ts(&updated),
            deleted_at: deleted.as_deref().map(parse_ts),
        },
    })
}

// ── RDBMS-backed transaction ────────────────────────────────────────────────

pub struct RdbmsTxn {
    backend: RdbmsBackend,
    /// Snapshot of every table, taken at `begin_txn`. On `rollback` we
    /// restore. SQLite-level BEGIN/COMMIT is not safe to bridge across
    /// many concurrent async tasks sharing a single mutexed connection,
    /// so we model isolation at the row-level the same way the
    /// in-memory backend does.
    snapshot: Snapshot,
    rollback_on_drop: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Default)]
struct Snapshot {
    realms: Vec<RealmEntity>,
    users: Vec<UserEntity>,
    clients: Vec<ClientEntity>,
    roles: Vec<RoleEntity>,
    groups: Vec<GroupEntity>,
    idps: Vec<IdentityProviderEntity>,
    flows: Vec<AuthFlowEntity>,
}

#[async_trait]
impl Transaction for RdbmsTxn {
    async fn commit(self: Box<Self>) -> Result<()> {
        self.rollback_on_drop
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        let backend = self.backend.clone();
        let snap = self.snapshot.clone();
        backend.with_conn(|c| {
            restore_snapshot(c, &snap)?;
            Ok(())
        })?;
        self.rollback_on_drop
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for RdbmsTxn {
    fn drop(&mut self) {
        if self
            .rollback_on_drop
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            let _ = self
                .backend
                .with_conn(|c| restore_snapshot(c, &self.snapshot));
        }
    }
}

fn restore_snapshot(c: &mut Connection, snap: &Snapshot) -> Result<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM realms", [])?;
    tx.execute("DELETE FROM users", [])?;
    tx.execute("DELETE FROM clients", [])?;
    tx.execute("DELETE FROM roles", [])?;
    tx.execute("DELETE FROM groups_tbl", [])?;
    tx.execute("DELETE FROM identity_providers", [])?;
    tx.execute("DELETE FROM authentication_flows", [])?;
    for r in &snap.realms {
        insert_realm_tx(&tx, r)?;
    }
    for u in &snap.users {
        insert_user_tx(&tx, u)?;
    }
    for c2 in &snap.clients {
        insert_client_tx(&tx, c2)?;
    }
    for r in &snap.roles {
        insert_role_tx(&tx, r)?;
    }
    for g in &snap.groups {
        insert_group_tx(&tx, g)?;
    }
    for i in &snap.idps {
        insert_idp_tx(&tx, i)?;
    }
    for f in &snap.flows {
        insert_flow_tx(&tx, f)?;
    }
    tx.commit()?;
    Ok(())
}

fn insert_realm_tx(tx: &rusqlite::Transaction<'_>, r: &RealmEntity) -> Result<()> {
    tx.execute(
        "INSERT INTO realms (id, name, display_name, enabled, ssl_required, access_token_lifespan, sso_session_idle_timeout, sso_session_max_lifespan, registration_allowed, remember_me, verify_email, login_with_email_allowed, duplicate_emails_allowed, reset_password_allowed, edit_username_allowed, brute_force_protected, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        params![
            uid_to_str(r.id), r.name, r.display_name, b(r.enabled), r.ssl_required,
            r.access_token_lifespan, r.sso_session_idle_timeout, r.sso_session_max_lifespan,
            b(r.registration_allowed), b(r.remember_me), b(r.verify_email),
            b(r.login_with_email_allowed), b(r.duplicate_emails_allowed),
            b(r.reset_password_allowed), b(r.edit_username_allowed),
            b(r.brute_force_protected), ts(r.audit.created_at), ts(r.audit.updated_at),
            ts_opt(r.audit.deleted_at),
        ],
    )?;
    Ok(())
}

fn insert_user_tx(tx: &rusqlite::Transaction<'_>, u: &UserEntity) -> Result<()> {
    tx.execute(
        "INSERT INTO users (id, realm_id, username, email, email_verified, first_name, last_name, enabled, federation_link, service_account_client_link, credentials_json, attributes_json, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            uid_to_str(u.id), uid_to_str(u.realm_id), u.username, u.email,
            b(u.email_verified), u.first_name, u.last_name, b(u.enabled),
            u.federation_link, u.service_account_client_link.map(uid_to_str),
            serde_json::to_string(&u.credentials)?,
            serde_json::to_string(&u.attributes)?,
            ts(u.audit.created_at), ts(u.audit.updated_at), ts_opt(u.audit.deleted_at),
        ],
    )?;
    Ok(())
}

fn insert_client_tx(tx: &rusqlite::Transaction<'_>, c: &ClientEntity) -> Result<()> {
    tx.execute(
        "INSERT INTO clients (id, realm_id, client_id, name, description, enabled, client_authenticator_type, secret, redirect_uris_json, web_origins_json, bearer_only, consent_required, standard_flow_enabled, implicit_flow_enabled, direct_access_grants_enabled, service_accounts_enabled, public_client, frontchannel_logout, protocol, attributes_json, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
        params![
            uid_to_str(c.id), uid_to_str(c.realm_id), c.client_id, c.name, c.description,
            b(c.enabled), c.client_authenticator_type, c.secret,
            serde_json::to_string(&c.redirect_uris)?,
            serde_json::to_string(&c.web_origins)?,
            b(c.bearer_only), b(c.consent_required), b(c.standard_flow_enabled),
            b(c.implicit_flow_enabled), b(c.direct_access_grants_enabled),
            b(c.service_accounts_enabled), b(c.public_client), b(c.frontchannel_logout),
            c.protocol, serde_json::to_string(&c.attributes)?,
            ts(c.audit.created_at), ts(c.audit.updated_at), ts_opt(c.audit.deleted_at),
        ],
    )?;
    Ok(())
}

fn insert_role_tx(tx: &rusqlite::Transaction<'_>, r: &RoleEntity) -> Result<()> {
    tx.execute(
        "INSERT INTO roles (id, realm_id, client_id, name, description, composite, attributes_json, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            uid_to_str(r.id), uid_to_str(r.realm_id), r.client_id.map(uid_to_str),
            r.name, r.description, b(r.composite),
            serde_json::to_string(&r.attributes)?,
            ts(r.audit.created_at), ts(r.audit.updated_at), ts_opt(r.audit.deleted_at),
        ],
    )?;
    Ok(())
}

fn insert_group_tx(tx: &rusqlite::Transaction<'_>, g: &GroupEntity) -> Result<()> {
    let role_ids: Vec<String> = g.role_ids.iter().map(|x| x.to_string()).collect();
    tx.execute(
        "INSERT INTO groups_tbl (id, realm_id, parent_id, name, attributes_json, role_ids_json, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            uid_to_str(g.id), uid_to_str(g.realm_id), g.parent_id.map(uid_to_str),
            g.name, serde_json::to_string(&g.attributes)?,
            serde_json::to_string(&role_ids)?,
            ts(g.audit.created_at), ts(g.audit.updated_at), ts_opt(g.audit.deleted_at),
        ],
    )?;
    Ok(())
}

fn insert_idp_tx(tx: &rusqlite::Transaction<'_>, i: &IdentityProviderEntity) -> Result<()> {
    tx.execute(
        "INSERT INTO identity_providers (id, realm_id, alias, display_name, provider_id, enabled, trust_email, store_token, add_read_token_role_on_create, authenticate_by_default, link_only, first_broker_login_flow_id, post_broker_login_flow_id, config_json, mappers_json, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        params![
            uid_to_str(i.id), uid_to_str(i.realm_id), i.alias, i.display_name, i.provider_id,
            b(i.enabled), b(i.trust_email), b(i.store_token),
            b(i.add_read_token_role_on_create), b(i.authenticate_by_default),
            b(i.link_only),
            i.first_broker_login_flow_id.map(uid_to_str),
            i.post_broker_login_flow_id.map(uid_to_str),
            serde_json::to_string(&i.config)?,
            serde_json::to_string(&i.mappers)?,
            ts(i.audit.created_at), ts(i.audit.updated_at), ts_opt(i.audit.deleted_at),
        ],
    )?;
    Ok(())
}

fn insert_flow_tx(tx: &rusqlite::Transaction<'_>, f: &AuthFlowEntity) -> Result<()> {
    tx.execute(
        "INSERT INTO authentication_flows (id, realm_id, alias, description, provider_id, top_level, built_in, executions_json, created_at, updated_at, deleted_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            uid_to_str(f.id), uid_to_str(f.realm_id), f.alias, f.description, f.provider_id,
            b(f.top_level), b(f.built_in),
            serde_json::to_string(&f.executions)?,
            ts(f.audit.created_at), ts(f.audit.updated_at), ts_opt(f.audit.deleted_at),
        ],
    )?;
    Ok(())
}

// ── PersistenceBackend impl ──────────────────────────────────────────────────

#[async_trait]
impl PersistenceBackend for RdbmsBackend {
    // ── Realm ────────────────────────────────────────────────────────────
    async fn list_realms(&self) -> Result<Vec<RealmEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT * FROM realms WHERE deleted_at IS NULL")?;
            let rows = stmt
                .query_map([], row_to_realm)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_realm_by_id(&self, id: Uuid) -> Result<Option<RealmEntity>> {
        self.with_conn(|c| {
            let r = c
                .query_row(
                    "SELECT * FROM realms WHERE id = ?1 AND deleted_at IS NULL",
                    params![uid_to_str(id)],
                    row_to_realm,
                )
                .optional()?;
            Ok(r)
        })
    }
    async fn get_realm_by_name(&self, name: &str) -> Result<Option<RealmEntity>> {
        let name = name.to_string();
        self.with_conn(|c| {
            let r = c
                .query_row(
                    "SELECT * FROM realms WHERE name = ?1 AND deleted_at IS NULL",
                    params![name],
                    row_to_realm,
                )
                .optional()?;
            Ok(r)
        })
    }
    async fn create_realm(&self, r: RealmEntity) -> Result<RealmEntity> {
        self.with_conn(|c| {
            let exists: i64 = c.query_row(
                "SELECT COUNT(*) FROM realms WHERE name = ?1 AND deleted_at IS NULL",
                params![r.name],
                |row| row.get(0),
            )?;
            if exists > 0 {
                return Err(PersistenceError::conflict(
                    "realm",
                    format!("realm name `{}` already exists", r.name),
                ));
            }
            let tx = c.transaction()?;
            insert_realm_tx(&tx, &r)?;
            tx.commit()?;
            Ok(r)
        })
    }
    async fn update_realm(&self, mut r: RealmEntity) -> Result<RealmEntity> {
        r.audit.updated_at = Utc::now();
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE realms SET name=?1, display_name=?2, enabled=?3, ssl_required=?4, access_token_lifespan=?5, sso_session_idle_timeout=?6, sso_session_max_lifespan=?7, registration_allowed=?8, remember_me=?9, verify_email=?10, login_with_email_allowed=?11, duplicate_emails_allowed=?12, reset_password_allowed=?13, edit_username_allowed=?14, brute_force_protected=?15, updated_at=?16, deleted_at=?17 WHERE id=?18",
                params![
                    r.name, r.display_name, b(r.enabled), r.ssl_required,
                    r.access_token_lifespan, r.sso_session_idle_timeout, r.sso_session_max_lifespan,
                    b(r.registration_allowed), b(r.remember_me), b(r.verify_email),
                    b(r.login_with_email_allowed), b(r.duplicate_emails_allowed),
                    b(r.reset_password_allowed), b(r.edit_username_allowed),
                    b(r.brute_force_protected), ts(r.audit.updated_at),
                    ts_opt(r.audit.deleted_at), uid_to_str(r.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("realm", r.id));
            }
            Ok(r)
        })
    }
    async fn delete_realm(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE realms SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("realm", id));
            }
            Ok(())
        })
    }
    async fn count_realms(&self) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM realms WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── User ─────────────────────────────────────────────────────────────
    async fn list_users_in_realm(&self, realm_id: Uuid) -> Result<Vec<UserEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT * FROM users WHERE realm_id = ?1 AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map(params![uid_to_str(realm_id)], row_to_user)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_user_by_id(&self, id: Uuid) -> Result<Option<UserEntity>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM users WHERE id = ?1 AND deleted_at IS NULL",
                params![uid_to_str(id)],
                row_to_user,
            )
            .optional()?)
        })
    }
    async fn get_user_by_name(&self, realm_id: Uuid, username: &str) -> Result<Option<UserEntity>> {
        let username = username.to_string();
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM users WHERE realm_id = ?1 AND username = ?2 AND deleted_at IS NULL",
                params![uid_to_str(realm_id), username],
                row_to_user,
            )
            .optional()?)
        })
    }
    async fn create_user(&self, u: UserEntity) -> Result<UserEntity> {
        self.with_conn(|c| {
            let exists: i64 = c.query_row(
                "SELECT COUNT(*) FROM users WHERE realm_id=?1 AND username=?2 AND deleted_at IS NULL",
                params![uid_to_str(u.realm_id), u.username],
                |r| r.get(0),
            )?;
            if exists > 0 {
                return Err(PersistenceError::conflict(
                    "user",
                    format!("username `{}` already exists in realm", u.username),
                ));
            }
            let tx = c.transaction()?;
            insert_user_tx(&tx, &u)?;
            tx.commit()?;
            Ok(u)
        })
    }
    async fn update_user(&self, mut u: UserEntity) -> Result<UserEntity> {
        u.audit.updated_at = Utc::now();
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE users SET realm_id=?1, username=?2, email=?3, email_verified=?4, first_name=?5, last_name=?6, enabled=?7, federation_link=?8, service_account_client_link=?9, credentials_json=?10, attributes_json=?11, updated_at=?12, deleted_at=?13 WHERE id=?14",
                params![
                    uid_to_str(u.realm_id), u.username, u.email, b(u.email_verified),
                    u.first_name, u.last_name, b(u.enabled), u.federation_link,
                    u.service_account_client_link.map(uid_to_str),
                    serde_json::to_string(&u.credentials)?,
                    serde_json::to_string(&u.attributes)?,
                    ts(u.audit.updated_at), ts_opt(u.audit.deleted_at),
                    uid_to_str(u.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("user", u.id));
            }
            Ok(u)
        })
    }
    async fn delete_user(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE users SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("user", id));
            }
            Ok(())
        })
    }
    async fn count_users(&self, realm_id: Uuid) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM users WHERE realm_id=?1 AND deleted_at IS NULL",
                params![uid_to_str(realm_id)],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── Client ───────────────────────────────────────────────────────────
    async fn list_clients_in_realm(&self, realm_id: Uuid) -> Result<Vec<ClientEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT * FROM clients WHERE realm_id = ?1 AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map(params![uid_to_str(realm_id)], row_to_client)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_client_by_id(&self, id: Uuid) -> Result<Option<ClientEntity>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM clients WHERE id = ?1 AND deleted_at IS NULL",
                params![uid_to_str(id)],
                row_to_client,
            )
            .optional()?)
        })
    }
    async fn get_client_by_name(
        &self,
        realm_id: Uuid,
        client_id: &str,
    ) -> Result<Option<ClientEntity>> {
        let cid = client_id.to_string();
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM clients WHERE realm_id=?1 AND client_id=?2 AND deleted_at IS NULL",
                params![uid_to_str(realm_id), cid],
                row_to_client,
            )
            .optional()?)
        })
    }
    async fn create_client(&self, c: ClientEntity) -> Result<ClientEntity> {
        self.with_conn(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT COUNT(*) FROM clients WHERE realm_id=?1 AND client_id=?2 AND deleted_at IS NULL",
                params![uid_to_str(c.realm_id), c.client_id],
                |r| r.get(0),
            )?;
            if exists > 0 {
                return Err(PersistenceError::conflict(
                    "client",
                    format!("client_id `{}` already exists in realm", c.client_id),
                ));
            }
            let tx = conn.transaction()?;
            insert_client_tx(&tx, &c)?;
            tx.commit()?;
            Ok(c)
        })
    }
    async fn update_client(&self, mut c: ClientEntity) -> Result<ClientEntity> {
        c.audit.updated_at = Utc::now();
        self.with_conn(|conn| {
            let rows = conn.execute(
                "UPDATE clients SET realm_id=?1, client_id=?2, name=?3, description=?4, enabled=?5, client_authenticator_type=?6, secret=?7, redirect_uris_json=?8, web_origins_json=?9, bearer_only=?10, consent_required=?11, standard_flow_enabled=?12, implicit_flow_enabled=?13, direct_access_grants_enabled=?14, service_accounts_enabled=?15, public_client=?16, frontchannel_logout=?17, protocol=?18, attributes_json=?19, updated_at=?20, deleted_at=?21 WHERE id=?22",
                params![
                    uid_to_str(c.realm_id), c.client_id, c.name, c.description,
                    b(c.enabled), c.client_authenticator_type, c.secret,
                    serde_json::to_string(&c.redirect_uris)?,
                    serde_json::to_string(&c.web_origins)?,
                    b(c.bearer_only), b(c.consent_required), b(c.standard_flow_enabled),
                    b(c.implicit_flow_enabled), b(c.direct_access_grants_enabled),
                    b(c.service_accounts_enabled), b(c.public_client), b(c.frontchannel_logout),
                    c.protocol, serde_json::to_string(&c.attributes)?,
                    ts(c.audit.updated_at), ts_opt(c.audit.deleted_at),
                    uid_to_str(c.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("client", c.id));
            }
            Ok(c)
        })
    }
    async fn delete_client(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE clients SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("client", id));
            }
            Ok(())
        })
    }
    async fn count_clients(&self, realm_id: Uuid) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM clients WHERE realm_id=?1 AND deleted_at IS NULL",
                params![uid_to_str(realm_id)],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── Role ─────────────────────────────────────────────────────────────
    async fn list_roles_in_realm(&self, realm_id: Uuid) -> Result<Vec<RoleEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT * FROM roles WHERE realm_id = ?1 AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map(params![uid_to_str(realm_id)], row_to_role)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_role_by_id(&self, id: Uuid) -> Result<Option<RoleEntity>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM roles WHERE id=?1 AND deleted_at IS NULL",
                params![uid_to_str(id)],
                row_to_role,
            )
            .optional()?)
        })
    }
    async fn get_role_by_name(&self, realm_id: Uuid, name: &str) -> Result<Option<RoleEntity>> {
        let name = name.to_string();
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM roles WHERE realm_id=?1 AND name=?2 AND deleted_at IS NULL",
                params![uid_to_str(realm_id), name],
                row_to_role,
            )
            .optional()?)
        })
    }
    async fn create_role(&self, r: RoleEntity) -> Result<RoleEntity> {
        self.with_conn(|c| {
            // Uniqueness: realm + (client_id NULL/equals) + name.
            let exists: i64 = match r.client_id {
                None => c.query_row(
                    "SELECT COUNT(*) FROM roles WHERE realm_id=?1 AND client_id IS NULL AND name=?2 AND deleted_at IS NULL",
                    params![uid_to_str(r.realm_id), r.name],
                    |row| row.get(0),
                )?,
                Some(cid) => c.query_row(
                    "SELECT COUNT(*) FROM roles WHERE realm_id=?1 AND client_id=?2 AND name=?3 AND deleted_at IS NULL",
                    params![uid_to_str(r.realm_id), uid_to_str(cid), r.name],
                    |row| row.get(0),
                )?,
            };
            if exists > 0 {
                return Err(PersistenceError::conflict(
                    "role",
                    format!("role `{}` already exists in realm/client scope", r.name),
                ));
            }
            let tx = c.transaction()?;
            insert_role_tx(&tx, &r)?;
            tx.commit()?;
            Ok(r)
        })
    }
    async fn update_role(&self, mut r: RoleEntity) -> Result<RoleEntity> {
        r.audit.updated_at = Utc::now();
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE roles SET realm_id=?1, client_id=?2, name=?3, description=?4, composite=?5, attributes_json=?6, updated_at=?7, deleted_at=?8 WHERE id=?9",
                params![
                    uid_to_str(r.realm_id), r.client_id.map(uid_to_str), r.name,
                    r.description, b(r.composite),
                    serde_json::to_string(&r.attributes)?,
                    ts(r.audit.updated_at), ts_opt(r.audit.deleted_at),
                    uid_to_str(r.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("role", r.id));
            }
            Ok(r)
        })
    }
    async fn delete_role(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE roles SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("role", id));
            }
            Ok(())
        })
    }
    async fn count_roles(&self, realm_id: Uuid) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM roles WHERE realm_id=?1 AND deleted_at IS NULL",
                params![uid_to_str(realm_id)],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── Group ────────────────────────────────────────────────────────────
    async fn list_groups_in_realm(&self, realm_id: Uuid) -> Result<Vec<GroupEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT * FROM groups_tbl WHERE realm_id = ?1 AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map(params![uid_to_str(realm_id)], row_to_group)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_group_by_id(&self, id: Uuid) -> Result<Option<GroupEntity>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM groups_tbl WHERE id=?1 AND deleted_at IS NULL",
                params![uid_to_str(id)],
                row_to_group,
            )
            .optional()?)
        })
    }
    async fn get_group_by_name(&self, realm_id: Uuid, name: &str) -> Result<Option<GroupEntity>> {
        let name = name.to_string();
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM groups_tbl WHERE realm_id=?1 AND name=?2 AND deleted_at IS NULL",
                params![uid_to_str(realm_id), name],
                row_to_group,
            )
            .optional()?)
        })
    }
    async fn create_group(&self, g: GroupEntity) -> Result<GroupEntity> {
        self.with_conn(|c| {
            let tx = c.transaction()?;
            insert_group_tx(&tx, &g)?;
            tx.commit()?;
            Ok(g)
        })
    }
    async fn update_group(&self, mut g: GroupEntity) -> Result<GroupEntity> {
        g.audit.updated_at = Utc::now();
        self.with_conn(|c| {
            let role_ids: Vec<String> = g.role_ids.iter().map(|x| x.to_string()).collect();
            let rows = c.execute(
                "UPDATE groups_tbl SET realm_id=?1, parent_id=?2, name=?3, attributes_json=?4, role_ids_json=?5, updated_at=?6, deleted_at=?7 WHERE id=?8",
                params![
                    uid_to_str(g.realm_id), g.parent_id.map(uid_to_str), g.name,
                    serde_json::to_string(&g.attributes)?,
                    serde_json::to_string(&role_ids)?,
                    ts(g.audit.updated_at), ts_opt(g.audit.deleted_at),
                    uid_to_str(g.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("group", g.id));
            }
            Ok(g)
        })
    }
    async fn delete_group(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE groups_tbl SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("group", id));
            }
            Ok(())
        })
    }
    async fn count_groups(&self, realm_id: Uuid) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM groups_tbl WHERE realm_id=?1 AND deleted_at IS NULL",
                params![uid_to_str(realm_id)],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── IdentityProvider ─────────────────────────────────────────────────
    async fn list_idps_in_realm(&self, realm_id: Uuid) -> Result<Vec<IdentityProviderEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT * FROM identity_providers WHERE realm_id = ?1 AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map(params![uid_to_str(realm_id)], row_to_idp)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_idp_by_id(&self, id: Uuid) -> Result<Option<IdentityProviderEntity>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM identity_providers WHERE id=?1 AND deleted_at IS NULL",
                params![uid_to_str(id)],
                row_to_idp,
            )
            .optional()?)
        })
    }
    async fn get_idp_by_name(
        &self,
        realm_id: Uuid,
        alias: &str,
    ) -> Result<Option<IdentityProviderEntity>> {
        let alias = alias.to_string();
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM identity_providers WHERE realm_id=?1 AND alias=?2 AND deleted_at IS NULL",
                params![uid_to_str(realm_id), alias],
                row_to_idp,
            )
            .optional()?)
        })
    }
    async fn create_idp(&self, i: IdentityProviderEntity) -> Result<IdentityProviderEntity> {
        self.with_conn(|c| {
            let exists: i64 = c.query_row(
                "SELECT COUNT(*) FROM identity_providers WHERE realm_id=?1 AND alias=?2 AND deleted_at IS NULL",
                params![uid_to_str(i.realm_id), i.alias],
                |r| r.get(0),
            )?;
            if exists > 0 {
                return Err(PersistenceError::conflict(
                    "identity_provider",
                    format!("alias `{}` already exists in realm", i.alias),
                ));
            }
            let tx = c.transaction()?;
            insert_idp_tx(&tx, &i)?;
            tx.commit()?;
            Ok(i)
        })
    }
    async fn update_idp(
        &self,
        mut i: IdentityProviderEntity,
    ) -> Result<IdentityProviderEntity> {
        i.audit.updated_at = Utc::now();
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE identity_providers SET realm_id=?1, alias=?2, display_name=?3, provider_id=?4, enabled=?5, trust_email=?6, store_token=?7, add_read_token_role_on_create=?8, authenticate_by_default=?9, link_only=?10, first_broker_login_flow_id=?11, post_broker_login_flow_id=?12, config_json=?13, mappers_json=?14, updated_at=?15, deleted_at=?16 WHERE id=?17",
                params![
                    uid_to_str(i.realm_id), i.alias, i.display_name, i.provider_id,
                    b(i.enabled), b(i.trust_email), b(i.store_token),
                    b(i.add_read_token_role_on_create), b(i.authenticate_by_default),
                    b(i.link_only),
                    i.first_broker_login_flow_id.map(uid_to_str),
                    i.post_broker_login_flow_id.map(uid_to_str),
                    serde_json::to_string(&i.config)?,
                    serde_json::to_string(&i.mappers)?,
                    ts(i.audit.updated_at), ts_opt(i.audit.deleted_at),
                    uid_to_str(i.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("identity_provider", i.id));
            }
            Ok(i)
        })
    }
    async fn delete_idp(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE identity_providers SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("identity_provider", id));
            }
            Ok(())
        })
    }
    async fn count_idps(&self, realm_id: Uuid) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM identity_providers WHERE realm_id=?1 AND deleted_at IS NULL",
                params![uid_to_str(realm_id)],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── AuthenticationFlow ───────────────────────────────────────────────
    async fn list_flows_in_realm(&self, realm_id: Uuid) -> Result<Vec<AuthFlowEntity>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT * FROM authentication_flows WHERE realm_id = ?1 AND deleted_at IS NULL",
            )?;
            let rows = stmt
                .query_map(params![uid_to_str(realm_id)], row_to_flow)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }
    async fn get_flow_by_id(&self, id: Uuid) -> Result<Option<AuthFlowEntity>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM authentication_flows WHERE id=?1 AND deleted_at IS NULL",
                params![uid_to_str(id)],
                row_to_flow,
            )
            .optional()?)
        })
    }
    async fn get_flow_by_name(
        &self,
        realm_id: Uuid,
        alias: &str,
    ) -> Result<Option<AuthFlowEntity>> {
        let alias = alias.to_string();
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT * FROM authentication_flows WHERE realm_id=?1 AND alias=?2 AND deleted_at IS NULL",
                params![uid_to_str(realm_id), alias],
                row_to_flow,
            )
            .optional()?)
        })
    }
    async fn create_flow(&self, f: AuthFlowEntity) -> Result<AuthFlowEntity> {
        self.with_conn(|c| {
            let exists: i64 = c.query_row(
                "SELECT COUNT(*) FROM authentication_flows WHERE realm_id=?1 AND alias=?2 AND deleted_at IS NULL",
                params![uid_to_str(f.realm_id), f.alias],
                |r| r.get(0),
            )?;
            if exists > 0 {
                return Err(PersistenceError::conflict(
                    "auth_flow",
                    format!("flow alias `{}` already exists in realm", f.alias),
                ));
            }
            let tx = c.transaction()?;
            insert_flow_tx(&tx, &f)?;
            tx.commit()?;
            Ok(f)
        })
    }
    async fn update_flow(&self, mut f: AuthFlowEntity) -> Result<AuthFlowEntity> {
        f.audit.updated_at = Utc::now();
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE authentication_flows SET realm_id=?1, alias=?2, description=?3, provider_id=?4, top_level=?5, built_in=?6, executions_json=?7, updated_at=?8, deleted_at=?9 WHERE id=?10",
                params![
                    uid_to_str(f.realm_id), f.alias, f.description, f.provider_id,
                    b(f.top_level), b(f.built_in),
                    serde_json::to_string(&f.executions)?,
                    ts(f.audit.updated_at), ts_opt(f.audit.deleted_at),
                    uid_to_str(f.id),
                ],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("auth_flow", f.id));
            }
            Ok(f)
        })
    }
    async fn delete_flow(&self, id: Uuid) -> Result<()> {
        self.with_conn(|c| {
            let rows = c.execute(
                "UPDATE authentication_flows SET deleted_at=?1, updated_at=?2 WHERE id=?3 AND deleted_at IS NULL",
                params![ts(Utc::now()), ts(Utc::now()), uid_to_str(id)],
            )?;
            if rows == 0 {
                return Err(PersistenceError::not_found("auth_flow", id));
            }
            Ok(())
        })
    }
    async fn count_flows(&self, realm_id: Uuid) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM authentication_flows WHERE realm_id=?1 AND deleted_at IS NULL",
                params![uid_to_str(realm_id)],
                |r| r.get(0),
            )?;
            Ok(n as usize)
        })
    }

    // ── Transaction ──────────────────────────────────────────────────────
    async fn begin_txn(&self) -> Result<Box<dyn Transaction>> {
        let snapshot = self.with_conn(|c| {
            let mut realms: Vec<RealmEntity> = Vec::new();
            let mut users: Vec<UserEntity> = Vec::new();
            let mut clients: Vec<ClientEntity> = Vec::new();
            let mut roles: Vec<RoleEntity> = Vec::new();
            let mut groups: Vec<GroupEntity> = Vec::new();
            let mut idps: Vec<IdentityProviderEntity> = Vec::new();
            let mut flows: Vec<AuthFlowEntity> = Vec::new();
            {
                let mut stmt = c.prepare("SELECT * FROM realms")?;
                for row in stmt.query_map([], row_to_realm)? {
                    realms.push(row?);
                }
            }
            {
                let mut stmt = c.prepare("SELECT * FROM users")?;
                for row in stmt.query_map([], row_to_user)? {
                    users.push(row?);
                }
            }
            {
                let mut stmt = c.prepare("SELECT * FROM clients")?;
                for row in stmt.query_map([], row_to_client)? {
                    clients.push(row?);
                }
            }
            {
                let mut stmt = c.prepare("SELECT * FROM roles")?;
                for row in stmt.query_map([], row_to_role)? {
                    roles.push(row?);
                }
            }
            {
                let mut stmt = c.prepare("SELECT * FROM groups_tbl")?;
                for row in stmt.query_map([], row_to_group)? {
                    groups.push(row?);
                }
            }
            {
                let mut stmt = c.prepare("SELECT * FROM identity_providers")?;
                for row in stmt.query_map([], row_to_idp)? {
                    idps.push(row?);
                }
            }
            {
                let mut stmt = c.prepare("SELECT * FROM authentication_flows")?;
                for row in stmt.query_map([], row_to_flow)? {
                    flows.push(row?);
                }
            }
            Ok(Snapshot {
                realms,
                users,
                clients,
                roles,
                groups,
                idps,
                flows,
            })
        })?;
        Ok(Box::new(RdbmsTxn {
            backend: self.clone(),
            snapshot,
            rollback_on_drop: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> RdbmsBackend {
        RdbmsBackend::in_memory().expect("open in-memory")
    }

    #[tokio::test]
    async fn migrations_applied_exactly_once() {
        let b = fresh();
        let applied = b.applied_migrations().unwrap();
        let versions: Vec<_> = applied.iter().map(|m| m.version.as_str()).collect();
        assert_eq!(versions, vec!["V001", "V002", "V003", "V004"]);
    }

    #[tokio::test]
    async fn create_and_get_realm_roundtrip_rdbms() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("master")).await.unwrap();
        let got = b.get_realm_by_id(r.id).await.unwrap().unwrap();
        assert_eq!(got.name, "master");
        assert_eq!(got.audit.created_at, r.audit.created_at);
    }

    #[tokio::test]
    async fn realm_name_unique_conflict_rdbms() {
        let b = fresh();
        b.create_realm(RealmEntity::new("dup")).await.unwrap();
        let err = b.create_realm(RealmEntity::new("dup")).await;
        assert!(matches!(err, Err(PersistenceError::Conflict { .. })));
    }

    #[tokio::test]
    async fn soft_delete_excludes_realm_rdbms() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("ephemeral")).await.unwrap();
        b.delete_realm(r.id).await.unwrap();
        assert!(b.get_realm_by_id(r.id).await.unwrap().is_none());
        assert_eq!(b.count_realms().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn user_attributes_serde_persists() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let mut u = UserEntity::new(r.id, "alice");
        u.email = Some("alice@example.com".into());
        u.attributes
            .insert("team".into(), vec!["platform".into(), "infra".into()]);
        u.credentials.push(UserCredential {
            id: Uuid::new_v4(),
            credential_type: "password".into(),
            secret_data: "{\"salt\":\"abc\",\"hash\":\"def\"}".into(),
            credential_data: "{\"algorithm\":\"pbkdf2-sha256\"}".into(),
            priority: 0,
        });
        let stored = b.create_user(u.clone()).await.unwrap();
        let fetched = b.get_user_by_id(stored.id).await.unwrap().unwrap();
        assert_eq!(fetched.email.as_deref(), Some("alice@example.com"));
        assert_eq!(
            fetched.attributes.get("team"),
            Some(&vec!["platform".to_string(), "infra".to_string()])
        );
        assert_eq!(fetched.credentials.len(), 1);
        assert_eq!(fetched.credentials[0].credential_type, "password");
    }

    #[tokio::test]
    async fn client_redirect_uris_roundtrip() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let mut c = ClientEntity::new(r.id, "spa");
        c.redirect_uris = vec!["https://app.example.com/callback".into()];
        c.web_origins = vec!["https://app.example.com".into()];
        c.public_client = true;
        let saved = b.create_client(c.clone()).await.unwrap();
        let got = b.get_client_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(got.redirect_uris, c.redirect_uris);
        assert!(got.public_client);
    }

    #[tokio::test]
    async fn idp_mappers_roundtrip() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let mut i = IdentityProviderEntity::new(r.id, "okta-prod", "oidc");
        i.config
            .insert("clientId".into(), "cave-runtime".into());
        i.mappers.push(IdpMapper {
            id: Uuid::new_v4(),
            name: "email-mapper".into(),
            mapper_type: "user-attribute".into(),
            config: Default::default(),
        });
        let saved = b.create_idp(i.clone()).await.unwrap();
        let got = b.get_idp_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(got.mappers.len(), 1);
        assert_eq!(got.mappers[0].name, "email-mapper");
        assert_eq!(got.config.get("clientId").map(String::as_str), Some("cave-runtime"));
    }

    #[tokio::test]
    async fn flow_executions_roundtrip() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let mut f = AuthFlowEntity::new(r.id, "browser");
        f.executions.push(FlowExecution {
            id: Uuid::new_v4(),
            authenticator: "auth-username-password-form".into(),
            requirement: "REQUIRED".into(),
            priority: 10,
            authenticator_config: Default::default(),
        });
        f.executions.push(FlowExecution {
            id: Uuid::new_v4(),
            authenticator: "auth-otp-form".into(),
            requirement: "CONDITIONAL".into(),
            priority: 20,
            authenticator_config: Default::default(),
        });
        let saved = b.create_flow(f.clone()).await.unwrap();
        let got = b.get_flow_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(got.executions.len(), 2);
        assert_eq!(got.executions[1].requirement, "CONDITIONAL");
    }

    #[tokio::test]
    async fn rdbms_txn_rollback_restores() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        b.create_user(UserEntity::new(r.id, "pre")).await.unwrap();

        let txn = b.begin_txn().await.unwrap();
        b.create_user(UserEntity::new(r.id, "transient"))
            .await
            .unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 2);
        txn.rollback().await.unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 1);
        assert!(b
            .get_user_by_name(r.id, "transient")
            .await
            .unwrap()
            .is_none());
        assert!(b.get_user_by_name(r.id, "pre").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn rdbms_txn_commit_keeps() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let txn = b.begin_txn().await.unwrap();
        b.create_user(UserEntity::new(r.id, "alice"))
            .await
            .unwrap();
        txn.commit().await.unwrap();
        assert_eq!(b.count_users(r.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn group_role_ids_roundtrip() {
        let b = fresh();
        let r = b.create_realm(RealmEntity::new("r")).await.unwrap();
        let role = b
            .create_role(RoleEntity::new(r.id, "admin"))
            .await
            .unwrap();
        let mut g = GroupEntity::new(r.id, "admins");
        g.role_ids.push(role.id);
        let saved = b.create_group(g).await.unwrap();
        let got = b.get_group_by_id(saved.id).await.unwrap().unwrap();
        assert_eq!(got.role_ids, vec![role.id]);
    }

    #[tokio::test]
    async fn double_open_replays_no_migrations() {
        // Opening the same file twice must not error.
        let tmp =
            std::env::temp_dir().join(format!("cave_auth_rdbms_test_{}.sqlite", Uuid::new_v4()));
        {
            let b = RdbmsBackend::open(&tmp).unwrap();
            b.create_realm(RealmEntity::new("persisted"))
                .await
                .unwrap();
        }
        let b2 = RdbmsBackend::open(&tmp).unwrap();
        let applied = b2.applied_migrations().unwrap();
        assert_eq!(applied.len(), 4);
        assert!(b2.get_realm_by_name("persisted").await.unwrap().is_some());
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn parse_uid_round_trip_and_error() {
        let id = Uuid::new_v4();
        let s = uid_to_str(id);
        assert_eq!(parse_uid(&s).unwrap(), id);
        assert!(parse_uid("not-a-uuid").is_err());
        assert_eq!(parse_uid_opt(None).unwrap(), None);
        assert_eq!(parse_uid_opt(Some("".into())).unwrap(), None);
        assert_eq!(parse_uid_opt(Some(s.clone())).unwrap(), Some(id));
    }
}
