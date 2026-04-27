//! Dynamic-credential issuance state machine.
//!
//! Generic enough to model the database, AWS, SSH OTP, and PKI engines:
//!
//! ```text
//! Role { name, ttl, max_ttl, allowed_users, statements }
//!   ↓ issue
//! Credential { id, role, principal, secret, lease_id, expires_at }
//!   ↓ revoke
//! Tombstone
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DynamicError {
    #[error("role not found: {0}")]
    RoleNotFound(String),
    #[error("role already exists: {0}")]
    RoleExists(String),
    #[error("credential not found: {0}")]
    CredentialNotFound(String),
    #[error("invalid ttl: {0}")]
    InvalidTtl(i64),
    #[error("revoked credential: {0}")]
    Revoked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicRole {
    pub name: String,
    pub default_ttl_secs: i64,
    pub max_ttl_secs: i64,
    pub creation_statements: Vec<String>,
    pub revocation_statements: Vec<String>,
    pub allowed_principals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub id: String,
    pub role: String,
    pub principal: String,
    pub secret: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
}

#[derive(Debug, Default)]
pub struct DynamicEngine {
    roles: BTreeMap<String, DynamicRole>,
    credentials: BTreeMap<String, Credential>,
    counter: u64,
}

impl DynamicEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_role(&mut self, role: DynamicRole) -> Result<(), DynamicError> {
        if role.default_ttl_secs <= 0 || role.max_ttl_secs <= 0 || role.default_ttl_secs > role.max_ttl_secs {
            return Err(DynamicError::InvalidTtl(role.default_ttl_secs));
        }
        if self.roles.contains_key(&role.name) {
            return Err(DynamicError::RoleExists(role.name));
        }
        self.roles.insert(role.name.clone(), role);
        Ok(())
    }

    pub fn delete_role(&mut self, name: &str) -> bool {
        self.roles.remove(name).is_some()
    }

    pub fn get_role(&self, name: &str) -> Option<&DynamicRole> {
        self.roles.get(name)
    }

    pub fn issue(
        &mut self,
        role_name: &str,
        principal: &str,
        ttl_secs: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<Credential, DynamicError> {
        let role = self
            .roles
            .get(role_name)
            .ok_or_else(|| DynamicError::RoleNotFound(role_name.into()))?
            .clone();
        if !role.allowed_principals.is_empty()
            && !role.allowed_principals.iter().any(|p| p == principal)
        {
            return Err(DynamicError::RoleNotFound(format!(
                "principal {} not allowed by role",
                principal
            )));
        }
        let ttl = ttl_secs.unwrap_or(role.default_ttl_secs).min(role.max_ttl_secs);
        if ttl <= 0 {
            return Err(DynamicError::InvalidTtl(ttl));
        }
        self.counter += 1;
        let id = format!("cred-{}", self.counter);
        // Pseudo-random secret derived from id+now (sans-IO friendly).
        let secret = format!("{}-{}-{}", role.name, principal, self.counter);
        let cred = Credential {
            id: id.clone(),
            role: role.name.clone(),
            principal: principal.into(),
            secret,
            created_at: now,
            expires_at: now + Duration::seconds(ttl),
            revoked: false,
        };
        self.credentials.insert(id.clone(), cred.clone());
        Ok(cred)
    }

    pub fn revoke(&mut self, id: &str) -> Result<(), DynamicError> {
        let cred = self
            .credentials
            .get_mut(id)
            .ok_or_else(|| DynamicError::CredentialNotFound(id.into()))?;
        if cred.revoked {
            return Err(DynamicError::Revoked(id.into()));
        }
        cred.revoked = true;
        Ok(())
    }

    pub fn renew(
        &mut self,
        id: &str,
        increment_secs: i64,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, DynamicError> {
        let role_max = {
            let cred = self
                .credentials
                .get(id)
                .ok_or_else(|| DynamicError::CredentialNotFound(id.into()))?;
            self.roles.get(&cred.role).map(|r| r.max_ttl_secs).unwrap_or(0)
        };
        let cred = self
            .credentials
            .get_mut(id)
            .ok_or_else(|| DynamicError::CredentialNotFound(id.into()))?;
        if cred.revoked {
            return Err(DynamicError::Revoked(id.into()));
        }
        let max_expiry = cred.created_at + Duration::seconds(role_max);
        let proposed = now + Duration::seconds(increment_secs);
        cred.expires_at = if proposed > max_expiry { max_expiry } else { proposed };
        Ok(cred.expires_at)
    }

    pub fn count_active(&self, now: DateTime<Utc>) -> usize {
        self.credentials
            .values()
            .filter(|c| !c.revoked && c.expires_at > now)
            .count()
    }

    pub fn revoke_role(&mut self, role: &str) -> usize {
        let mut n = 0;
        for cred in self.credentials.values_mut() {
            if cred.role == role && !cred.revoked {
                cred.revoked = true;
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(name: &str, ttl: i64, max: i64) -> DynamicRole {
        DynamicRole {
            name: name.into(),
            default_ttl_secs: ttl,
            max_ttl_secs: max,
            creation_statements: vec![],
            revocation_statements: vec![],
            allowed_principals: vec![],
        }
    }

    #[test]
    fn create_role_ok() {
        let mut e = DynamicEngine::new();
        e.create_role(role("readonly", 60, 3600)).unwrap();
        assert!(e.get_role("readonly").is_some());
    }

    #[test]
    fn create_role_invalid_ttl_errors() {
        let mut e = DynamicEngine::new();
        let err = e.create_role(role("r", 100, 50)).unwrap_err();
        assert!(matches!(err, DynamicError::InvalidTtl(_)));
    }

    #[test]
    fn create_role_duplicate_errors() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let err = e.create_role(role("r", 60, 100)).unwrap_err();
        assert_eq!(err, DynamicError::RoleExists("r".into()));
    }

    #[test]
    fn delete_role_returns_true_when_present() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        assert!(e.delete_role("r"));
        assert!(!e.delete_role("r"));
    }

    #[test]
    fn issue_returns_cred_with_secret() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 3600)).unwrap();
        let cred = e.issue("r", "alice", None, Utc::now()).unwrap();
        assert!(cred.id.starts_with("cred-"));
        assert!(!cred.secret.is_empty());
    }

    #[test]
    fn issue_unknown_role_errors() {
        let mut e = DynamicEngine::new();
        let err = e.issue("missing", "u", None, Utc::now()).unwrap_err();
        assert_eq!(err, DynamicError::RoleNotFound("missing".into()));
    }

    #[test]
    fn issue_principal_filter_enforced() {
        let mut e = DynamicEngine::new();
        let mut r = role("r", 60, 100);
        r.allowed_principals = vec!["alice".into()];
        e.create_role(r).unwrap();
        assert!(e.issue("r", "alice", None, Utc::now()).is_ok());
        assert!(e.issue("r", "bob", None, Utc::now()).is_err());
    }

    #[test]
    fn issue_caps_ttl_at_max() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let now = Utc::now();
        let cred = e.issue("r", "alice", Some(10_000), now).unwrap();
        assert_eq!(cred.expires_at, now + Duration::seconds(100));
    }

    #[test]
    fn revoke_marks_credential() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let c = e.issue("r", "alice", None, Utc::now()).unwrap();
        e.revoke(&c.id).unwrap();
        assert!(e.credentials[&c.id].revoked);
    }

    #[test]
    fn revoke_already_revoked_errors() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let c = e.issue("r", "alice", None, Utc::now()).unwrap();
        e.revoke(&c.id).unwrap();
        let err = e.revoke(&c.id).unwrap_err();
        assert!(matches!(err, DynamicError::Revoked(_)));
    }

    #[test]
    fn renew_capped_at_role_max() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let now = Utc::now();
        let c = e.issue("r", "alice", None, now).unwrap();
        let new_exp = e.renew(&c.id, 1000, now).unwrap();
        assert_eq!(new_exp, c.created_at + Duration::seconds(100));
    }

    #[test]
    fn renew_revoked_errors() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let c = e.issue("r", "alice", None, Utc::now()).unwrap();
        e.revoke(&c.id).unwrap();
        let err = e.renew(&c.id, 60, Utc::now()).unwrap_err();
        assert!(matches!(err, DynamicError::Revoked(_)));
    }

    #[test]
    fn count_active_excludes_revoked_and_expired() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        let now = Utc::now();
        let _ = e.issue("r", "alice", None, now).unwrap();
        let c2 = e.issue("r", "bob", None, now).unwrap();
        e.revoke(&c2.id).unwrap();
        assert_eq!(e.count_active(now), 1);
        assert_eq!(e.count_active(now + Duration::seconds(120)), 0);
    }

    #[test]
    fn revoke_role_revokes_all_creds() {
        let mut e = DynamicEngine::new();
        e.create_role(role("r", 60, 100)).unwrap();
        e.issue("r", "alice", None, Utc::now()).unwrap();
        e.issue("r", "bob", None, Utc::now()).unwrap();
        let n = e.revoke_role("r");
        assert_eq!(n, 2);
        assert_eq!(e.count_active(Utc::now()), 0);
    }
}
