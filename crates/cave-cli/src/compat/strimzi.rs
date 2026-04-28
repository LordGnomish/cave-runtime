//! `cavectl strimzi …` — Strimzi (Kafka on Kubernetes) compat shim.
//!
//! Targets `/api/compat/strimzi/v1/...`. Mirrors the most common
//! `kafka-topics`, `kafka-users`, `kafka-consumer-groups`, and
//! `kafka-acls` operations plus Strimzi cluster/connect surfaces.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum StrimziCmd {
    Topics {
        #[command(subcommand)]
        cmd: TopicCmd,
    },
    Users {
        #[command(subcommand)]
        cmd: UserCmd,
    },
    Groups {
        #[command(subcommand)]
        cmd: GroupCmd,
    },
    Acls {
        #[command(subcommand)]
        cmd: AclCmd,
    },
    Cluster {
        #[command(subcommand)]
        cmd: ClusterCmd,
    },
    Connect {
        #[command(subcommand)]
        cmd: ConnectCmd,
    },
    Bridge {
        #[command(subcommand)]
        cmd: BridgeCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum TopicCmd {
    List,
    Create {
        name: String,
        #[arg(long, default_value_t = 1)]
        partitions: u32,
        #[arg(long, default_value_t = 3)]
        replicas: u32,
        #[arg(long, num_args = 0..)]
        config: Vec<String>,
    },
    Delete { name: String },
    Describe { name: String },
    Alter {
        name: String,
        #[arg(long)]
        partitions: Option<u32>,
        #[arg(long, num_args = 0..)]
        config: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum UserCmd {
    List,
    Create {
        name: String,
        /// Authentication: `tls`, `scram-sha-512`.
        #[arg(long, default_value = "scram-sha-512")]
        auth: String,
    },
    Delete { name: String },
    Describe { name: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum GroupCmd {
    List,
    Describe { id: String },
    Delete { id: String },
    /// Reset offsets for a group.
    Reset {
        id: String,
        #[arg(long)]
        topic: String,
        /// `earliest`, `latest`, `to-datetime <iso>`, `to-offset <n>`.
        #[arg(long)]
        to: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AclCmd {
    List {
        #[arg(long)]
        topic: Option<String>,
        #[arg(long)]
        principal: Option<String>,
    },
    Grant {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        resource: String,
        /// `read`, `write`, `describe`, `all`.
        #[arg(long)]
        operation: String,
    },
    Revoke {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        resource: String,
        #[arg(long)]
        operation: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ClusterCmd {
    Status,
    /// Force a rolling restart.
    RollingRestart {
        #[arg(long)]
        component: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConnectCmd {
    Connectors {
        #[command(subcommand)]
        cmd: ConnectorCmd,
    },
    Status,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConnectorCmd {
    List,
    Create {
        name: String,
        #[arg(long)]
        config: String,
    },
    Delete { name: String },
    Pause { name: String },
    Resume { name: String },
    Restart { name: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum BridgeCmd {
    Status,
}

const AUTH_KINDS: &[&str] = &["tls", "scram-sha-512", "scram-sha-256"];
const ACL_OPS: &[&str] = &["read", "write", "describe", "create", "delete", "alter", "all"];
const CLUSTER_COMPONENTS: &[&str] = &["kafka", "zookeeper", "entity-operator", "kraft"];

pub fn prepare(cmd: &StrimziCmd) -> Result<PreparedRequest> {
    match cmd {
        StrimziCmd::Topics { cmd } => prepare_topics(cmd),
        StrimziCmd::Users { cmd } => prepare_users(cmd),
        StrimziCmd::Groups { cmd } => prepare_groups(cmd),
        StrimziCmd::Acls { cmd } => prepare_acls(cmd),
        StrimziCmd::Cluster { cmd } => prepare_cluster(cmd),
        StrimziCmd::Connect { cmd } => prepare_connect(cmd),
        StrimziCmd::Bridge { cmd } => match cmd {
            BridgeCmd::Status => Ok(PreparedRequest::new(
                HttpVerb::Get,
                "/api/compat/strimzi/v1/bridge/status",
            )),
        },
    }
}

fn prepare_topics(cmd: &TopicCmd) -> Result<PreparedRequest> {
    match cmd {
        TopicCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/strimzi/v1/topics",
        )),
        TopicCmd::Create {
            name,
            partitions,
            replicas,
            config,
        } => {
            validate_name(name)?;
            if *partitions == 0 {
                bail!("partitions must be > 0");
            }
            if *replicas == 0 {
                bail!("replicas must be > 0");
            }
            let mut body: Value = json!({
                "name": name,
                "partitions": partitions,
                "replicas": replicas,
            });
            if !config.is_empty() {
                body["config"] = json!(parse_kv(config)?);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/strimzi/v1/topics")
                    .with_body(body),
            )
        }
        TopicCmd::Delete { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Delete,
                format!("/api/compat/strimzi/v1/topics/{}", name),
            ))
        }
        TopicCmd::Describe { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/strimzi/v1/topics/{}", name),
            ))
        }
        TopicCmd::Alter {
            name,
            partitions,
            config,
        } => {
            validate_name(name)?;
            if partitions.is_none() && config.is_empty() {
                bail!("alter needs --partitions or --config");
            }
            let mut body: Value = json!({});
            if let Some(p) = partitions {
                body["partitions"] = json!(p);
            }
            if !config.is_empty() {
                body["config"] = json!(parse_kv(config)?);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Patch,
                format!("/api/compat/strimzi/v1/topics/{}", name),
            )
            .with_body(body))
        }
    }
}

fn prepare_users(cmd: &UserCmd) -> Result<PreparedRequest> {
    match cmd {
        UserCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/strimzi/v1/users",
        )),
        UserCmd::Create { name, auth } => {
            validate_name(name)?;
            if !AUTH_KINDS.contains(&auth.as_str()) {
                bail!("unknown auth `{}`; want one of {:?}", auth, AUTH_KINDS);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/strimzi/v1/users")
                    .with_body(json!({"name": name, "auth": auth})),
            )
        }
        UserCmd::Delete { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Delete,
                format!("/api/compat/strimzi/v1/users/{}", name),
            ))
        }
        UserCmd::Describe { name } => {
            validate_name(name)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/strimzi/v1/users/{}", name),
            ))
        }
    }
}

fn prepare_groups(cmd: &GroupCmd) -> Result<PreparedRequest> {
    match cmd {
        GroupCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/strimzi/v1/groups",
        )),
        GroupCmd::Describe { id } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/strimzi/v1/groups/{}", id),
        )),
        GroupCmd::Delete { id } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/strimzi/v1/groups/{}", id),
        )),
        GroupCmd::Reset { id, topic, to } => {
            if to.is_empty() {
                bail!("--to required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/strimzi/v1/groups/{}/reset", id),
            )
            .with_body(json!({"topic": topic, "to": to})))
        }
    }
}

