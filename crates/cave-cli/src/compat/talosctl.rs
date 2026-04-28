//! `cavectl talosctl …` — Talos Linux runtime ops compat shim.
//!
//! Targets `/api/compat/talos/v1/...`. Cave runtime owns the
//! sovereign-Linux pieces (cave-kernel + cave-runtime); this shim
//! gives operators familiar with `talosctl` the same verb shape for
//! node-level operations.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum TalosCmd {
    /// Bootstrap the etcd cluster (call once on a fresh control plane).
    Bootstrap,
    /// Reboot a node.
    Reboot {
        #[arg(long)]
        node: String,
        /// `default`, `powercycle`.
        #[arg(long, default_value = "default")]
        mode: String,
    },
    /// Graceful shutdown.
    Shutdown {
        #[arg(long)]
        node: String,
        #[arg(long)]
        force: bool,
    },
    /// Upgrade Talos OS.
    Upgrade {
        #[arg(long)]
        node: String,
        #[arg(long)]
        image: String,
        /// Skip etcd-backup pre-check.
        #[arg(long)]
        preserve: bool,
    },
    /// Upgrade Kubernetes.
    UpgradeK8s {
        #[arg(long)]
        to: String,
    },
    /// Reset the node (wipe state, optionally graceful etcd leave).
    Reset {
        #[arg(long)]
        node: String,
        #[arg(long, default_value = "system")]
        mode: String,
        #[arg(long = "graceful")]
        graceful: bool,
    },
    /// List services.
    Services {
        #[arg(long)]
        node: Option<String>,
    },
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
    },
    /// Read a file from a node.
    Read {
        #[arg(long)]
        node: String,
        path: String,
    },
    /// List a directory on a node.
    List {
        #[arg(long)]
        node: String,
        path: String,
    },
    /// Stream logs from a service or container.
    Logs {
        #[arg(long)]
        node: String,
        target: String,
        #[arg(short = 'f', long)]
        follow: bool,
        #[arg(long)]
        tail: Option<u64>,
    },
    /// `get <resource>` (Talos COSI runtime resources).
    Get {
        resource: String,
        id: Option<String>,
        #[arg(long)]
        node: Option<String>,
    },
    /// Apply a Talos config patch.
    Patch {
        #[arg(long)]
        node: String,
        #[arg(long = "patch-file")]
        patch_file: String,
    },
    /// `talosctl etcd …` (subset).
    Etcd {
        #[command(subcommand)]
        cmd: EtcdCmd,
    },
    /// Print processes.
    Processes {
        #[arg(long)]
        node: String,
    },
    /// Print container summaries (CRI).
    Containers {
        #[arg(long)]
        node: String,
        /// Show kubelet-managed containers (`-k`).
        #[arg(short = 'k', long)]
        kubernetes: bool,
    },
    /// Print kernel ring buffer.
    Dmesg {
        #[arg(long)]
        node: String,
    },
    /// Collect a debug support bundle.
    Support {
        #[arg(long, num_args = 0..)]
        node: Vec<String>,
        #[arg(short = 'O', long, default_value = "support.zip")]
        output: String,
    },
    /// Print Talos & API versions.
    Version,
    /// Print kubeconfig fragment for the cluster.
    Kubeconfig,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServiceCmd {
    Start {
        #[arg(long)]
        node: String,
        name: String,
    },
    Stop {
        #[arg(long)]
        node: String,
        name: String,
    },
    Restart {
        #[arg(long)]
        node: String,
        name: String,
    },
    Status {
        #[arg(long)]
        node: String,
        name: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum EtcdCmd {
    Status {
        #[arg(long)]
        node: Option<String>,
    },
    Members,
    Snapshot {
        #[arg(short = 'O', long)]
        output: String,
    },
    ForfeitLeadership {
        #[arg(long)]
        node: String,
    },
    /// `talosctl etcd alarm list|disarm`
    Alarm {
        #[command(subcommand)]
        cmd: AlarmCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AlarmCmd {
    List,
    Disarm,
}

const REBOOT_MODES: &[&str] = &["default", "powercycle"];
const RESET_MODES: &[&str] = &["system", "user", "all"];

pub fn prepare(cmd: &TalosCmd) -> Result<PreparedRequest> {
    match cmd {
        TalosCmd::Bootstrap => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/talos/v1/bootstrap",
        )
        .with_body(json!({}))),
        TalosCmd::Reboot { node, mode } => {
            validate_node(node)?;
            if !REBOOT_MODES.contains(&mode.as_str()) {
                bail!("unknown reboot mode `{}`; want one of {:?}", mode, REBOOT_MODES);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/talos/v1/nodes/{}/reboot", node),
            )
            .with_body(json!({"mode": mode})))
        }
        TalosCmd::Shutdown { node, force } => {
            validate_node(node)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/talos/v1/nodes/{}/shutdown", node),
            )
            .with_body(json!({"force": force})))
        }
        TalosCmd::Upgrade {
            node,
            image,
            preserve,
        } => {
            validate_node(node)?;
            if image.is_empty() {
                bail!("--image required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/talos/v1/nodes/{}/upgrade", node),
            )
            .with_body(json!({"image": image, "preserve": preserve})))
        }
        TalosCmd::UpgradeK8s { to } => {
            if to.is_empty() {
                bail!("--to required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/talos/v1/upgrade-k8s",
            )
            .with_body(json!({"to": to})))
        }
        TalosCmd::Reset {
            node,
            mode,
            graceful,
        } => {
            validate_node(node)?;
            if !RESET_MODES.contains(&mode.as_str()) {
                bail!("unknown reset mode `{}`; want one of {:?}", mode, RESET_MODES);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/talos/v1/nodes/{}/reset", node),
            )
            .with_body(json!({"mode": mode, "graceful": graceful})))
        }
        TalosCmd::Services { node } => {
            let path = match node {
                Some(n) => {
                    validate_node(n)?;
                    format!("/api/compat/talos/v1/nodes/{}/services", n)
                }
                None => "/api/compat/talos/v1/services".to_string(),
            };
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        TalosCmd::Service { cmd } => prepare_service(cmd),
        TalosCmd::Read { node, path } => {
            validate_node(node)?;
            if path.is_empty() {
                bail!("path required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!(
                    "/api/compat/talos/v1/nodes/{}/read?path={}",
                    node,
                    urlencode(path)
                ),
            ))
        }
        TalosCmd::List { node, path } => {
            validate_node(node)?;
            if path.is_empty() {
                bail!("path required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!(
                    "/api/compat/talos/v1/nodes/{}/list?path={}",
                    node,
                    urlencode(path)
                ),
            ))
        }
        TalosCmd::Logs {
            node,
            target,
            follow,
            tail,
        } => {
            validate_node(node)?;
            let mut path = format!(
                "/api/compat/talos/v1/nodes/{}/logs/{}",
                node, target
            );
            let mut params: Vec<String> = Vec::new();
            if *follow {
                params.push("follow=true".to_string());
            }
            if let Some(t) = tail {
                params.push(format!("tail={}", t));
            }
            if !params.is_empty() {
                path.push('?');
                path.push_str(&params.join("&"));
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        TalosCmd::Get {
            resource,
            id,
            node,
        } => {
            if resource.is_empty() {
                bail!("resource required");
            }
            let mut path = match node {
                Some(n) => {
                    validate_node(n)?;
                    format!(
                        "/api/compat/talos/v1/nodes/{}/resources/{}",
                        n, resource
                    )
                }
                None => format!("/api/compat/talos/v1/resources/{}", resource),
            };
            if let Some(i) = id {
                path.push('/');
                path.push_str(i);
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        TalosCmd::Patch { node, patch_file } => {
            validate_node(node)?;
            if patch_file.is_empty() {
                bail!("--patch-file required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Patch,
                format!("/api/compat/talos/v1/nodes/{}/config", node),
            )
            .with_body(json!({"patch_file": patch_file})))
        }
        TalosCmd::Etcd { cmd } => prepare_etcd(cmd),
        TalosCmd::Processes { node } => {
            validate_node(node)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/talos/v1/nodes/{}/processes", node),
            ))
        }
        TalosCmd::Containers { node, kubernetes } => {
            validate_node(node)?;
            let mut path = format!("/api/compat/talos/v1/nodes/{}/containers", node);
            if *kubernetes {
                path.push_str("?kubernetes=true");
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        TalosCmd::Dmesg { node } => {
            validate_node(node)?;
            Ok(PreparedRequest::new(
                HttpVerb::Get,
                format!("/api/compat/talos/v1/nodes/{}/dmesg", node),
            ))
        }
        TalosCmd::Support { node, output } => {
            if output.is_empty() {
                bail!("--output required");
            }
            let mut body: Value = json!({"output": output});
            if !node.is_empty() {
                body["nodes"] = json!(node);
            }
            Ok(
                PreparedRequest::new(HttpVerb::Post, "/api/compat/talos/v1/support")
                    .with_body(body),
            )
        }
        TalosCmd::Version => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/talos/v1/version",
        )),
        TalosCmd::Kubeconfig => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/talos/v1/kubeconfig",
        )),
    }
}

