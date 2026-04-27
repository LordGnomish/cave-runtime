//! Tenant invariant for cave-docdb — default-deny.
//!
//! Each database is bound to a single tenant id. Cross-tenant queries are
//! refused before reaching the engine.

use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TenantError {
    #[error("invalid tenant id: {0}")]
    Invalid(String),
    #[error("tenant denied: {0}")]
    Denied(String),
}

pub fn validate_tenant_id(s: &str) -> Result<(), TenantError> {
    if s.is_empty() || s.len() > 63 {
        return Err(TenantError::Invalid(format!("len {}", s.len())));
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err(TenantError::Invalid("leading/trailing hyphen".into()));
    }
    for ch in s.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
        if !ok {
            return Err(TenantError::Invalid(format!("invalid char '{}'", ch)));
        }
    }
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct TenantPolicy {
    allowed: BTreeSet<String>,
}

impl TenantPolicy {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn allow(&mut self, t: &str) -> Result<(), TenantError> {
        validate_tenant_id(t)?;
        self.allowed.insert(t.to_string());
        Ok(())
    }
    pub fn revoke(&mut self, t: &str) -> bool {
        self.allowed.remove(t)
    }
    pub fn authorize<'a>(&self, t: Option<&'a str>) -> Result<&'a str, TenantError> {
        let t = t.ok_or_else(|| TenantError::Denied("no binding".into()))?;
        validate_tenant_id(t)?;
        if self.allowed.contains(t) {
            Ok(t)
        } else {
            Err(TenantError::Denied(t.to_string()))
        }
    }
    pub fn count(&self) -> usize {
        self.allowed.len()
    }
}

/// Parse the tenant id from a MongoDB connection options string.
/// MongoDB drivers carry vendor extensions in `appName` or via `?cave_tenant=`.
pub fn extract_from_app_name(app_name: &str) -> Option<&str> {
    for token in app_name.split(';') {
        if let Some(rest) = token.strip_prefix("cave_tenant=") {
            return Some(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_denies() {
        let p = TenantPolicy::new();
        assert!(p.authorize(Some("a")).is_err());
    }
    #[test]
    fn allow_then_authorize() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        assert_eq!(p.authorize(Some("acme")).unwrap(), "acme");
    }
    #[test]
    fn revoke_drops_grant() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        p.revoke("acme");
        assert!(p.authorize(Some("acme")).is_err());
    }
    #[test]
    fn validate_uppercase_fails() {
        assert!(validate_tenant_id("Acme").is_err());
    }
    #[test]
    fn validate_long_fails() {
        assert!(validate_tenant_id(&"x".repeat(64)).is_err());
    }
    #[test]
    fn extract_from_app_name_finds_tenant() {
        assert_eq!(
            extract_from_app_name("driver=python;cave_tenant=acme;version=1"),
            Some("acme")
        );
    }
    #[test]
    fn extract_from_app_name_returns_none_when_absent() {
        assert_eq!(extract_from_app_name("driver=python"), None);
    }
    #[test]
    fn no_binding_denied() {
        let p = TenantPolicy::new();
        assert!(p.authorize(None).is_err());
    }
}