fn prepare_acls(cmd: &AclCmd) -> Result<PreparedRequest> {
    match cmd {
        AclCmd::List { topic, principal } => {
            let mut path = "/api/compat/strimzi/v1/acls".to_string();
            let mut params: Vec<String> = Vec::new();
            if let Some(t) = topic {
                params.push(format!("topic={}", t));
            }
            if let Some(p) = principal {
                params.push(format!("principal={}", p));
            }
            if !params.is_empty() {
                path.push('?');
                path.push_str(&params.join("&"));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        AclCmd::Grant {
            principal,
            resource,
            operation,
        } => {
            check_op(operation)?;
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/strimzi/v1/acls")
                    .with_body(json!({
                        "principal": principal,
                        "resource": resource,
                        "operation": operation,
                        "permission": "allow",
                    })),
            )
        }
        AclCmd::Revoke {
            principal,
            resource,
            operation,
        } => {
            check_op(operation)?;
            Ok(PreparedRequest::new(
                HttpVerb::Delete,
                "/api/compat/strimzi/v1/acls",
            )
            .with_body(json!({
                "principal": principal,
                "resource": resource,
                "operation": operation,
            })))
        }
    }
}

fn prepare_cluster(cmd: &ClusterCmd) -> Result<PreparedRequest> {
    match cmd {
        ClusterCmd::Status => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/strimzi/v1/cluster/status",
        )),
        ClusterCmd::RollingRestart { component } => {
            if !CLUSTER_COMPONENTS.contains(&component.as_str()) {
                bail!(
                    "unknown component `{}`; want one of {:?}",
                    component,
                    CLUSTER_COMPONENTS
                );
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/strimzi/v1/cluster/rolling-restart",
            )
            .with_body(json!({"component": component})))
        }
    }
}

fn prepare_connect(cmd: &ConnectCmd) -> Result<PreparedRequest> {
    match cmd {
        ConnectCmd::Status => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/strimzi/v1/connect/status",
        )),
        ConnectCmd::Connectors { cmd } => prepare_connector(cmd),
    }
}

