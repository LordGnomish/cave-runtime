//! Database Secrets Engine — dynamic credential generation for PostgreSQL and MySQL.
//!
//! Generates short-lived credentials without a live DB connection.
//! The operator is expected to configure the DB role and the engine generates
//! credentials via the configured statements when a real connection is wired up.

use chrono::{DateTime, Utc};
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::VaultError;
use crate::models::LeaseInfo;

// ── Database types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DbType {
    Postgresql,
    Mysql,
}

// ── DB Role configuration ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbRole {
    pub name: String,
    pub db_name: String,
    pub db_type: DbType,
    pub creation_statements: Vec<String>,
    pub revocation_statements: Vec<String>,
    pub default_ttl: u64,
    pub max_ttl: u64,
}

// ── Dynamic credential ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicCreds {
    pub username: String,
    pub password: String,
    pub lease: LeaseInfo,
    pub db_name: String,
    pub role: String,
    pub revoked: bool,
    pub created_at: DateTime<Utc>,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct DatabaseEngine {
    pub roles: HashMap<String, DbRole>,
    pub active_creds: HashMap<String, DynamicCreds>,
}

impl DatabaseEngine {
    pub fn new() -> Self {
        Self {
            roles: HashMap::new(),
            active_creds: HashMap::new(),
        }
    }

    /// Configure a DB role.
    pub fn configure_role(&mut self, role: DbRole) {
        self.roles.insert(role.name.clone(), role);
    }

    /// Generate dynamic credentials for a role.
    pub fn generate_credentials(&mut self, role_name: &str) -> Result<DynamicCreds, VaultError> {
        let role = self
            .roles
            .get(role_name)
            .ok_or_else(|| VaultError::NotFound(format!("db role '{role_name}'")))?
            .clone();

        let prefix = match role.db_type {
            DbType::Postgresql => "v-pg",
            DbType::Mysql      => "v-my",
        };
        let username = format!(
            "{}-{}-{}",
            prefix,
            role_name,
            &Uuid::new_v4().to_string()[..8]
        );
        let password: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let lease = LeaseInfo::new(role.default_ttl, true);
        let lease_id = lease.lease_id.clone();

        let creds = DynamicCreds {
            username,
            password,
            lease,
            db_name: role.db_name.clone(),
            role: role_name.to_string(),
            revoked: false,
            created_at: Utc::now(),
        };

        self.active_creds.insert(lease_id, creds.clone());
        Ok(creds)
    }

    /// Revoke a dynamic credential by lease ID.
    pub fn revoke_credentials(&mut self, lease_id: &str) -> Result<(), VaultError> {
        let creds = self
            .active_creds
            .get_mut(lease_id)
            .ok_or_else(|| VaultError::NotFound(format!("lease {lease_id}")))?;
        creds.revoked = true;
        Ok(())
    }

    /// Prune expired credentials.
    pub fn prune_expired(&mut self) {
        self.active_creds.retain(|_, c| !c.lease.is_expired() && !c.revoked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_role(name: &str, db_type: DbType) -> DbRole {
        DbRole {
            name: name.to_string(),
            db_name: "mydb".to_string(),
            db_type,
            creation_statements: vec!["CREATE USER ...".into()],
            revocation_statements: vec!["DROP USER ...".into()],
            default_ttl: 3600,
            max_ttl: 86400,
        }
    }

    #[test]
    fn test_db_generate_postgres_creds() {
        let mut engine = DatabaseEngine::new();
        engine.configure_role(make_role("readonly", DbType::Postgresql));
        let creds = engine.generate_credentials("readonly").unwrap();
        assert!(creds.username.starts_with("v-pg-readonly-"));
        assert_eq!(creds.password.len(), 32);
        assert!(!creds.revoked);
    }

    #[test]
    fn test_db_generate_mysql_creds() {
        let mut engine = DatabaseEngine::new();
        engine.configure_role(make_role("writer", DbType::Mysql));
        let creds = engine.generate_credentials("writer").unwrap();
        assert!(creds.username.starts_with("v-my-writer-"));
    }

    #[test]
    fn test_db_revoke_credentials() {
        let mut engine = DatabaseEngine::new();
        engine.configure_role(make_role("rw", DbType::Postgresql));
        let creds = engine.generate_credentials("rw").unwrap();
        let lease_id = creds.lease.lease_id.clone();
        engine.revoke_credentials(&lease_id).unwrap();
        assert!(engine.active_creds[&lease_id].revoked);
    }

    #[test]
    fn test_db_unknown_role_error() {
        let mut engine = DatabaseEngine::new();
        let result = engine.generate_credentials("nonexistent");
        assert!(result.is_err());
    }
}
