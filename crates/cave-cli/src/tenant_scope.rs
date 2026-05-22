// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant scope resolution for `cavectl`.
//!
//! Per ADR-RUNTIME-CLI-CONSOLIDATION-001 every native verb carries a
//! tenant. The resolution order is, in priority:
//!   1. explicit `--tenant`/`-t` flag
//!   2. `CAVE_TENANT` env var
//!   3. config file (`~/.config/cavectl/config.toml`, `[default]
//!      tenant`)
//!   4. None — caller decides whether to prompt or default to a
//!      platform-admin scope.
//!
//! This module is the pure resolver. The CLI layer feeds it the
//! current flag/env/config snapshot.

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantSource {
    /// Resolved from `--tenant`/`-t`.
    Flag,
    /// Resolved from `CAVE_TENANT`.
    Env,
    /// Resolved from the config file's `[default] tenant`.
    Config,
    /// No tenant was configured anywhere.
    Unset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTenant {
    pub value: Option<String>,
    pub source: TenantSource,
}

#[derive(Debug, Clone, Default)]
pub struct TenantInputs<'a> {
    pub flag: Option<&'a str>,
    pub env: Option<&'a str>,
    pub config: Option<&'a str>,
}

/// Apply the priority rules and return the resolved tenant.
///
/// Validates the chosen value before returning. An empty-string
/// override at any level is treated as "explicitly clear" — useful
/// for one-off platform-admin invocations without exporting a new env
/// var. Whitespace is rejected.
pub fn resolve(inputs: TenantInputs<'_>) -> Result<ResolvedTenant> {
    if let Some(v) = inputs.flag {
        if v.is_empty() {
            return Ok(ResolvedTenant {
                value: None,
                source: TenantSource::Flag,
            });
        }
        validate_tenant(v)?;
        return Ok(ResolvedTenant {
            value: Some(v.to_string()),
            source: TenantSource::Flag,
        });
    }
    if let Some(v) = inputs.env {
        if v.is_empty() {
            return Ok(ResolvedTenant {
                value: None,
                source: TenantSource::Env,
            });
        }
        validate_tenant(v)?;
        return Ok(ResolvedTenant {
            value: Some(v.to_string()),
            source: TenantSource::Env,
        });
    }
    if let Some(v) = inputs.config {
        if v.is_empty() {
            return Ok(ResolvedTenant {
                value: None,
                source: TenantSource::Config,
            });
        }
        validate_tenant(v)?;
        return Ok(ResolvedTenant {
            value: Some(v.to_string()),
            source: TenantSource::Config,
        });
    }
    Ok(ResolvedTenant {
        value: None,
        source: TenantSource::Unset,
    })
}

/// Tenants follow DNS-1123 label rules: lowercase, alnum + `-`, must
/// start and end alnum, max 63 chars.
pub fn validate_tenant(v: &str) -> Result<()> {
    if v.is_empty() {
        bail!("tenant cannot be empty");
    }
    if v.len() > 63 {
        bail!("tenant too long (max 63)");
    }
    let bytes = v.as_bytes();
    if !is_alnum_lower(bytes[0]) || !is_alnum_lower(bytes[bytes.len() - 1]) {
        bail!("tenant must start and end with alphanumeric");
    }
    for &b in bytes {
        if !(is_alnum_lower(b) || b == b'-') {
            bail!("tenant allows only [a-z0-9-]");
        }
    }
    Ok(())
}

fn is_alnum_lower(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'0'..=b'9')
}