fn prepare_connector(cmd: &ConnectorCmd) -> Result<PreparedRequest> {
    let base = "/api/compat/strimzi/v1/connect/connectors";
    match cmd {
        ConnectorCmd::List => Ok(PreparedRequest::new(HttpVerb::Get, base)),
        ConnectorCmd::Create { name, config } => {
            if config.is_empty() {
                bail!("--config required");
            }
            Ok(PreparedRequest::new(HttpVerb::Post, base)
                .with_body(json!({"name": name, "config": config})))
        }
        ConnectorCmd::Delete { name } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("{}/{}", base, name),
        )),
        ConnectorCmd::Pause { name } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            format!("{}/{}/pause", base, name),
        )
        .with_body(json!({}))),
        ConnectorCmd::Resume { name } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            format!("{}/{}/resume", base, name),
        )
        .with_body(json!({}))),
        ConnectorCmd::Restart { name } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            format!("{}/{}/restart", base, name),
        )
        .with_body(json!({}))),
    }
}

fn validate_name(n: &str) -> Result<()> {
    if n.is_empty() {
        bail!("name required");
    }
    if n.len() > 249 {
        bail!("name too long (max 249, Kafka limit)");
    }
    for ch in n.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-') {
            bail!("name allows only alnum, `.`, `_`, `-`");
        }
    }
    Ok(())
}

fn check_op(op: &str) -> Result<()> {
    if !ACL_OPS.contains(&op.to_lowercase().as_str()) {
        bail!("unknown operation `{}`; want one of {:?}", op, ACL_OPS);
    }
    Ok(())
}

