// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl flag toggle/set/unset/list/group`
//!
//! Native flag verb. Per ADR-RUNTIME-CLI-CONSOLIDATION-001, `flag`
//! is the canonical Cave verb (singular). The legacy `flags` plural
//! command remains as a compat alias in main.rs but new code targets
//! this module.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use super::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum FlagCmd {
    /// Toggle a flag (on if off, off if on).
    Toggle {
        key: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// Set a flag — value is `on`, `off`, or `<n>%`.
    Set {
        key: String,
        value: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// Unset (delete) a flag.
    Unset {
        key: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// List flags.
    List {
        #[arg(short = 't', long)]
        tenant: Option<String>,
        #[arg(long)]
        group: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlagValue {
    On,
    Off,
    Percentage(u8),
}

impl FlagValue {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "on" | "true" | "enabled" => Ok(FlagValue::On),
            "off" | "false" | "disabled" => Ok(FlagValue::Off),
            other if other.ends_with('%') => {
                let n: u8 = other.trim_end_matches('%').parse().map_err(|_| {
                    anyhow::anyhow!("invalid percentage `{}` (want 0..=100)", s)
                })?;
                if n > 100 {
                    bail!("percentage out of range: {}", n);
                }
                Ok(FlagValue::Percentage(n))
            }
            _ => bail!("unrecognised flag value `{}`", s),
        }
    }

    pub fn to_body(&self) -> Value {
        match self {
            FlagValue::On => json!({"enabled": true}),
            FlagValue::Off => json!({"enabled": false}),
            FlagValue::Percentage(n) => json!({"rollout": n}),
        }
    }
}

pub fn prepare(cmd: &FlagCmd) -> Result<PreparedRequest> {
    match cmd {
        FlagCmd::Toggle { key, tenant } => {
            validate_key(key)?;
            Ok(
                PreparedRequest::new(HttpVerb::Post, scoped_key(key, tenant.as_deref(), "toggle"))
                    .with_body(json!({})),
            )
        }
        FlagCmd::Set { key, value, tenant } => {
            validate_key(key)?;
            let body = FlagValue::parse(value)?.to_body();
            Ok(PreparedRequest::new(HttpVerb::Put, scoped_key(key, tenant.as_deref(), ""))
                .with_body(body))
        }
        FlagCmd::Unset { key, tenant } => {
            validate_key(key)?;
            Ok(PreparedRequest::new(
                HttpVerb::Delete,
                scoped_key(key, tenant.as_deref(), ""),
            ))
        }
        FlagCmd::List { tenant, group } => {
            let mut path = match tenant.as_deref() {
                Some(t) => format!("/api/native/tenants/{}/flags", t),
                None => "/api/native/flags".to_string(),
            };
            if let Some(g) = group {
                path.push_str(&format!("?group={}", g));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
    }
}

fn scoped_key(key: &str, tenant: Option<&str>, suffix: &str) -> String {
    let base = match tenant {
        Some(t) => format!("/api/native/tenants/{}/flags/{}", t, key),
        None => format!("/api/native/flags/{}", key),
    };
    if suffix.is_empty() {
        base
    } else {
        format!("{}/{}", base, suffix)
    }
}

fn validate_key(k: &str) -> Result<()> {
    if k.is_empty() {
        bail!("flag key required");
    }
    if k.len() > 256 {
        bail!("flag key too long (max 256)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle() {
        let r = prepare(&FlagCmd::Toggle {
            key: "billing.v2".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/native/flags/billing.v2/toggle");
    }

    #[test]
    fn toggle_with_tenant() {
        let r = prepare(&FlagCmd::Toggle {
            key: "x".into(),
            tenant: Some("acme".into()),
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/flags/x/toggle");
    }

    #[test]
    fn set_on() {
        let r = prepare(&FlagCmd::Set {
            key: "x".into(),
            value: "on".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Put);
        assert_eq!(r.body.unwrap(), json!({"enabled": true}));
    }

    #[test]
    fn set_off() {
        let r = prepare(&FlagCmd::Set {
            key: "x".into(),
            value: "off".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["enabled"], false);
    }

    #[test]
    fn set_percentage() {
        let r = prepare(&FlagCmd::Set {
            key: "x".into(),
            value: "25%".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["rollout"], 25);
    }

    #[test]
    fn set_with_tenant() {
        let r = prepare(&FlagCmd::Set {
            key: "x".into(),
            value: "on".into(),
            tenant: Some("acme".into()),
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/flags/x");
    }

    #[test]
    fn unset() {
        let r = prepare(&FlagCmd::Unset {
            key: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn list_no_group() {
        let r = prepare(&FlagCmd::List {
            tenant: None,
            group: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/flags");
    }

    #[test]
    fn list_with_group() {
        let r = prepare(&FlagCmd::List {
            tenant: None,
            group: Some("checkout".into()),
        })
        .unwrap();
        assert!(r.path.contains("group=checkout"));
    }

    #[test]
    fn list_with_tenant() {
        let r = prepare(&FlagCmd::List {
            tenant: Some("acme".into()),
            group: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/flags");
    }

    #[test]
    fn list_uses_get() {
        let r = prepare(&FlagCmd::List {
            tenant: None,
            group: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(FlagValue::parse("true").unwrap(), FlagValue::On);
        assert_eq!(FlagValue::parse("ENABLED").unwrap(), FlagValue::On);
        assert_eq!(FlagValue::parse("disabled").unwrap(), FlagValue::Off);
    }

    #[test]
    fn parse_zero_pct() {
        assert_eq!(FlagValue::parse("0%").unwrap(), FlagValue::Percentage(0));
    }

    #[test]
    fn parse_full_pct() {
        assert_eq!(FlagValue::parse("100%").unwrap(), FlagValue::Percentage(100));
    }

    #[test]
    fn parse_rejects_over_100() {
        assert!(FlagValue::parse("200%").is_err());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(FlagValue::parse("maybe").is_err());
    }

    #[test]
    fn parse_rejects_empty() {
        assert!(FlagValue::parse("").is_err());
    }

    #[test]
    fn to_body_on() {
        assert_eq!(FlagValue::On.to_body(), json!({"enabled": true}));
    }

    #[test]
    fn to_body_off() {
        assert_eq!(FlagValue::Off.to_body(), json!({"enabled": false}));
    }

    #[test]
    fn to_body_pct() {
        assert_eq!(FlagValue::Percentage(50).to_body(), json!({"rollout": 50}));
    }

    #[test]
    fn toggle_rejects_empty_key() {
        assert!(prepare(&FlagCmd::Toggle {
            key: "".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn set_rejects_oversized_key() {
        assert!(prepare(&FlagCmd::Set {
            key: "x".repeat(257),
            value: "on".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn set_rejects_bad_value() {
        assert!(prepare(&FlagCmd::Set {
            key: "x".into(),
            value: "yolo".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn unset_uses_delete_method() {
        let r = prepare(&FlagCmd::Unset {
            key: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn unset_no_body() {
        let r = prepare(&FlagCmd::Unset {
            key: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn toggle_body_empty_obj() {
        let r = prepare(&FlagCmd::Toggle {
            key: "x".into(),
            tenant: None,
        })
        .unwrap();
        let body = r.body.unwrap();
        assert!(body.is_object());
        assert_eq!(body.as_object().unwrap().len(), 0);
    }

    #[test]
    fn set_path_no_suffix() {
        let r = prepare(&FlagCmd::Set {
            key: "x".into(),
            value: "on".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/flags/x");
    }
}
