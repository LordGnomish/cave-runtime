//! Tenant invariant for cave-vault — default-deny.
//!
//! Vault namespaces map to tenants 1:1. Every API call must carry a tenant
//! id; no implicit fallback.

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
        self.allowed.insert(t.into());
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_denies() {
        assert!(TenantPolicy::new().authorize(Some("a")).is_err());
    }
    #[test]
    fn allow_then_authorize() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        assert_eq!(p.authorize(Some("acme")).unwrap(), "acme");
    }
    #[test]
    fn validate_invalid_chars_fails() {
        assert!(validate_tenant_id("a_b").is_err());
    }
    #[test]
    fn revoke_works() {
        let mut p = TenantPolicy::new();
        p.allow("a").unwrap();
        p.revoke("a");
        assert!(p.authorize(Some("a")).is_err());
    }
    #[test]
    fn no_binding_denied() {
        assert!(TenantPolicy::new().authorize(None).is_err());
    }
}
