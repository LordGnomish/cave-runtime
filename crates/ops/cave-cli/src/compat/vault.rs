// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl vault …` — Vault / OpenBao compat shim.
//!
//! Targets `/api/compat/vault/v1/...`. Cave's cave-vault crate owns
//! the actual KV/PKI/transit/SSH/TOTP backends; the shim translates
//! Vault's CLI shape into routed requests.

use anyhow::{Result, bail};
use clap::Subcommand;
use serde_json::{Value, json};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum VaultCmd {
    /// `vault read <path>`
    Read { path: String },
    /// `vault write <path> key=val ...`
    Write {
        path: String,
        #[arg(num_args = 1..)]
        kv: Vec<String>,
    },
    /// `vault list <path>`
    List { path: String },
    /// `vault delete <path>`
    Delete { path: String },
    /// `vault kv` subcommands.
    Kv {
        #[command(subcommand)]
        cmd: KvCmd,
    },
    /// `vault auth list|enable|disable`
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },
    /// `vault secrets list|enable|disable`
    Secrets {
        #[command(subcommand)]
        cmd: SecretsCmd,
    },
    /// `vault policy ...`
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    /// `vault token create|lookup|revoke`
    Token {
        #[command(subcommand)]
        cmd: TokenCmd,
    },
    /// `vault login -method=...`
    Login {
        #[arg(long = "method", default_value = "token")]
        method: String,
        #[arg(num_args = 0..)]
        params: Vec<String>,
    },
    /// `vault status`
    Status,
}

