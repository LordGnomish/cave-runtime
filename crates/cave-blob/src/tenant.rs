//! Tenant invariant — default deny.
//!
//! Mirrors `cave-pg::wire::tenant`. Every bucket / container creation must
//! supply a tenant id; objects inherit their tenant from their parent bucket.

use std::collections::BTreeSet;

use crate::error::{BlobError, BlobResult};

pub fn validate_tenant_id(s: &str) -> BlobResult<()> {
    if s.is_empty() {
        return Err(BlobError::TenantDenied("empty".into()));
    }
    if s.len() > 63 {
        return Err(BlobError::TenantDenied("too long".into()));
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err(BlobError::TenantDenied("leading/trailing hyphen".into()));
    }
    for ch in s.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
        if !ok {
            return Err(BlobError::TenantDenied(format!("invalid char '{}'", ch)));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct TenantPolicy {
    allowed: BTreeSet<String>,
}

impl TenantPolicy {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn allow(&mut self, t: &str) -> BlobResult<()> {
        validate_tenant_id(t)?;
        self.allowed.insert(t.to_string());
        Ok(())
    }
    pub fn revoke(&mut self, t: &str) -> bool {
        self.allowed.remove(t)
    }
    pub fn authorize<'a>(&self, t: Option<&'a str>) -> BlobResult<&'a str> {
        let t = t.ok_or_else(|| BlobError::TenantDenied("no binding".into()))?;
        validate_tenant_id(t)?;
        if self.allowed.contains(t) {
            Ok(t)
        } else {
            Err(BlobError::TenantDenied(t.to_string()))
        }
    }
    pub fn count(&self) -> usize {
        self.allowed.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_denies_all() {
        let p = TenantPolicy::new();
        assert!(p.authorize(Some("acme")).is_err());
        assert!(p.authorize(None).is_err());
    }
    #[test]
    fn allow_then_authorize_succeeds() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        assert_eq!(p.authorize(Some("acme")).unwrap(), "acme");
    }
    #[test]
    fn revoke_drops_grant() {
        let mut p = TenantPolicy::new();
        p.allow("a").unwrap();
        assert!(p.revoke("a"));
        assert!(p.authorize(Some("a")).is_err());
    }
    #[test]
    fn validate_simple_ok() {
        assert!(validate_tenant_id("acme").is_ok());
    }
    #[test]
    fn validate_uppercase_fails() {
        assert!(validate_tenant_id("Acme").is_err());
    }
    #[test]
    fn validate_long_fails() {
        assert!(validate_tenant_id(&"a".repeat(64)).is_err());
    }
    #[test]
    fn validate_leading_hyphen_fails() {
        assert!(validate_tenant_id("-x").is_err());
    }
    #[test]
    fn allow_invalid_id_does_not_insert() {
        let mut p = TenantPolicy::new();
        let _ = p.allow("BAD");
        assert_eq!(p.count(), 0);
    }
}
