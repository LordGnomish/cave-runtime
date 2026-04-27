//! Per-tenant isolation for cave-cache — default-deny.
//!
//! Mirrors `cave-pg::wire::tenant`. Connections must present a `cave_tenant`
//! attribute (carried in the HELLO AUTH options for RESP3 clients, or via the
//! `CLIENT SETINFO cave-tenant <id>` extension for RESP2 fallback). Without
//! a binding, every command is denied by [`TenantPolicy::authorize`].

use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TenantError {
    #[error("invalid tenant id: {0}")]
    Invalid(String),
    #[error("tenant denied: {0}")]
    Denied(String),
}

pub type TenantResult<T> = Result<T, TenantError>;

pub fn validate_tenant_id(s: &str) -> TenantResult<()> {
    if s.is_empty() {
        return Err(TenantError::Invalid("empty".into()));
    }
    if s.len() > 63 {
        return Err(TenantError::Invalid("too long".into()));
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

#[derive(Debug, Clone, Default)]
pub struct TenantPolicy {
    allowed: BTreeSet<String>,
}

impl TenantPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow(&mut self, tenant: &str) -> TenantResult<()> {
        validate_tenant_id(tenant)?;
        self.allowed.insert(tenant.to_string());
        Ok(())
    }

    pub fn revoke(&mut self, tenant: &str) -> bool {
        self.allowed.remove(tenant)
    }

    pub fn authorize<'a>(&self, tenant: Option<&'a str>) -> TenantResult<&'a str> {
        let t = tenant.ok_or_else(|| TenantError::Denied("no tenant binding".into()))?;
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

/// Build the keyspace prefix Redis should apply for the given tenant. Mirrors
/// the AWS ElastiCache "namespace" concept and prevents key collision across
/// tenants on a shared instance.
pub fn keyspace_prefix(tenant: &str) -> String {
    format!("{{t:{}}}:", tenant)
}

/// Strip a tenant prefix from a key. Returns `None` if the key lacks the
/// expected `{t:...}` prefix.
pub fn strip_prefix<'a>(key: &'a [u8]) -> Option<(&'a str, &'a [u8])> {
    if key.first() != Some(&b'{') {
        return None;
    }
    let close = key.iter().position(|&b| b == b'}')?;
    if close < 4 || &key[1..3] != b"t:" {
        return None;
    }
    let tenant = std::str::from_utf8(&key[3..close]).ok()?;
    if key.len() < close + 2 || key[close + 1] != b':' {
        return None;
    }
    Some((tenant, &key[close + 2..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_denies() {
        let p = TenantPolicy::new();
        assert!(p.authorize(Some("acme")).is_err());
    }
    #[test]
    fn allow_then_authorize() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        assert_eq!(p.authorize(Some("acme")).unwrap(), "acme");
    }
    #[test]
    fn no_binding_denied() {
        let p = TenantPolicy::new();
        assert!(p.authorize(None).is_err());
    }
    #[test]
    fn invalid_id_rejected_on_allow() {
        let mut p = TenantPolicy::new();
        assert!(p.allow("BAD").is_err());
        assert_eq!(p.count(), 0);
    }
    #[test]
    fn revoke_removes_grant() {
        let mut p = TenantPolicy::new();
        p.allow("a").unwrap();
        assert!(p.revoke("a"));
        assert!(p.authorize(Some("a")).is_err());
    }
    #[test]
    fn keyspace_prefix_format() {
        assert_eq!(keyspace_prefix("acme"), "{t:acme}:");
    }
    #[test]
    fn strip_prefix_returns_tenant_and_key() {
        let key = b"{t:acme}:hello";
        let (tenant, rest) = strip_prefix(key).unwrap();
        assert_eq!(tenant, "acme");
        assert_eq!(rest, b"hello");
    }
    #[test]
    fn strip_prefix_returns_none_for_unprefixed() {
        assert!(strip_prefix(b"plain-key").is_none());
    }
    #[test]
    fn strip_prefix_returns_none_for_other_tag() {
        assert!(strip_prefix(b"{n:foo}:bar").is_none());
    }
    #[test]
    fn validate_simple_ok() {
        assert!(validate_tenant_id("ok").is_ok());
    }
    #[test]
    fn validate_uppercase_fails() {
        assert!(validate_tenant_id("Up").is_err());
    }
    #[test]
    fn validate_leading_hyphen_fails() {
        assert!(validate_tenant_id("-x").is_err());
    }
    #[test]
    fn validate_long_fails() {
        assert!(validate_tenant_id(&"a".repeat(64)).is_err());
    }
}