#[derive(Subcommand, Debug, Clone)]
pub enum KvCmd {
    Get {
        path: String,
        #[arg(long)]
        version: Option<u32>,
    },
    Put {
        path: String,
        #[arg(num_args = 1..)]
        kv: Vec<String>,
    },
    Delete {
        path: String,
        #[arg(long, num_args = 1..)]
        versions: Vec<u32>,
    },
    Undelete {
        path: String,
        #[arg(long, num_args = 1..)]
        versions: Vec<u32>,
    },
    Destroy {
        path: String,
        #[arg(long, num_args = 1..)]
        versions: Vec<u32>,
    },
    List {
        path: String,
    },
    Metadata {
        path: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthCmd {
    List,
    Enable {
        #[arg(long)]
        path: Option<String>,
        kind: String,
    },
    Disable {
        path: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum SecretsCmd {
    List,
    Enable {
        #[arg(long)]
        path: Option<String>,
        kind: String,
    },
    Disable {
        path: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PolicyCmd {
    Read { name: String },
    Write { name: String, file: String },
    List,
    Delete { name: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum TokenCmd {
    Create {
        #[arg(long)]
        ttl: Option<String>,
        #[arg(long, num_args = 0..)]
        policy: Vec<String>,
    },
    Lookup {
        #[arg(default_value = "self")]
        token: String,
    },
    Revoke {
        token: String,
    },
}

const AUTH_KINDS: &[&str] = &["token", "userpass", "approle", "kubernetes", "ldap", "oidc"];
const SECRETS_KINDS: &[&str] = &[
    "kv", "pki", "transit", "ssh", "totp", "database", "aws", "azure", "gcp",
];
const LOGIN_METHODS: &[&str] = &["token", "userpass", "approle", "kubernetes", "oidc"];

pub fn prepare(cmd: &VaultCmd) -> Result<PreparedRequest> {
    match cmd {
        VaultCmd::Read { path } => Ok(PreparedRequest::new(HttpVerb::Get, vault_path(path))),
        VaultCmd::Write { path, kv } => {
            let body = parse_kv(kv)?;
            Ok(PreparedRequest::new(HttpVerb::Put, vault_path(path)).with_body(body))
        }
        VaultCmd::List { path } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("{}?list=true", vault_path(path)),
        )),
        VaultCmd::Delete { path } => Ok(PreparedRequest::new(HttpVerb::Delete, vault_path(path))),
        VaultCmd::Kv { cmd } => prepare_kv(cmd),
        VaultCmd::Auth { cmd } => prepare_auth(cmd),
        VaultCmd::Secrets { cmd } => prepare_secrets(cmd),
        VaultCmd::Policy { cmd } => prepare_policy(cmd),
        VaultCmd::Token { cmd } => prepare_token(cmd),
        VaultCmd::Login { method, params } => {
            if !LOGIN_METHODS.contains(&method.as_str()) {
                bail!(
                    "unknown auth method `{}`; want one of {:?}",
                    method,
                    LOGIN_METHODS
                );
            }
            let kvs = parse_kv(params).unwrap_or_else(|_| json!({}));
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/auth/{}/login", method),
            )
            .with_body(kvs))
        }
        VaultCmd::Status => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/vault/v1/sys/health",
        )),
    }
}

fn prepare_kv(cmd: &KvCmd) -> Result<PreparedRequest> {
    match cmd {
        KvCmd::Get { path, version } => {
            let mut p = format!(
                "/api/compat/vault/v1/{}/data/{}",
                split_mount(path)?.0,
                split_mount(path)?.1
            );
            if let Some(v) = version {
                p.push_str(&format!("?version={}", v));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, p))
        }
        KvCmd::Put { path, kv } => {
            let (mount, rest) = split_mount(path)?;
            let body = json!({"data": parse_kv(kv)?});
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/{}/data/{}", mount, rest),
            )
            .with_body(body))
        }
        KvCmd::Delete { path, versions } => {
            let (mount, rest) = split_mount(path)?;
            if versions.is_empty() {
                return Ok(PreparedRequest::new(
                    HttpVerb::Delete,
                    format!("/api/compat/vault/v1/{}/data/{}", mount, rest),
                ));
            }
            let body = json!({"versions": versions});
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/{}/delete/{}", mount, rest),
            )
            .with_body(body))
        }
        KvCmd::Undelete { path, versions } => {
            let (mount, rest) = split_mount(path)?;
            if versions.is_empty() {
                bail!("at least one version required");
            }
            let body = json!({"versions": versions});
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/{}/undelete/{}", mount, rest),
            )
            .with_body(body))
        }
        KvCmd::Destroy { path, versions } => {
            let (mount, rest) = split_mount(path)?;
            if versions.is_empty() {
                bail!("at least one version required");
            }
            let body = json!({"versions": versions});
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/{}/destroy/{}", mount, rest),
            )
            .with_body(body))
        }
        KvCmd::List { path } => {
            let (mount, rest) = split_mount(path)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/vault/v1/{}/metadata/{}?list=true", mount, rest),
            ))
        }
        KvCmd::Metadata { path } => {
            let (mount, rest) = split_mount(path)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/vault/v1/{}/metadata/{}", mount, rest),
            ))
        }
    }
}

fn prepare_auth(cmd: &AuthCmd) -> Result<PreparedRequest> {
    match cmd {
        AuthCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/vault/v1/sys/auth",
        )),
        AuthCmd::Enable { kind, path } => {
            if !AUTH_KINDS.contains(&kind.as_str()) {
                bail!("unknown auth kind `{}`; want one of {:?}", kind, AUTH_KINDS);
            }
            let mount = path.clone().unwrap_or_else(|| kind.clone());
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/sys/auth/{}", mount),
            )
            .with_body(json!({"type": kind})))
        }
        AuthCmd::Disable { path } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/vault/v1/sys/auth/{}", path),
        )),
    }
}

fn prepare_secrets(cmd: &SecretsCmd) -> Result<PreparedRequest> {
    match cmd {
        SecretsCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/vault/v1/sys/mounts",
        )),
        SecretsCmd::Enable { kind, path } => {
            if !SECRETS_KINDS.contains(&kind.as_str()) {
                bail!(
                    "unknown secrets kind `{}`; want one of {:?}",
                    kind,
                    SECRETS_KINDS
                );
            }
            let mount = path.clone().unwrap_or_else(|| kind.clone());
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/vault/v1/sys/mounts/{}", mount),
            )
            .with_body(json!({"type": kind})))
        }
        SecretsCmd::Disable { path } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/vault/v1/sys/mounts/{}", path),
        )),
    }
}