fn prepare_service(cmd: &ServiceCmd) -> Result<PreparedRequest> {
    let (node, name, action) = match cmd {
        ServiceCmd::Start { node, name } => (node, name, "start"),
        ServiceCmd::Stop { node, name } => (node, name, "stop"),
        ServiceCmd::Restart { node, name } => (node, name, "restart"),
        ServiceCmd::Status { node, name } => (node, name, "status"),
    };
    validate_node(node)?;
    if name.is_empty() {
        bail!("service name required");
    }
    let path = format!(
        "/api/compat/talos/v1/nodes/{}/services/{}/{}",
        node, name, action
    );
    if action == "status" {
        Ok(PreparedRequest::new(HttpVerb::Get, path))
    } else {
        Ok(PreparedRequest::new(HttpVerb::Post, path).with_body(json!({})))
    }
}

fn prepare_etcd(cmd: &EtcdCmd) -> Result<PreparedRequest> {
    match cmd {
        EtcdCmd::Status { node } => {
            let path = match node {
                Some(n) => {
                    validate_node(n)?;
                    format!("/api/compat/talos/v1/nodes/{}/etcd/status", n)
                }
                None => "/api/compat/talos/v1/etcd/status".to_string(),
            };
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        EtcdCmd::Members => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/talos/v1/etcd/members",
        )),
        EtcdCmd::Snapshot { output } => {
            if output.is_empty() {
                bail!("--output required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/talos/v1/etcd/snapshot",
            )
            .with_body(json!({"output": output})))
        }
        EtcdCmd::ForfeitLeadership { node } => {
            validate_node(node)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                format!("/api/compat/talos/v1/nodes/{}/etcd/forfeit", node),
            )
            .with_body(json!({})))
        }
        EtcdCmd::Alarm { cmd } => match cmd {
            AlarmCmd::List => Ok(PreparedRequest::new(
                HttpVerb::Get,
                "/api/compat/talos/v1/etcd/alarms",
            )),
            AlarmCmd::Disarm => Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/talos/v1/etcd/alarms/disarm",
            )
            .with_body(json!({}))),
        },
    }
}

