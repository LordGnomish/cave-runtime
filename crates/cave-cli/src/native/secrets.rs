// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl secrets list/add/rotate/get/delete`
//!
//! Native secrets verb. Backend is cave-vault (OpenBao parity) but
//! Cave-domain rules apply: tenant scope mandatory by construction,
//! audit trail emitted on every read.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::json;

use super::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum SecretsCmd {
    /// List secrets in current tenant.
    List {
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// Get a secret by name.
    Get {
        name: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// Add a new secret.
    Add {
        name: String,
        #[arg(long)]
        value: String,
        #[arg(long, default_value = "generic")]
        kind: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// Rotate a secret — server generates a new value.
    Rotate {
        name: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
    /// Delete a secret.
    Delete {
        name: String,
        #[arg(short = 't', long)]
        tenant: Option<String>,
    },
}

const KINDS: &[&str] = &["generic", "api_key", "db_password", "token", "tls_cert"];

pub fn prepare(cmd: &SecretsCmd) -> Result<PreparedRequest> {
    match cmd {
        SecretsCmd::List { tenant } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            scoped("secrets", tenant.as_deref(), None),
        )),
        SecretsCmd::Get { name, tenant } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                scoped("secrets", tenant.as_deref(), Some(name)),
            ))
        }
        SecretsCmd::Add {
            name,
            value,
            kind,
            tenant,
        } => {
            validate_name(name)?;
            if !KINDS.contains(&kind.as_str()) {
                bail!("unknown secret kind `{}`; want one of {:?}", kind, KINDS);
            }
            if value.is_empty() {
                bail!("value required");
            }
            let body = json!({"name": name, "value": value, "kind": kind});
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                scoped("secrets", tenant.as_deref(), None),
            )
            .with_body(body))
        }
        SecretsCmd::Rotate { name, tenant } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!(
                    "{}/rotate",
                    scoped("secrets", tenant.as_deref(), Some(name))
                ),
            )
            .with_body(json!({})))
        }
        SecretsCmd::Delete { name, tenant } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Delete,
                scoped("secrets", tenant.as_deref(), Some(name)),
            ))
        }
    }
}

fn scoped(resource: &str, tenant: Option<&str>, name: Option<&str>) -> String {
    let base = match tenant {
        Some(t) => format!("/api/native/tenants/{}/{}", t, resource),
        None => format!("/api/native/{}", resource),
    };
    match name {
        Some(n) => format!("{}/{}", base, n),
        None => base,
    }
}

fn validate_name(n: &str) -> Result<()> {
    if n.is_empty() {
        bail!("secret name required");
    }
    if n.len() > 256 {
        bail!("secret name too long (max 256)");
    }
    for ch in n.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' || ch == '/') {
            bail!("secret name allows only alnum, `-`, `_`, `.`, `/`");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_no_tenant() {
        let r = prepare(&SecretsCmd::List { tenant: None }).unwrap();
        assert_eq!(r.path, "/api/native/secrets");
    }

    #[test]
    fn list_with_tenant() {
        let r = prepare(&SecretsCmd::List {
            tenant: Some("acme".into()),
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/secrets");
    }

    #[test]
    fn get_secret() {
        let r = prepare(&SecretsCmd::Get {
            name: "db-pass".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/secrets/db-pass");
    }

    #[test]
    fn get_with_tenant() {
        let r = prepare(&SecretsCmd::Get {
            name: "db-pass".into(),
            tenant: Some("acme".into()),
        })
        .unwrap();
        assert_eq!(r.path, "/api/native/tenants/acme/secrets/db-pass");
    }

    #[test]
    fn add_secret() {
        let r = prepare(&SecretsCmd::Add {
            name: "api-key".into(),
            value: "hunter2".into(),
            kind: "api_key".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/native/secrets");
        let body = r.body.unwrap();
        assert_eq!(body["name"], "api-key");
        assert_eq!(body["value"], "hunter2");
        assert_eq!(body["kind"], "api_key");
    }

    #[test]
    fn add_default_kind_generic() {
        let r = prepare(&SecretsCmd::Add {
            name: "x".into(),
            value: "y".into(),
            kind: "generic".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["kind"], "generic");
    }

    #[test]
    fn add_kinds_round_trip() {
        for k in KINDS {
            let r = prepare(&SecretsCmd::Add {
                name: "x".into(),
                value: "y".into(),
                kind: (*k).into(),
                tenant: None,
            });
            assert!(r.is_ok(), "kind {} should be accepted", k);
        }
    }

    #[test]
    fn add_rejects_unknown_kind() {
        assert!(prepare(&SecretsCmd::Add {
            name: "x".into(),
            value: "y".into(),
            kind: "nuke".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn add_rejects_empty_value() {
        assert!(prepare(&SecretsCmd::Add {
            name: "x".into(),
            value: "".into(),
            kind: "generic".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn rotate() {
        let r = prepare(&SecretsCmd::Rotate {
            name: "db-pass".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/native/secrets/db-pass/rotate");
    }

    #[test]
    fn rotate_with_tenant() {
        let r = prepare(&SecretsCmd::Rotate {
            name: "db-pass".into(),
            tenant: Some("acme".into()),
        })
        .unwrap();
        assert_eq!(
            r.path,
            "/api/native/tenants/acme/secrets/db-pass/rotate"
        );
    }

    #[test]
    fn delete_secret() {
        let r = prepare(&SecretsCmd::Delete {
            name: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
        assert_eq!(r.path, "/api/native/secrets/x");
    }

    #[test]
    fn delete_no_body() {
        let r = prepare(&SecretsCmd::Delete {
            name: "x".into(),
            tenant: None,
        })
        .unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn rotate_empty_body() {
        let r = prepare(&SecretsCmd::Rotate {
            name: "x".into(),
            tenant: None,
        })
        .unwrap();
        let body = r.body.unwrap();
        assert!(body.is_object());
        assert_eq!(body.as_object().unwrap().len(), 0);
    }

    #[test]
    fn list_uses_get() {
        let r = prepare(&SecretsCmd::List { tenant: None }).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn validate_accepts_dot_in_name() {
        let r = prepare(&SecretsCmd::Get {
            name: "kv.app.db".into(),
            tenant: None,
        });
        assert!(r.is_ok());
    }

    #[test]
    fn validate_accepts_path_in_name() {
        let r = prepare(&SecretsCmd::Get {
            name: "kv/app/db".into(),
            tenant: None,
        });
        assert!(r.is_ok());
    }

    #[test]
    fn validate_rejects_empty_name() {
        assert!(prepare(&SecretsCmd::Get {
            name: "".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn validate_rejects_oversized_name() {
        let big = "a".repeat(257);
        assert!(prepare(&SecretsCmd::Get {
            name: big,
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn validate_rejects_special_chars() {
        assert!(prepare(&SecretsCmd::Get {
            name: "bad name!".into(),
            tenant: None,
        })
        .is_err());
    }

    #[test]
    fn add_uses_post() {
        let r = prepare(&SecretsCmd::Add {
            name: "x".into(),
            value: "y".into(),
            kind: "generic".into(),
            tenant: None,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }
}