/// Parse a `[default] tenant = "..."` value out of a TOML-ish config.
///
/// Tiny purpose-built parser: looks for the line under `[default]`
/// header and extracts the quoted string. Avoids pulling a full TOML
/// dep here — the broader workspace already has `toml`, but this
/// lookup is small enough to keep self-contained and unit-testable.
pub fn parse_config_tenant(toml: &str) -> Option<String> {
    let mut in_default = false;
    for raw in toml.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            in_default = rest.trim_end_matches(']').trim() == "default";
            continue;
        }
        if !in_default {
            continue;
        }
        if let Some(rest) = line.strip_prefix("tenant") {
            // Match `tenant = "x"` or `tenant="x"`.
            let after_eq = rest.split_once('=').map(|(_, r)| r.trim())?;
            // Strip optional quotes; preserve raw value for tests.
            let v = after_eq
                .trim_start_matches('"')
                .trim_end_matches('"')
                .trim_start_matches('\'')
                .trim_end_matches('\'');
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(
        flag: Option<&'a str>,
        env: Option<&'a str>,
        config: Option<&'a str>,
    ) -> TenantInputs<'a> {
        TenantInputs { flag, env, config }
    }

    #[test]
    fn unset_when_all_none() {
        let r = resolve(inputs(None, None, None)).unwrap();
        assert!(r.value.is_none());
        assert_eq!(r.source, TenantSource::Unset);
    }

    #[test]
    fn flag_wins_over_env_and_config() {
        let r = resolve(inputs(Some("flag"), Some("env"), Some("cfg"))).unwrap();
        assert_eq!(r.value.as_deref(), Some("flag"));
        assert_eq!(r.source, TenantSource::Flag);
    }

    #[test]
    fn env_wins_over_config() {
        let r = resolve(inputs(None, Some("env"), Some("cfg"))).unwrap();
        assert_eq!(r.value.as_deref(), Some("env"));
        assert_eq!(r.source, TenantSource::Env);
    }

    #[test]
    fn config_only() {
        let r = resolve(inputs(None, None, Some("cfg"))).unwrap();
        assert_eq!(r.value.as_deref(), Some("cfg"));
        assert_eq!(r.source, TenantSource::Config);
    }

    #[test]
    fn empty_flag_clears() {
        let r = resolve(inputs(Some(""), Some("env"), Some("cfg"))).unwrap();
        assert!(r.value.is_none());
        assert_eq!(r.source, TenantSource::Flag);
    }

    #[test]
    fn empty_env_clears() {
        let r = resolve(inputs(None, Some(""), Some("cfg"))).unwrap();
        assert!(r.value.is_none());
        assert_eq!(r.source, TenantSource::Env);
    }

    #[test]
    fn empty_config_clears() {
        let r = resolve(inputs(None, None, Some(""))).unwrap();
        assert!(r.value.is_none());
        assert_eq!(r.source, TenantSource::Config);
    }

    #[test]
    fn rejects_invalid_flag() {
        assert!(resolve(inputs(Some("Bad!"), None, None)).is_err());
    }

    #[test]
    fn rejects_invalid_env() {
        assert!(resolve(inputs(None, Some("Bad!"), None)).is_err());
    }

    #[test]
    fn rejects_invalid_config() {
        assert!(resolve(inputs(None, None, Some("Bad!"))).is_err());
    }

    #[test]
    fn validate_accepts_dns_label() {
        assert!(validate_tenant("acme").is_ok());
        assert!(validate_tenant("a-1-b").is_ok());
        assert!(validate_tenant("0a").is_ok());
    }

    #[test]
    fn validate_rejects_uppercase() {
        assert!(validate_tenant("Acme").is_err());
    }

    #[test]
    fn validate_rejects_underscore() {
        assert!(validate_tenant("a_b").is_err());
    }

    #[test]
    fn validate_rejects_leading_dash() {
        assert!(validate_tenant("-a").is_err());
    }

    #[test]
    fn validate_rejects_trailing_dash() {
        assert!(validate_tenant("a-").is_err());
    }

    #[test]
    fn validate_rejects_too_long() {
        let big = "a".repeat(64);
        assert!(validate_tenant(&big).is_err());
    }

    #[test]
    fn validate_accepts_max_length() {
        let max = "a".repeat(63);
        assert!(validate_tenant(&max).is_ok());
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_tenant("").is_err());
    }

    #[test]
    fn validate_rejects_whitespace() {
        assert!(validate_tenant("a b").is_err());
        assert!(validate_tenant("a\tb").is_err());
    }

    #[test]
    fn parse_config_tenant_basic() {
        let cfg = "[default]\ntenant = \"acme\"\n";
        assert_eq!(parse_config_tenant(cfg), Some("acme".to_string()));
    }

    #[test]
    fn parse_config_tenant_no_quotes() {
        let cfg = "[default]\ntenant = acme\n";
        assert_eq!(parse_config_tenant(cfg), Some("acme".to_string()));
    }

    #[test]
    fn parse_config_tenant_no_spaces() {
        let cfg = "[default]\ntenant=\"acme\"\n";
        assert_eq!(parse_config_tenant(cfg), Some("acme".to_string()));
    }

    #[test]
    fn parse_config_tenant_other_section_ignored() {
        let cfg = "[other]\ntenant = \"x\"\n";
        assert!(parse_config_tenant(cfg).is_none());
    }

    #[test]
    fn parse_config_tenant_default_then_other() {
        let cfg = "[default]\ntenant = \"a\"\n[other]\ntenant = \"b\"\n";
        assert_eq!(parse_config_tenant(cfg), Some("a".to_string()));
    }

    #[test]
    fn parse_config_tenant_skips_comments() {
        let cfg = "# header\n[default]\n# inline\ntenant = \"acme\"\n";
        assert_eq!(parse_config_tenant(cfg), Some("acme".to_string()));
    }

    #[test]
    fn parse_config_tenant_missing_key() {
        let cfg = "[default]\nfoo = 1\n";
        assert!(parse_config_tenant(cfg).is_none());
    }

    #[test]
    fn parse_config_tenant_empty() {
        assert!(parse_config_tenant("").is_none());
    }

    #[test]
    fn parse_config_tenant_first_default_wins() {
        let cfg = "[default]\ntenant = \"a\"\n[default]\ntenant = \"b\"\n";
        assert_eq!(parse_config_tenant(cfg), Some("a".to_string()));
    }

    #[test]
    fn resolve_with_dash_tenant() {
        let r = resolve(inputs(Some("a-b-c"), None, None)).unwrap();
        assert_eq!(r.value.as_deref(), Some("a-b-c"));
    }

    #[test]
    fn config_value_with_dashes() {
        let cfg = "[default]\ntenant = \"a-b-c\"\n";
        assert_eq!(parse_config_tenant(cfg), Some("a-b-c".to_string()));
    }

    #[test]
    fn flag_overrides_with_value() {
        let r = resolve(inputs(Some("override"), Some("from-env"), None)).unwrap();
        assert_eq!(r.value.as_deref(), Some("override"));
    }

    #[test]
    fn end_to_end_resolution_via_config() {
        let cfg = "[default]\ntenant = \"acme\"\n";
        let v = parse_config_tenant(cfg);
        let r = resolve(inputs(None, None, v.as_deref())).unwrap();
        assert_eq!(r.value.as_deref(), Some("acme"));
        assert_eq!(r.source, TenantSource::Config);
    }
}