fn validate_node(n: &str) -> Result<()> {
    if n.is_empty() {
        bail!("--node required");
    }
    Ok(())
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap() {
        let r = prepare(&TalosCmd::Bootstrap).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/compat/talos/v1/bootstrap");
    }

    #[test]
    fn reboot_default_mode() {
        let r = prepare(&TalosCmd::Reboot {
            node: "n1".into(),
            mode: "default".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/nodes/n1/reboot"));
        assert_eq!(r.body.unwrap()["mode"], "default");
    }

    #[test]
    fn reboot_powercycle() {
        let r = prepare(&TalosCmd::Reboot {
            node: "n1".into(),
            mode: "powercycle".into(),
        });
        assert!(r.is_ok());
    }

    #[test]
    fn reboot_unknown_mode() {
        assert!(prepare(&TalosCmd::Reboot {
            node: "n1".into(),
            mode: "smashing".into(),
        })
        .is_err());
    }

    #[test]
    fn reboot_empty_node() {
        assert!(prepare(&TalosCmd::Reboot {
            node: "".into(),
            mode: "default".into(),
        })
        .is_err());
    }

    #[test]
    fn shutdown() {
        let r = prepare(&TalosCmd::Shutdown {
            node: "n1".into(),
            force: false,
        })
        .unwrap();
        assert!(r.path.ends_with("/shutdown"));
        assert_eq!(r.body.unwrap()["force"], false);
    }

    #[test]
    fn shutdown_force() {
        let r = prepare(&TalosCmd::Shutdown {
            node: "n1".into(),
            force: true,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["force"], true);
    }

    #[test]
    fn upgrade() {
        let r = prepare(&TalosCmd::Upgrade {
            node: "n1".into(),
            image: "ghcr.io/siderolabs/installer:v1.7.0".into(),
            preserve: true,
        })
        .unwrap();
        let body = r.body.unwrap();
        assert!(body["image"].as_str().unwrap().contains("v1.7.0"));
        assert_eq!(body["preserve"], true);
    }

    #[test]
    fn upgrade_empty_image() {
        assert!(prepare(&TalosCmd::Upgrade {
            node: "n1".into(),
            image: "".into(),
            preserve: false,
        })
        .is_err());
    }

    #[test]
    fn upgrade_k8s() {
        let r = prepare(&TalosCmd::UpgradeK8s {
            to: "v1.30.0".into(),
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/talos/v1/upgrade-k8s");
    }

    #[test]
    fn upgrade_k8s_empty() {
        assert!(prepare(&TalosCmd::UpgradeK8s { to: "".into() }).is_err());
    }

    #[test]
    fn reset_mode() {
        let r = prepare(&TalosCmd::Reset {
            node: "n1".into(),
            mode: "system".into(),
            graceful: true,
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["mode"], "system");
    }

    #[test]
    fn reset_unknown_mode() {
        assert!(prepare(&TalosCmd::Reset {
            node: "n1".into(),
            mode: "zap".into(),
            graceful: false,
        })
        .is_err());
    }

    #[test]
    fn services_cluster_wide() {
        let r = prepare(&TalosCmd::Services { node: None }).unwrap();
        assert_eq!(r.path, "/api/compat/talos/v1/services");
    }

    #[test]
    fn services_per_node() {
        let r = prepare(&TalosCmd::Services {
            node: Some("n1".into()),
        })
        .unwrap();
        assert!(r.path.ends_with("/nodes/n1/services"));
    }

    #[test]
    fn service_start() {
        let r = prepare(&TalosCmd::Service {
            cmd: ServiceCmd::Start {
                node: "n1".into(),
                name: "kubelet".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/services/kubelet/start"));
    }

    #[test]
    fn service_status_uses_get() {
        let r = prepare(&TalosCmd::Service {
            cmd: ServiceCmd::Status {
                node: "n1".into(),
                name: "kubelet".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn service_stop() {
        let r = prepare(&TalosCmd::Service {
            cmd: ServiceCmd::Stop {
                node: "n1".into(),
                name: "etcd".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/services/etcd/stop"));
    }

    #[test]
    fn service_restart() {
        let r = prepare(&TalosCmd::Service {
            cmd: ServiceCmd::Restart {
                node: "n1".into(),
                name: "etcd".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/services/etcd/restart"));
    }

    #[test]
    fn service_empty_name_rejected() {
        assert!(prepare(&TalosCmd::Service {
            cmd: ServiceCmd::Start {
                node: "n1".into(),
                name: "".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn read_path_url_encoded() {
        let r = prepare(&TalosCmd::Read {
            node: "n1".into(),
            path: "/etc/cni/net.d".into(),
        })
        .unwrap();
        assert!(r.path.contains("path=/etc/cni/net.d"));
    }

    #[test]
    fn read_empty_path_rejected() {
        assert!(prepare(&TalosCmd::Read {
            node: "n1".into(),
            path: "".into(),
        })
        .is_err());
    }

    #[test]
    fn list_path() {
        let r = prepare(&TalosCmd::List {
            node: "n1".into(),
            path: "/etc".into(),
        })
        .unwrap();
        assert!(r.path.contains("path=/etc"));
    }

    #[test]
    fn logs_basic() {
        let r = prepare(&TalosCmd::Logs {
            node: "n1".into(),
            target: "kubelet".into(),
            follow: false,
            tail: None,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/talos/v1/nodes/n1/logs/kubelet");
    }

    #[test]
    fn logs_follow_tail() {
        let r = prepare(&TalosCmd::Logs {
            node: "n1".into(),
            target: "kubelet".into(),
            follow: true,
            tail: Some(50),
        })
        .unwrap();
        assert!(r.path.contains("follow=true"));
        assert!(r.path.contains("tail=50"));
    }

    #[test]
    fn get_resource() {
        let r = prepare(&TalosCmd::Get {
            resource: "machineconfigs".into(),
            id: None,
            node: Some("n1".into()),
        })
        .unwrap();
        assert!(r
            .path
            .ends_with("/nodes/n1/resources/machineconfigs"));
    }

    #[test]
    fn get_resource_with_id() {
        let r = prepare(&TalosCmd::Get {
            resource: "addresses".into(),
            id: Some("eth0".into()),
            node: None,
        })
        .unwrap();
        assert!(r.path.ends_with("/resources/addresses/eth0"));
    }

    #[test]
    fn get_empty_resource_rejected() {
        assert!(prepare(&TalosCmd::Get {
            resource: "".into(),
            id: None,
            node: None,
        })
        .is_err());
    }

    #[test]
    fn patch_node_config() {
        let r = prepare(&TalosCmd::Patch {
            node: "n1".into(),
            patch_file: "patch.yaml".into(),
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Patch);
        assert!(r.path.ends_with("/nodes/n1/config"));
    }

    #[test]
    fn patch_empty_file_rejected() {
        assert!(prepare(&TalosCmd::Patch {
            node: "n1".into(),
            patch_file: "".into(),
        })
        .is_err());
    }

    #[test]
    fn etcd_status() {
        let r = prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::Status { node: None },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/talos/v1/etcd/status");
    }

    #[test]
    fn etcd_members() {
        let r = prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::Members,
        })
        .unwrap();
        assert!(r.path.ends_with("/etcd/members"));
    }

    #[test]
    fn etcd_snapshot() {
        let r = prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::Snapshot {
                output: "/tmp/etcd.db".into(),
            },
        })
        .unwrap();
        assert_eq!(r.body.unwrap()["output"], "/tmp/etcd.db");
    }

    #[test]
    fn etcd_snapshot_empty_output() {
        assert!(prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::Snapshot { output: "".into() },
        })
        .is_err());
    }

    #[test]
    fn etcd_forfeit() {
        let r = prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::ForfeitLeadership {
                node: "n1".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/etcd/forfeit"));
    }

    #[test]
    fn etcd_alarm_list() {
        let r = prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::Alarm {
                cmd: AlarmCmd::List,
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn etcd_alarm_disarm() {
        let r = prepare(&TalosCmd::Etcd {
            cmd: EtcdCmd::Alarm {
                cmd: AlarmCmd::Disarm,
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/disarm"));
    }

    #[test]
    fn processes() {
        let r = prepare(&TalosCmd::Processes {
            node: "n1".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/processes"));
    }

    #[test]
    fn containers_kubernetes_flag() {
        let r = prepare(&TalosCmd::Containers {
            node: "n1".into(),
            kubernetes: true,
        })
        .unwrap();
        assert!(r.path.contains("kubernetes=true"));
    }

    #[test]
    fn dmesg() {
        let r = prepare(&TalosCmd::Dmesg {
            node: "n1".into(),
        })
        .unwrap();
        assert!(r.path.ends_with("/dmesg"));
    }

    #[test]
    fn support_default_output() {
        let r = prepare(&TalosCmd::Support {
            node: vec!["n1".into(), "n2".into()],
            output: "support.zip".into(),
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/talos/v1/support");
        let body = r.body.unwrap();
        assert_eq!(body["output"], "support.zip");
        assert!(body["nodes"].is_array());
    }

    #[test]
    fn support_no_nodes_omits() {
        let r = prepare(&TalosCmd::Support {
            node: vec![],
            output: "x.zip".into(),
        })
        .unwrap();
        assert!(r.body.unwrap().get("nodes").is_none());
    }

    #[test]
    fn version() {
        let r = prepare(&TalosCmd::Version).unwrap();
        assert!(r.path.ends_with("/version"));
    }

    #[test]
    fn kubeconfig() {
        let r = prepare(&TalosCmd::Kubeconfig).unwrap();
        assert!(r.path.ends_with("/kubeconfig"));
    }

    #[test]
    fn paths_use_compat_talos_prefix() {
        let r = prepare(&TalosCmd::Bootstrap).unwrap();
        assert!(r.path.starts_with("/api/compat/talos/v1/"));
    }
}