fn prepare_policy(cmd: &PolicyCmd) -> Result<PreparedRequest> {
    match cmd {
        PolicyCmd::Read { name } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/vault/v1/sys/policy/{}", name),
        )),
        PolicyCmd::Write { name, file } => Ok(PreparedRequest::new(
            HttpVerb::Put,
            format!("/api/compat/vault/v1/sys/policy/{}", name),
        )
        .with_body(json!({"file": file}))),
        PolicyCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/vault/v1/sys/policy",
        )),
        PolicyCmd::Delete { name } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/vault/v1/sys/policy/{}", name),
        )),
    }
}

fn prepare_token(cmd: &TokenCmd) -> Result<PreparedRequest> {
    match cmd {
        TokenCmd::Create { ttl, policy } => {
            let mut body: Value = json!({});
            if let Some(t) = ttl {
                body["ttl"] = json!(t);
            }
            if !policy.is_empty() {
                body["policies"] = json!(policy);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/vault/v1/auth/token/create")
                    .with_body(body),
            )
        }
        TokenCmd::Lookup { token } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/vault/v1/auth/token/lookup/{}", token),
        )),
        TokenCmd::Revoke { token } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            format!("/api/compat/vault/v1/auth/token/revoke/{}", token),
        )
        .with_body(json!({}))),
    }
}

fn vault_path(p: &str) -> String {
    let trimmed = p.trim_start_matches('/');
    format!("/api/compat/vault/v1/{}", trimmed)
}

/// Split a vault KV-v2 path like `secret/app/db` into mount `secret`
/// and the remainder `app/db`. Vault CLI's `kv` subcommand strips the
/// `data/` segment that the HTTP API needs; this helper reinserts it.
pub fn split_mount(path: &str) -> Result<(String, String)> {
    let trimmed = path.trim_start_matches('/');
    let (mount, rest) = trimmed
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("path needs <mount>/<key> form"))?;
    if mount.is_empty() {
        bail!("empty mount");
    }
    Ok((mount.to_string(), rest.to_string()))
}

