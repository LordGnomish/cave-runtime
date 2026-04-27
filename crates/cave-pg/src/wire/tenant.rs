//! Per-tenant isolation invariant — **default deny**.
//!
//! Mirrors `cave-iceberg::tenant`: tenant ids are RFC 1123 DNS labels (1-63
//! ASCII chars, lowercase + digit + hyphen, no leading/trailing hyphen). The
//! [`TenantPolicy`] is constructed empty (no tenants allowed) and callers
//! must explicitly grant access. A connection without a tenant binding fails
//! [`TenantPolicy::authorize`].

use std::collections::BTreeSet;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TenantError {
    #[error("invalid tenant id: {0}")]
    Invalid(String),
    #[error("tenant denied: {0}")]
    Denied(String),
}

pub type TenantResult<T> = Result<T, TenantError>;

pub const DEFAULT_TENANT_ID: &str = "default";

pub fn validate_tenant_id(s: &str) -> TenantResult<()> {
    if s.is_empty() {
        return Err(TenantError::Invalid("empty".into()));
    }
    if s.len() > 63 {
        return Err(TenantError::Invalid(format!("length {} > 63", s.len())));
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

/// Default-deny tenant ACL.
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
            // Return a stable reference (same string from the input — interned via the input).
            // Callers can use this to thread the tenant id through downstream logging.
            Ok(t)
        } else {
            Err(TenantError::Denied(t.to_string()))
        }
    }

    pub fn allowed(&self) -> impl Iterator<Item = &str> {
        self.allowed.iter().map(String::as_str)
    }

    pub fn count(&self) -> usize {
        self.allowed.len()
    }
}

/// Extract the tenant id from a startup message's parameter list. PostgreSQL
/// allows arbitrary `option=value` pairs via the `options` GUC; cave-pg uses
/// a dedicated `cave_tenant` key as the canonical binding, falling back to a
/// `cave-tenant=` substring inside `options`.
pub fn extract_tenant<'a>(parameters: &'a [(String, String)]) -> Option<&'a str> {
    for (k, v) in parameters {
        if k == "cave_tenant" {
            return Some(v.as_str());
        }
    }
    for (k, v) in parameters {
        if k == "options" {
            for token in v.split_whitespace() {
                if let Some(rest) = token.strip_prefix("--cave-tenant=") {
                    return Some(rest);
                }
                if let Some(rest) = token.strip_prefix("-c") {
                    if let Some(rest) = rest.strip_prefix("cave_tenant=") {
                        return Some(rest);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_simple_ok() {
        assert!(validate_tenant_id("acme").is_ok());
    }

    #[test]
    fn validate_with_hyphen_ok() {
        assert!(validate_tenant_id("acme-prod").is_ok());
    }

    #[test]
    fn validate_with_digits_ok() {
        assert!(validate_tenant_id("tenant-007").is_ok());
    }

    #[test]
    fn validate_empty_errs() {
        assert!(validate_tenant_id("").is_err());
    }

    #[test]
    fn validate_uppercase_errs() {
        assert!(validate_tenant_id("Acme").is_err());
    }

    #[test]
    fn validate_underscore_errs() {
        assert!(validate_tenant_id("a_b").is_err());
    }

    #[test]
    fn validate_too_long_errs() {
        let s: String = "a".repeat(64);
        assert!(validate_tenant_id(&s).is_err());
    }

    #[test]
    fn validate_leading_hyphen_errs() {
        assert!(validate_tenant_id("-x").is_err());
    }

    #[test]
    fn validate_trailing_hyphen_errs() {
        assert!(validate_tenant_id("x-").is_err());
    }

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
    fn missing_binding_denied() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        let err = p.authorize(None).unwrap_err();
        assert_eq!(err, TenantError::Denied("no tenant binding".into()));
    }

    #[test]
    fn unknown_tenant_denied() {
        let mut p = TenantPolicy::new();
        p.allow("acme").unwrap();
        let err = p.authorize(Some("evil")).unwrap_err();
        assert_eq!(err, TenantError::Denied("evil".into()));
    }

    #[test]
    fn revoke_removes_grant() {
        let mut p = TenantPolicy::new();
        p.allow("a").unwrap();
        assert!(p.revoke("a"));
        assert!(p.authorize(Some("a")).is_err());
    }

    #[test]
    fn allow_invalid_id_errors_without_inserting() {
        let mut p = TenantPolicy::new();
        let err = p.allow("-bad").unwrap_err();
        assert!(matches!(err, TenantError::Invalid(_)));
        assert_eq!(p.count(), 0);
    }

    #[test]
    fn extract_tenant_explicit_key() {
        let params = vec![
            ("user".into(), "alice".into()),
            ("cave_tenant".into(), "acme".into()),
        ];
        assert_eq!(extract_tenant(&params), Some("acme"));
    }

    #[test]
    fn extract_tenant_via_options_long() {
        let params = vec![("options".into(), "--cave-tenant=widgets".into())];
        assert_eq!(extract_tenant(&params), Some("widgets"));
    }

    #[test]
    fn extract_tenant_via_options_set() {
        let params = vec![("options".into(), "-ccave_tenant=stuff".into())];
        assert_eq!(extract_tenant(&params), Some("stuff"));
    }

    #[test]
    fn extract_tenant_missing_returns_none() {
        let params = vec![("user".into(), "alice".into())];
        assert_eq!(extract_tenant(&params), None);
    }

    #[test]
    fn explicit_key_wins_over_options() {
        let params = vec![
            ("cave_tenant".into(), "first".into()),
            ("options".into(), "--cave-tenant=second".into()),
        ];
        assert_eq!(extract_tenant(&params), Some("first"));
    }

    #[test]
    fn allowed_iterator_lists_all() {
        let mut p = TenantPolicy::new();
        p.allow("alpha").unwrap();
        p.allow("beta").unwrap();
        let all: Vec<&str> = p.allowed().collect();
        assert_eq!(all, vec!["alpha", "beta"]);
    }
}
