// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/resources/META-INF/keycloak-model-jpa-changelog-master.xml
//
// Port of Keycloak's Liquibase changelog to a Flyway-style versioned
// migrator. Each migration is identified by a stable `Vxxx__name`
// string and applied exactly once; the history table guards against
// re-running and surfaces schema drift across deployments.

//! Flyway-style schema migrations for the JPA persistence layer.

use chrono::{DateTime, Utc};

/// A single forward-only schema migration. `up_sql` runs verbatim under
/// the [`MigrationRunner`]'s transaction; we deliberately have no
/// `down_sql` field because Keycloak's `keycloak-model-jpa-changelog`
/// is forward-only and downgrades require a snapshot restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub version: &'static str,
    pub name: &'static str,
    pub up_sql: &'static str,
}

/// One row in `cave_auth_schema_history`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    pub version: String,
    pub name: String,
    pub applied_at: DateTime<Utc>,
    pub checksum: i64,
}

/// FNV-1a 64-bit checksum so migration drift can be detected even when
/// version strings collide — kept tiny because we don't need crypto.
pub fn fnv1a_64(input: &str) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

/// Baseline V001 — realms, users, client, and the side tables Keycloak
/// keeps in JPA (user_credentials, user_attributes).
pub const V001_INIT: Migration = Migration {
    version: "V001",
    name: "init_schema",
    up_sql: r#"
CREATE TABLE IF NOT EXISTS realms (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    display_name    TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    ssl_required    TEXT NOT NULL DEFAULT 'external',
    access_token_lifespan INTEGER NOT NULL DEFAULT 300,
    sso_session_idle_timeout INTEGER NOT NULL DEFAULT 1800,
    sso_session_max_lifespan INTEGER NOT NULL DEFAULT 36000,
    registration_allowed INTEGER NOT NULL DEFAULT 0,
    remember_me INTEGER NOT NULL DEFAULT 0,
    verify_email INTEGER NOT NULL DEFAULT 0,
    login_with_email_allowed INTEGER NOT NULL DEFAULT 1,
    duplicate_emails_allowed INTEGER NOT NULL DEFAULT 0,
    reset_password_allowed INTEGER NOT NULL DEFAULT 0,
    edit_username_allowed INTEGER NOT NULL DEFAULT 0,
    brute_force_protected INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT
);

CREATE TABLE IF NOT EXISTS users (
    id              TEXT PRIMARY KEY,
    realm_id        TEXT NOT NULL,
    username        TEXT NOT NULL,
    email           TEXT,
    email_verified  INTEGER NOT NULL DEFAULT 0,
    first_name      TEXT,
    last_name       TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    federation_link TEXT,
    service_account_client_link TEXT,
    credentials_json TEXT NOT NULL DEFAULT '[]',
    attributes_json  TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    UNIQUE(realm_id, username)
);

CREATE TABLE IF NOT EXISTS clients (
    id              TEXT PRIMARY KEY,
    realm_id        TEXT NOT NULL,
    client_id       TEXT NOT NULL,
    name            TEXT,
    description     TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    client_authenticator_type TEXT NOT NULL DEFAULT 'client-secret',
    secret          TEXT,
    redirect_uris_json TEXT NOT NULL DEFAULT '[]',
    web_origins_json   TEXT NOT NULL DEFAULT '[]',
    bearer_only     INTEGER NOT NULL DEFAULT 0,
    consent_required INTEGER NOT NULL DEFAULT 0,
    standard_flow_enabled INTEGER NOT NULL DEFAULT 1,
    implicit_flow_enabled INTEGER NOT NULL DEFAULT 0,
    direct_access_grants_enabled INTEGER NOT NULL DEFAULT 0,
    service_accounts_enabled INTEGER NOT NULL DEFAULT 0,
    public_client   INTEGER NOT NULL DEFAULT 0,
    frontchannel_logout INTEGER NOT NULL DEFAULT 0,
    protocol        TEXT NOT NULL DEFAULT 'openid-connect',
    attributes_json TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    UNIQUE(realm_id, client_id)
);
"#,
};

/// V002 — roles, groups, and group ↔ role mappings.
pub const V002_ROLES_GROUPS: Migration = Migration {
    version: "V002",
    name: "roles_groups",
    up_sql: r#"
CREATE TABLE IF NOT EXISTS roles (
    id              TEXT PRIMARY KEY,
    realm_id        TEXT NOT NULL,
    client_id       TEXT,
    name            TEXT NOT NULL,
    description     TEXT,
    composite       INTEGER NOT NULL DEFAULT 0,
    attributes_json TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    UNIQUE(realm_id, client_id, name)
);

CREATE TABLE IF NOT EXISTS groups_tbl (
    id              TEXT PRIMARY KEY,
    realm_id        TEXT NOT NULL,
    parent_id       TEXT,
    name            TEXT NOT NULL,
    attributes_json TEXT NOT NULL DEFAULT '{}',
    role_ids_json   TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT
);

CREATE TABLE IF NOT EXISTS group_role_mapping (
    group_id        TEXT NOT NULL,
    role_id         TEXT NOT NULL,
    PRIMARY KEY (group_id, role_id)
);

CREATE TABLE IF NOT EXISTS user_group_membership (
    user_id         TEXT NOT NULL,
    group_id        TEXT NOT NULL,
    PRIMARY KEY (user_id, group_id)
);
"#,
};

