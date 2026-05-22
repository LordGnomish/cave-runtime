// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant invariant — every Iceberg metadata object is scoped per tenant.
//!
//! Mirrors the rules in cave-cri's `tenant.rs` (RFC 1123 DNS label).

use crate::table_format::iceberg::error::{IcebergError, IcebergResult};

pub const DEFAULT_TENANT_ID: &str = "default";

pub fn default_tenant_id() -> String {
    DEFAULT_TENANT_ID.to_string()
}

pub fn validate_tenant_id(s: &str) -> IcebergResult<()> {
    if s.is_empty() {
        return Err(IcebergError::InvalidTenant("empty".into()));
    }
    if s.len() > 63 {
        return Err(IcebergError::InvalidTenant(format!(
            "length {} > 63",
            s.len()
        )));
    }
    if s.starts_with('-') || s.ends_with('-') {
        return Err(IcebergError::InvalidTenant(
            "must not start or end with '-'".into(),
        ));
    }
    for ch in s.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-';
        if !ok {
            return Err(IcebergError::InvalidTenant(format!(
                "invalid char '{}'",
                ch
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_default() {
        // citation: cave-cri tenant.rs DEFAULT_TENANT_ID
        assert_eq!(default_tenant_id(), "default");
    }

    #[test]
    fn validate_simple_ok() {
        assert!(validate_tenant_id("acme").is_ok());
    }

    #[test]
    fn validate_with_hyphen_ok() {
        assert!(validate_tenant_id("data-lake").is_ok());
    }

    #[test]
    fn validate_empty_err() {
        assert!(validate_tenant_id("").is_err());
    }

    #[test]
    fn validate_uppercase_err() {
        assert!(validate_tenant_id("Acme").is_err());
    }

    #[test]
    fn validate_underscore_err() {
        assert!(validate_tenant_id("a_b").is_err());
    }

    #[test]
    fn validate_too_long_err() {
        let s: String = "a".repeat(64);
        assert!(validate_tenant_id(&s).is_err());
    }

    #[test]
    fn validate_leading_hyphen_err() {
        assert!(validate_tenant_id("-x").is_err());
    }

    #[test]
    fn validate_trailing_hyphen_err() {
        assert!(validate_tenant_id("x-").is_err());
    }
}