fn parse_kv(kv: &[String]) -> Result<Value> {
    let mut data = serde_json::Map::new();
    for entry in kv {
        let (k, v) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("expected key=value, got `{}`", entry))?;
        data.insert(k.to_string(), json!(v));
    }
    Ok(Value::Object(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_path() {
        let r = prepare(&VaultCmd::Read {
            path: "secret/data/db".into(),
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert_eq!(r.path, "/api/compat/vault/v1/secret/data/db");
    }

    #[test]
    fn write_kv() {
        let r = prepare(&VaultCmd::Write {
            path: "secret/db".into(),
            kv: vec!["user=admin".into(), "pass=hunter2".into()],
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Put);
        let body = r.body.unwrap();
        assert_eq!(body["user"], "admin");
        assert_eq!(body["pass"], "hunter2");
    }

    #[test]
    fn write_rejects_bad_kv() {
        assert!(
            prepare(&VaultCmd::Write {
                path: "x".into(),
                kv: vec!["no_equals".into()],
            })
            .is_err()
        );
    }

    #[test]
    fn list_appends_query() {
        let r = prepare(&VaultCmd::List {
            path: "secret/".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("?list=true"));
    }

    #[test]
    fn delete_path() {
        let r = prepare(&VaultCmd::Delete {
            path: "secret/x".into(),
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn kv_get_v2_path() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::Get {
                path: "secret/app/db".into(),
                version: None,
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/vault/v1/secret/data/app/db");
    }

    #[test]
    fn kv_get_with_version() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::Get {
                path: "secret/x".into(),
                version: Some(2),
            },
        })
        .unwrap();
        assert!(r.path.contains("version=2"));
    }

    #[test]
    fn kv_put() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::Put {
                path: "secret/x".into(),
                kv: vec!["k=v".into()],
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        let body = r.body.unwrap();
        assert_eq!(body["data"]["k"], "v");
    }

    #[test]
    fn kv_delete_no_versions_uses_delete() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::Delete {
                path: "secret/x".into(),
                versions: vec![],
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn kv_delete_with_versions_uses_post() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::Delete {
                path: "secret/x".into(),
                versions: vec![1, 2],
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.contains("/secret/delete/x"));
    }

    #[test]
    fn kv_undelete_requires_versions() {
        assert!(
            prepare(&VaultCmd::Kv {
                cmd: KvCmd::Undelete {
                    path: "secret/x".into(),
                    versions: vec![],
                },
            })
            .is_err()
        );
    }

    #[test]
    fn kv_destroy_requires_versions() {
        assert!(
            prepare(&VaultCmd::Kv {
                cmd: KvCmd::Destroy {
                    path: "secret/x".into(),
                    versions: vec![],
                },
            })
            .is_err()
        );
    }

    #[test]
    fn kv_list_metadata_endpoint() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::List {
                path: "secret/x".into(),
            },
        })
        .unwrap();
        assert!(r.path.contains("/secret/metadata/x"));
        assert!(r.path.ends_with("?list=true"));
    }

    #[test]
    fn kv_metadata() {
        let r = prepare(&VaultCmd::Kv {
            cmd: KvCmd::Metadata {
                path: "secret/x".into(),
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/vault/v1/secret/metadata/x");
    }

    #[test]
    fn auth_list() {
        let r = prepare(&VaultCmd::Auth { cmd: AuthCmd::List }).unwrap();
        assert_eq!(r.path, "/api/compat/vault/v1/sys/auth");
    }

    #[test]
    fn auth_enable_kinds_round_trip() {
        for k in AUTH_KINDS {
            let r = prepare(&VaultCmd::Auth {
                cmd: AuthCmd::Enable {
                    kind: (*k).into(),
                    path: None,
                },
            });
            assert!(r.is_ok());
        }
    }

    #[test]
    fn auth_enable_rejects_unknown() {
        assert!(
            prepare(&VaultCmd::Auth {
                cmd: AuthCmd::Enable {
                    kind: "voodoo".into(),
                    path: None,
                },
            })
            .is_err()
        );
    }

    #[test]
    fn auth_disable_path() {
        let r = prepare(&VaultCmd::Auth {
            cmd: AuthCmd::Disable {
                path: "userpass".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn secrets_enable_kinds_round_trip() {
        for k in SECRETS_KINDS {
            let r = prepare(&VaultCmd::Secrets {
                cmd: SecretsCmd::Enable {
                    kind: (*k).into(),
                    path: None,
                },
            });
            assert!(r.is_ok());
        }
    }

    #[test]
    fn secrets_enable_rejects_unknown() {
        assert!(
            prepare(&VaultCmd::Secrets {
                cmd: SecretsCmd::Enable {
                    kind: "blockchain".into(),
                    path: None,
                },
            })
            .is_err()
        );
    }

    #[test]
    fn secrets_enable_with_path() {
        let r = prepare(&VaultCmd::Secrets {
            cmd: SecretsCmd::Enable {
                kind: "kv".into(),
                path: Some("kv-app".into()),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/sys/mounts/kv-app"));
    }

    #[test]
    fn secrets_disable() {
        let r = prepare(&VaultCmd::Secrets {
            cmd: SecretsCmd::Disable {
                path: "kv-app".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn policy_read() {
        let r = prepare(&VaultCmd::Policy {
            cmd: PolicyCmd::Read {
                name: "admin".into(),
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/vault/v1/sys/policy/admin");
    }

    #[test]
    fn policy_write() {
        let r = prepare(&VaultCmd::Policy {
            cmd: PolicyCmd::Write {
                name: "admin".into(),
                file: "policies/admin.hcl".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Put);
        assert_eq!(r.body.unwrap()["file"], "policies/admin.hcl");
    }

    #[test]
    fn policy_list() {
        let r = prepare(&VaultCmd::Policy {
            cmd: PolicyCmd::List,
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn policy_delete() {
        let r = prepare(&VaultCmd::Policy {
            cmd: PolicyCmd::Delete {
                name: "admin".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn token_create_with_policies() {
        let r = prepare(&VaultCmd::Token {
            cmd: TokenCmd::Create {
                ttl: Some("1h".into()),
                policy: vec!["admin".into(), "ops".into()],
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["ttl"], "1h");
        assert_eq!(body["policies"][0], "admin");
    }

    #[test]
    fn token_lookup_default_self() {
        let r = prepare(&VaultCmd::Token {
            cmd: TokenCmd::Lookup {
                token: "self".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/lookup/self"));
    }

    #[test]
    fn token_revoke() {
        let r = prepare(&VaultCmd::Token {
            cmd: TokenCmd::Revoke {
                token: "abc".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/revoke/abc"));
    }

    #[test]
    fn login_method_round_trip() {
        for m in LOGIN_METHODS {
            let r = prepare(&VaultCmd::Login {
                method: (*m).into(),
                params: vec![],
            });
            assert!(r.is_ok());
        }
    }

    #[test]
    fn login_rejects_unknown_method() {
        assert!(
            prepare(&VaultCmd::Login {
                method: "telepathy".into(),
                params: vec![],
            })
            .is_err()
        );
    }

    #[test]
    fn login_with_params() {
        let r = prepare(&VaultCmd::Login {
            method: "userpass".into(),
            params: vec!["username=alice".into(), "password=secret".into()],
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["username"], "alice");
    }

    #[test]
    fn status() {
        let r = prepare(&VaultCmd::Status).unwrap();
        assert_eq!(r.path, "/api/compat/vault/v1/sys/health");
    }

    #[test]
    fn split_mount_simple() {
        let (m, r) = split_mount("secret/app/db").unwrap();
        assert_eq!(m, "secret");
        assert_eq!(r, "app/db");
    }

    #[test]
    fn split_mount_strips_leading_slash() {
        let (m, _) = split_mount("/secret/x").unwrap();
        assert_eq!(m, "secret");
    }

    #[test]
    fn split_mount_rejects_no_slash() {
        assert!(split_mount("secretonly").is_err());
    }

    #[test]
    fn paths_use_compat_vault_prefix() {
        let r = prepare(&VaultCmd::Status).unwrap();
        assert!(r.path.starts_with("/api/compat/vault/v1/"));
    }

    // ── plugin catalog ──────────────────────────────────────────────────────

    #[test]
    fn plugin_list_path() {
        let r = prepare(&VaultCmd::Plugin {
            cmd: PluginCmd::List {
                plugin_type: "secret".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert_eq!(r.path, "/api/compat/vault/v1/sys/plugins/catalog/secret");
    }

    #[test]
    fn plugin_info_path() {
        let r = prepare(&VaultCmd::Plugin {
            cmd: PluginCmd::Info {
                plugin_type: "database".into(),
                name: "mysql".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
        assert_eq!(
            r.path,
            "/api/compat/vault/v1/sys/plugins/catalog/database/mysql"
        );
    }

    #[test]
    fn plugin_register_posts_body() {
        let r = prepare(&VaultCmd::Plugin {
            cmd: PluginCmd::Register {
                plugin_type: "secret".into(),
                name: "vault-plugin-foo".into(),
                command: "foo".into(),
                sha256: "a".repeat(64),
                version: Some("1.2.0".into()),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(
            r.path,
            "/api/compat/vault/v1/sys/plugins/catalog/secret/vault-plugin-foo"
        );
        let body = r.body.unwrap();
        assert_eq!(body["command"], "foo");
        assert_eq!(body["sha256"], "a".repeat(64));
        assert_eq!(body["version"], "1.2.0");
    }

    #[test]
    fn plugin_register_rejects_unknown_type() {
        assert!(prepare(&VaultCmd::Plugin {
            cmd: PluginCmd::Register {
                plugin_type: "widget".into(),
                name: "x".into(),
                command: "x".into(),
                sha256: "a".repeat(64),
                version: None,
            },
        })
        .is_err());
    }

    #[test]
    fn plugin_deregister_deletes() {
        let r = prepare(&VaultCmd::Plugin {
            cmd: PluginCmd::Deregister {
                plugin_type: "auth".into(),
                name: "vault-plugin-bar".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
        assert!(r.path.ends_with("/sys/plugins/catalog/auth/vault-plugin-bar"));
    }
}