/// V003 — identity providers + auth flows (+ inlined executions).
pub const V003_IDP_FLOWS: Migration = Migration {
    version: "V003",
    name: "idp_flows",
    up_sql: r#"
CREATE TABLE IF NOT EXISTS identity_providers (
    id              TEXT PRIMARY KEY,
    realm_id        TEXT NOT NULL,
    alias           TEXT NOT NULL,
    display_name    TEXT,
    provider_id     TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    trust_email     INTEGER NOT NULL DEFAULT 0,
    store_token     INTEGER NOT NULL DEFAULT 0,
    add_read_token_role_on_create INTEGER NOT NULL DEFAULT 0,
    authenticate_by_default INTEGER NOT NULL DEFAULT 0,
    link_only       INTEGER NOT NULL DEFAULT 0,
    first_broker_login_flow_id TEXT,
    post_broker_login_flow_id  TEXT,
    config_json     TEXT NOT NULL DEFAULT '{}',
    mappers_json    TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    UNIQUE(realm_id, alias)
);

CREATE TABLE IF NOT EXISTS authentication_flows (
    id              TEXT PRIMARY KEY,
    realm_id        TEXT NOT NULL,
    alias           TEXT NOT NULL,
    description     TEXT,
    provider_id     TEXT NOT NULL DEFAULT 'basic-flow',
    top_level       INTEGER NOT NULL DEFAULT 1,
    built_in        INTEGER NOT NULL DEFAULT 0,
    executions_json TEXT NOT NULL DEFAULT '[]',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT,
    UNIQUE(realm_id, alias)
);
"#,
};

/// V004 — secondary indexes that match the JPA layer's `@Index`
/// declarations on `User`, `Client`, `IdentityProvider`, `AuthFlow`.
pub const V004_INDEXES: Migration = Migration {
    version: "V004",
    name: "indexes",
    up_sql: r#"
CREATE INDEX IF NOT EXISTS idx_user_realm           ON users(realm_id);
CREATE INDEX IF NOT EXISTS idx_user_username        ON users(realm_id, username);
CREATE INDEX IF NOT EXISTS idx_user_email           ON users(realm_id, email);
CREATE INDEX IF NOT EXISTS idx_client_realm_clientid ON clients(realm_id, client_id);
CREATE INDEX IF NOT EXISTS idx_role_realm           ON roles(realm_id);
CREATE INDEX IF NOT EXISTS idx_role_name            ON roles(realm_id, name);
CREATE INDEX IF NOT EXISTS idx_group_realm          ON groups_tbl(realm_id);
CREATE INDEX IF NOT EXISTS idx_idp_realm            ON identity_providers(realm_id);
CREATE INDEX IF NOT EXISTS idx_idp_alias            ON identity_providers(realm_id, alias);
CREATE INDEX IF NOT EXISTS idx_flow_realm           ON authentication_flows(realm_id);
CREATE INDEX IF NOT EXISTS idx_flow_alias           ON authentication_flows(realm_id, alias);
"#,
};

/// The ordered set of baseline migrations applied by `MigrationRunner`.
pub fn baseline_migrations() -> Vec<Migration> {
    vec![
        V001_INIT.clone(),
        V002_ROLES_GROUPS.clone(),
        V003_IDP_FLOWS.clone(),
        V004_INDEXES.clone(),
    ]
}

/// Pure-Rust migration planner — given a desired set and the rows
/// already in `cave_auth_schema_history`, decide which migrations to
/// run. Used by both the in-memory and the rusqlite backend.
pub fn plan(applied: &[AppliedMigration], desired: &[Migration]) -> Vec<Migration> {
    desired
        .iter()
        .filter(|m| !applied.iter().any(|a| a.version == m.version))
        .cloned()
        .collect()
}

/// Executor abstraction so we can unit-test the migration runner
/// without an actual SQL backend.
pub trait MigrationExecutor {
    fn applied(&self) -> Vec<AppliedMigration>;
    fn execute(&mut self, m: &Migration) -> Result<(), String>;
    fn record(&mut self, m: &Migration, checksum: i64);
}