fn parse_kv(kv: &[String]) -> Result<serde_json::Map<String, Value>> {
    let mut m = serde_json::Map::new();
    for entry in kv {
        let (k, v) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("expected key=value, got `{}`", entry))?;
        m.insert(k.to_string(), json!(v));
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_list() {
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::List,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/strimzi/v1/topics");
    }

    #[test]
    fn topics_create() {
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name: "orders".into(),
                partitions: 12,
                replicas: 3,
                config: vec![],
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["partitions"], 12);
        assert_eq!(body["replicas"], 3);
    }

    #[test]
    fn topics_create_with_config() {
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name: "orders".into(),
                partitions: 1,
                replicas: 1,
                config: vec![
                    "retention.ms=86400000".into(),
                    "min.insync.replicas=2".into(),
                ],
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["config"]["retention.ms"], "86400000");
    }

    #[test]
    fn topics_create_zero_partitions_rejected() {
        assert!(prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name: "x".into(),
                partitions: 0,
                replicas: 1,
                config: vec![],
            },
        })
        .is_err());
    }

    #[test]
    fn topics_create_zero_replicas_rejected() {
        assert!(prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name: "x".into(),
                partitions: 1,
                replicas: 0,
                config: vec![],
            },
        })
        .is_err());
    }

    #[test]
    fn topics_delete() {
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Delete {
                name: "orders".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn topics_describe() {
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Describe {
                name: "orders".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn topics_alter_partitions() {
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Alter {
                name: "orders".into(),
                partitions: Some(24),
                config: vec![],
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Patch);
        assert_eq!(r.body.unwrap()["partitions"], 24);
    }

    #[test]
    fn topics_alter_no_changes_rejected() {
        assert!(prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Alter {
                name: "orders".into(),
                partitions: None,
                config: vec![],
            },
        })
        .is_err());
    }

    #[test]
    fn topics_create_invalid_name_rejected() {
        assert!(prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name: "bad name!".into(),
                partitions: 1,
                replicas: 1,
                config: vec![],
            },
        })
        .is_err());
    }

    #[test]
    fn topics_name_max_length_ok() {
        let name = "a".repeat(249);
        let r = prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name,
                partitions: 1,
                replicas: 1,
                config: vec![],
            },
        });
        assert!(r.is_ok());
    }

    #[test]
    fn topics_name_over_limit_rejected() {
        let name = "a".repeat(250);
        assert!(prepare(&StrimziCmd::Topics {
            cmd: TopicCmd::Create {
                name,
                partitions: 1,
                replicas: 1,
                config: vec![],
            },
        })
        .is_err());
    }

    #[test]
    fn users_create_default_scram() {
        let r = prepare(&StrimziCmd::Users {
            cmd: UserCmd::Create {
                name: "alice".into(),
                auth: "scram-sha-512".into(),
            },
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["auth"], "scram-sha-512");
    }

    #[test]
    fn users_create_unknown_auth_rejected() {
        assert!(prepare(&StrimziCmd::Users {
            cmd: UserCmd::Create {
                name: "alice".into(),
                auth: "magic-key".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn users_list() {
        let r = prepare(&StrimziCmd::Users {
            cmd: UserCmd::List,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/strimzi/v1/users");
    }

    #[test]
    fn groups_describe() {
        let r = prepare(&StrimziCmd::Groups {
            cmd: GroupCmd::Describe {
                id: "consumers-1".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/groups/consumers-1"));
    }

    #[test]
    fn groups_reset_offsets() {
        let r = prepare(&StrimziCmd::Groups {
            cmd: GroupCmd::Reset {
                id: "g1".into(),
                topic: "orders".into(),
                to: "earliest".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        let body = r.body.unwrap();
        assert_eq!(body["to"], "earliest");
        assert_eq!(body["topic"], "orders");
    }

    #[test]
    fn groups_reset_empty_to_rejected() {
        assert!(prepare(&StrimziCmd::Groups {
            cmd: GroupCmd::Reset {
                id: "g".into(),
                topic: "t".into(),
                to: "".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn acls_list_filtered() {
        let r = prepare(&StrimziCmd::Acls {
            cmd: AclCmd::List {
                topic: Some("orders".into()),
                principal: Some("User:alice".into()),
            },
        })
        .unwrap();
        let p = r.path;
        assert!(p.contains("topic=orders"));
        assert!(p.contains("principal=User:alice"));
    }

    #[test]
    fn acls_grant() {
        let r = prepare(&StrimziCmd::Acls {
            cmd: AclCmd::Grant {
                principal: "User:alice".into(),
                resource: "topic:orders".into(),
                operation: "read".into(),
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["operation"], "read");
        assert_eq!(body["permission"], "allow");
    }

    #[test]
    fn acls_grant_unknown_op_rejected() {
        assert!(prepare(&StrimziCmd::Acls {
            cmd: AclCmd::Grant {
                principal: "x".into(),
                resource: "y".into(),
                operation: "explode".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn acls_grant_op_case_insensitive() {
        let r = prepare(&StrimziCmd::Acls {
            cmd: AclCmd::Grant {
                principal: "x".into(),
                resource: "y".into(),
                operation: "READ".into(),
            },
        });
        assert!(r.is_ok());
    }

    #[test]
    fn acls_revoke() {
        let r = prepare(&StrimziCmd::Acls {
            cmd: AclCmd::Revoke {
                principal: "x".into(),
                resource: "y".into(),
                operation: "write".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn cluster_status() {
        let r = prepare(&StrimziCmd::Cluster {
            cmd: ClusterCmd::Status,
        })
        .unwrap();
        assert!(r.path.ends_with("/cluster/status"));
    }

    #[test]
    fn cluster_rolling_restart() {
        let r = prepare(&StrimziCmd::Cluster {
            cmd: ClusterCmd::RollingRestart {
                component: "kafka".into(),
            },
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["component"], "kafka");
    }

    #[test]
    fn cluster_unknown_component_rejected() {
        assert!(prepare(&StrimziCmd::Cluster {
            cmd: ClusterCmd::RollingRestart {
                component: "frontend".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn connect_status() {
        let r = prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Status,
        })
        .unwrap();
        assert!(r.path.ends_with("/connect/status"));
    }

    #[test]
    fn connectors_create() {
        let r = prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Connectors {
                cmd: ConnectorCmd::Create {
                    name: "sink-pg".into(),
                    config: r#"{"connector.class":"io.confluent.JdbcSink"}"#.into(),
                },
            },
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["name"], "sink-pg");
    }

    #[test]
    fn connectors_create_empty_config_rejected() {
        assert!(prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Connectors {
                cmd: ConnectorCmd::Create {
                    name: "n".into(),
                    config: "".into(),
                },
            },
        })
        .is_err());
    }

    #[test]
    fn connectors_pause() {
        let r = prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Connectors {
                cmd: ConnectorCmd::Pause {
                    name: "n".into(),
                },
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/n/pause"));
    }

    #[test]
    fn connectors_resume() {
        let r = prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Connectors {
                cmd: ConnectorCmd::Resume {
                    name: "n".into(),
                },
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/n/resume"));
    }

    #[test]
    fn connectors_restart() {
        let r = prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Connectors {
                cmd: ConnectorCmd::Restart {
                    name: "n".into(),
                },
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/n/restart"));
    }

    #[test]
    fn connectors_delete() {
        let r = prepare(&StrimziCmd::Connect {
            cmd: ConnectCmd::Connectors {
                cmd: ConnectorCmd::Delete {
                    name: "n".into(),
                },
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn bridge_status() {
        let r = prepare(&StrimziCmd::Bridge {
            cmd: BridgeCmd::Status,
        })
        .unwrap();
        assert!(r.path.ends_with("/bridge/status"));
    }

    #[test]
    fn paths_use_compat_strimzi_prefix() {
        let r = prepare(&StrimziCmd::Bridge {
            cmd: BridgeCmd::Status,
        })
        .unwrap();
        assert!(r.path.starts_with("/api/compat/strimzi/v1/"));
    }
}
