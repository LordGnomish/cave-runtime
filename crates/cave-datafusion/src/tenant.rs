//! Tenant invariant — every plan + batch is scoped per tenant.
//!
//! Mirrors cave-iceberg/src/tenant.rs (RFC 1123 DNS label).

use crate::error::{DataFusionError, DfResult};

pub const DEFAULT_TENANT_ID: &str = "default";

pub fn default_tenant_id() -> String {
    DEFAULT_TENANT_ID.to_string()
}

pub fn validate_tenant_id(s: &str) -> DfResult<()> {
    if s.is_empty() {
        return Err(DataFusionError::InvalidTenant("empty".into()));
    }
    if s.len() > 63 {
        return Err(DataFusionError::InvalidTenant(format!("length {} > 63", s.len())));
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err(DataFusionError::InvalidTenant(
            "must not start or end with '-'".into(),
        ));
    }
    for ch in s.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
        if !ok {
            return Err(DataFusionError::InvalidTenant(format!("invalid char '{}'", ch)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_default() {
        // citation: cave-iceberg tenant.rs DEFAULT_TENANT_ID
        assert_eq!(default_tenant_id(), "default");
    }

    #[test]
    fn validate_default_ok() {
        assert!(validate_tenant_id("default").is_ok());
    }

    #[test]
    fn validate_simple_ok() {
        assert!(validate_tenant_id("burak").is_ok());
    }

    #[test]
    fn validate_empty_err() {
        assert!(validate_tenant_id("").is_err());
    }

    #[test]
    fn validate_uppercase_err() {
        assert!(validate_tenant_id("Burak").is_err());
    }

    #[test]
    fn validate_too_long_err() {
        let s: String = "a".repeat(64);
        assert!(validate_tenant_id(&s).is_err());
    }
}