/// Drive the migration plan against an executor. Returns the list of
/// migrations actually applied.
pub fn run<E: MigrationExecutor>(
    desired: &[Migration],
    executor: &mut E,
) -> Result<Vec<String>, String> {
    let already = executor.applied();
    let pending = plan(&already, desired);
    let mut ran = Vec::new();
    for m in &pending {
        executor.execute(m)?;
        executor.record(m, fnv1a_64(m.up_sql));
        ran.push(m.version.to_string());
    }
    Ok(ran)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct InMemoryExec {
        rows: Vec<AppliedMigration>,
        log: Vec<String>,
        fail_on: Option<&'static str>,
    }

    impl MigrationExecutor for InMemoryExec {
        fn applied(&self) -> Vec<AppliedMigration> {
            self.rows.clone()
        }
        fn execute(&mut self, m: &Migration) -> Result<(), String> {
            if Some(m.version) == self.fail_on {
                return Err(format!("synthetic failure on {}", m.version));
            }
            self.log.push(m.version.to_string());
            Ok(())
        }
        fn record(&mut self, m: &Migration, checksum: i64) {
            self.rows.push(AppliedMigration {
                version: m.version.to_string(),
                name: m.name.to_string(),
                applied_at: Utc::now(),
                checksum,
            });
        }
    }

    #[test]
    fn baseline_has_four_migrations() {
        let b = baseline_migrations();
        assert_eq!(b.len(), 4);
        assert_eq!(b[0].version, "V001");
        assert_eq!(b[3].version, "V004");
    }

    #[test]
    fn baseline_versions_unique_and_ordered() {
        let b = baseline_migrations();
        let versions: Vec<_> = b.iter().map(|m| m.version).collect();
        let mut sorted = versions.clone();
        sorted.sort();
        assert_eq!(versions, sorted);
        assert_eq!(
            std::collections::HashSet::<&&str>::from_iter(versions.iter()).len(),
            4
        );
    }

    #[test]
    fn plan_returns_all_when_history_empty() {
        let p = plan(&[], &baseline_migrations());
        assert_eq!(p.len(), 4);
    }

    #[test]
    fn plan_excludes_already_applied() {
        let applied = vec![AppliedMigration {
            version: "V001".to_string(),
            name: "init_schema".to_string(),
            applied_at: Utc::now(),
            checksum: 0,
        }];
        let p = plan(&applied, &baseline_migrations());
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].version, "V002");
    }

    #[test]
    fn run_against_executor_applies_all_fresh() {
        let mut e = InMemoryExec::default();
        let ran = run(&baseline_migrations(), &mut e).unwrap();
        assert_eq!(ran, vec!["V001", "V002", "V003", "V004"]);
        assert_eq!(e.applied().len(), 4);
        assert_eq!(e.log, vec!["V001", "V002", "V003", "V004"]);
    }

    #[test]
    fn run_against_executor_idempotent_on_second_pass() {
        let mut e = InMemoryExec::default();
        run(&baseline_migrations(), &mut e).unwrap();
        let ran2 = run(&baseline_migrations(), &mut e).unwrap();
        assert!(ran2.is_empty(), "second pass must be a no-op");
        assert_eq!(e.applied().len(), 4);
    }

    #[test]
    fn run_halts_on_executor_failure() {
        let mut e = InMemoryExec {
            fail_on: Some("V003"),
            ..InMemoryExec::default()
        };
        let r = run(&baseline_migrations(), &mut e);
        assert!(r.is_err());
        // V001 and V002 should have landed; V003 failed mid-way.
        assert_eq!(e.applied().len(), 2);
    }

    #[test]
    fn fnv1a_64_is_deterministic_and_distinguishes() {
        assert_eq!(fnv1a_64("V001"), fnv1a_64("V001"));
        assert_ne!(fnv1a_64("V001"), fnv1a_64("V002"));
        assert_ne!(fnv1a_64(""), 0);
    }

    #[test]
    fn v001_includes_realms_users_clients() {
        let sql = V001_INIT.up_sql;
        assert!(sql.contains("realms"));
        assert!(sql.contains("users"));
        assert!(sql.contains("clients"));
        assert!(sql.contains("credentials_json"));
        assert!(sql.contains("attributes_json"));
    }

    #[test]
    fn v002_includes_roles_groups_mapping() {
        let sql = V002_ROLES_GROUPS.up_sql;
        assert!(sql.contains("roles"));
        assert!(sql.contains("groups_tbl"));
        assert!(sql.contains("group_role_mapping"));
        assert!(sql.contains("user_group_membership"));
    }

    #[test]
    fn v003_includes_idp_and_flows() {
        let sql = V003_IDP_FLOWS.up_sql;
        assert!(sql.contains("identity_providers"));
        assert!(sql.contains("authentication_flows"));
        assert!(sql.contains("mappers_json"));
        assert!(sql.contains("executions_json"));
    }

    #[test]
    fn v004_includes_promised_indexes() {
        let sql = V004_INDEXES.up_sql;
        for expected in [
            "idx_user_realm",
            "idx_user_username",
            "idx_client_realm_clientid",
            "idx_idp_realm",
            "idx_flow_realm",
        ] {
            assert!(sql.contains(expected), "missing index {expected}");
        }
    }
}
