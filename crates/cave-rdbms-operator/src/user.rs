// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::{PgError, PgResult};
use crate::types::PgRole;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct UserManager {
    roles: Arc<RwLock<HashMap<String, PgRole>>>,
}

impl Default for UserManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UserManager {
    pub fn new() -> Self {
        UserManager {
            roles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate CREATE ROLE SQL and store the role.
    pub fn create_role(&self, role: PgRole) -> PgResult<String> {
        let mut sql = format!("CREATE ROLE \"{}\"", role.name);

        if role.login {
            sql.push_str(" LOGIN");
        } else {
            sql.push_str(" NOLOGIN");
        }
        if role.superuser {
            sql.push_str(" SUPERUSER");
        }
        if role.create_db {
            sql.push_str(" CREATEDB");
        }
        if role.create_role {
            sql.push_str(" CREATEROLE");
        }
        if role.replication {
            sql.push_str(" REPLICATION");
        }
        if role.connection_limit >= 0 {
            sql.push_str(&format!(" CONNECTION LIMIT {}", role.connection_limit));
        }
        if let Some(ref valid_until) = role.valid_until {
            sql.push_str(&format!(" VALID UNTIL '{}'", valid_until.to_rfc3339()));
        }
        sql.push(';');

        let mut roles = self.roles.write().unwrap();
        if roles.contains_key(&role.name) {
            return Err(PgError::UserError(format!("role already exists: {}", role.name)));
        }
        roles.insert(role.name.clone(), role);
        Ok(sql)
    }

    /// Generate DROP ROLE SQL.
    pub fn drop_role(&self, name: &str) -> PgResult<String> {
        let mut roles = self.roles.write().unwrap();
        if roles.remove(name).is_none() {
            return Err(PgError::UserError(format!("role not found: {}", name)));
        }
        Ok(format!("DROP ROLE \"{}\";", name))
    }

    /// Generate ALTER ROLE SQL.
    pub fn alter_role(&self, name: &str, updates: RoleUpdate) -> PgResult<String> {
        let mut roles = self.roles.write().unwrap();
        let role = roles
            .get_mut(name)
            .ok_or_else(|| PgError::UserError(format!("role not found: {}", name)))?;

        let mut parts: Vec<String> = Vec::new();

        if let Some(superuser) = updates.superuser {
            role.superuser = superuser;
            parts.push(if superuser { "SUPERUSER".to_string() } else { "NOSUPERUSER".to_string() });
        }
        if let Some(create_db) = updates.create_db {
            role.create_db = create_db;
            parts.push(if create_db { "CREATEDB".to_string() } else { "NOCREATEDB".to_string() });
        }
        if let Some(create_role) = updates.create_role {
            role.create_role = create_role;
            parts.push(if create_role { "CREATEROLE".to_string() } else { "NOCREATEROLE".to_string() });
        }
        if let Some(login) = updates.login {
            role.login = login;
            parts.push(if login { "LOGIN".to_string() } else { "NOLOGIN".to_string() });
        }
        if let Some(limit) = updates.connection_limit {
            role.connection_limit = limit;
            parts.push(format!("CONNECTION LIMIT {}", limit));
        }
        if let Some(ref password) = updates.password {
            parts.push(format!("PASSWORD '{}'", password));
        }

        if parts.is_empty() {
            Ok(format!("ALTER ROLE \"{}\";", name))
        } else {
            Ok(format!("ALTER ROLE \"{}\" {};", name, parts.join(" ")))
        }
    }

    pub fn grant_role(&self, role: &str, to: &str) -> String {
        format!("GRANT \"{}\" TO \"{}\";", role, to)
    }

    pub fn revoke_role(&self, role: &str, from: &str) -> String {
        format!("REVOKE \"{}\" FROM \"{}\";", role, from)
    }

    pub fn create_user_sql(username: &str, password: &str, options: &UserOptions) -> String {
        let mut sql = format!("CREATE USER \"{}\" WITH PASSWORD '{}'", username, password);
        if options.superuser {
            sql.push_str(" SUPERUSER");
        }
        if options.create_db {
            sql.push_str(" CREATEDB");
        }
        if options.login {
            sql.push_str(" LOGIN");
        }
        if options.connection_limit >= 0 {
            sql.push_str(&format!(" CONNECTION LIMIT {}", options.connection_limit));
        }
        sql.push(';');
        sql
    }

    pub fn register_role(&self, role: PgRole) -> PgResult<()> {
        let mut roles = self.roles.write().unwrap();
        roles.insert(role.name.clone(), role);
        Ok(())
    }

    pub fn get_role(&self, name: &str) -> Option<PgRole> {
        let roles = self.roles.read().unwrap();
        roles.get(name).cloned()
    }

    pub fn list_roles(&self) -> Vec<PgRole> {
        let roles = self.roles.read().unwrap();
        roles.values().cloned().collect()
    }
}

pub struct RoleUpdate {
    pub superuser: Option<bool>,
    pub create_db: Option<bool>,
    pub create_role: Option<bool>,
    pub login: Option<bool>,
    pub connection_limit: Option<i32>,
    pub password: Option<String>,
}

pub struct UserOptions {
    pub superuser: bool,
    pub create_db: bool,
    pub login: bool,
    pub connection_limit: i32,
}
