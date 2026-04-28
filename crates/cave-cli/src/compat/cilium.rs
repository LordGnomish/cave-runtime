//! `cavectl cilium …` — Cilium CLI compat shim.
//!
//! Targets `/api/compat/cilium/v1/...`. cave-net (Cilium reimpl) owns
//! the eBPF dataplane; this shim translates `cilium` verb shape into
//! routed requests.

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::native::{HttpVerb, PreparedRequest};

#[derive(Subcommand, Debug, Clone)]
pub enum CiliumCmd {
    /// Show agent status.
    Status {
        #[arg(long)]
        verbose: bool,
        /// `--brief` — short summary only.
        #[arg(long)]
        brief: bool,
    },
    Connectivity {
        #[command(subcommand)]
        cmd: ConnectivityCmd,
    },
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    Endpoint {
        #[command(subcommand)]
        cmd: EndpointCmd,
    },
    Identity {
        #[command(subcommand)]
        cmd: IdentityCmd,
    },
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
    },
    Bpf {
        #[command(subcommand)]
        cmd: BpfCmd,
    },
    /// Run the cluster pre-flight check.
    Preflight,
    /// Get/set debug verbosity.
    Debuginfo,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConnectivityCmd {
    Test {
        /// Test scope: `all`, `pod-to-pod`, `pod-to-service`,
        /// `pod-to-world`, `node-to-node`.
        #[arg(long, default_value = "all")]
        scope: String,
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long = "force-deploy")]
        force_deploy: bool,
    },
    Perf {
        #[arg(long, default_value = "30s")]
        duration: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PolicyCmd {
    Get { id: Option<String> },
    Import { file: String },
    Delete { id: String },
    Trace {
        #[arg(long)]
        src: String,
        #[arg(long)]
        dst: String,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        protocol: Option<String>,
    },
    Validate { file: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum EndpointCmd {
    List,
    Get { id: String },
    Log { id: String },
    Disconnect { id: String },
    /// Apply config to an endpoint.
    Config {
        id: String,
        #[arg(num_args = 1..)]
        kv: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum IdentityCmd {
    List,
    Get { id: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServiceCmd {
    List,
    Get { id: String },
    Update {
        id: String,
        #[arg(long)]
        backends: String,
    },
    Delete { id: String },
}

#[derive(Subcommand, Debug, Clone)]
pub enum BpfCmd {
    Lb {
        #[command(subcommand)]
        cmd: BpfLbCmd,
    },
    Tunnel {
        #[command(subcommand)]
        cmd: BpfTunnelCmd,
    },
    /// `cilium bpf endpoint list`
    Endpoint {
        #[command(subcommand)]
        cmd: BpfEndpointCmd,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum BpfLbCmd {
    List,
    Flush,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BpfTunnelCmd {
    List,
    Flush,
}

#[derive(Subcommand, Debug, Clone)]
pub enum BpfEndpointCmd {
    List,
}

const CONN_SCOPES: &[&str] = &[
    "all",
    "pod-to-pod",
    "pod-to-service",
    "pod-to-world",
    "node-to-node",
];

pub fn prepare(cmd: &CiliumCmd) -> Result<PreparedRequest> {
    match cmd {
        CiliumCmd::Status { verbose, brief } => {
            if *verbose && *brief {
                bail!("--verbose and --brief are mutually exclusive");
            }
            let mut path = "/api/compat/cilium/v1/status".to_string();
            if *verbose {
                path.push_str("?verbose=true");
            } else if *brief {
                path.push_str("?brief=true");
            }
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        CiliumCmd::Connectivity { cmd } => prepare_connectivity(cmd),
        CiliumCmd::Policy { cmd } => prepare_policy(cmd),
        CiliumCmd::Endpoint { cmd } => prepare_endpoint(cmd),
        CiliumCmd::Identity { cmd } => prepare_identity(cmd),
        CiliumCmd::Service { cmd } => prepare_service(cmd),
        CiliumCmd::Bpf { cmd } => prepare_bpf(cmd),
        CiliumCmd::Preflight => Ok(PreparedRequest::new(
            HttpVerb::Post,
            "/api/compat/cilium/v1/preflight",
        )
        .with_body(json!({}))),
        CiliumCmd::Debuginfo => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/cilium/v1/debuginfo",
        )),
    }
}

fn prepare_connectivity(cmd: &ConnectivityCmd) -> Result<PreparedRequest> {
    match cmd {
        ConnectivityCmd::Test {
            scope,
            namespace,
            force_deploy,
        } => {
            if !CONN_SCOPES.contains(&scope.as_str()) {
                bail!(
                    "unknown connectivity scope `{}`; want one of {:?}",
                    scope,
                    CONN_SCOPES
                );
            }
            let mut body: Value = json!({"scope": scope});
            if let Some(n) = namespace {
                body["namespace"] = json!(n);
            }
            if *force_deploy {
                body["force_deploy"] = json!(true);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/connectivity/test",
            )
            .with_body(body))
        }
        ConnectivityCmd::Perf { duration } => {
            parse_duration_seconds(duration)?;
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/connectivity/perf",
            )
            .with_body(json!({"duration": duration})))
        }
    }
}

fn prepare_policy(cmd: &PolicyCmd) -> Result<PreparedRequest> {
    match cmd {
        PolicyCmd::Get { id } => {
            let path = match id {
                Some(i) => format!("/api/compat/cilium/v1/policy/{}", i),
                None => "/api/compat/cilium/v1/policy".to_string(),
            };
            Ok(PreparedRequest::new(HttpVerb::Get, path))
        }
        PolicyCmd::Import { file } => {
            if file.is_empty() {
                bail!("policy file required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/policy/import",
            )
            .with_body(json!({"file": file})))
        }
        PolicyCmd::Delete { id } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/cilium/v1/policy/{}", id),
        )),
        PolicyCmd::Trace {
            src,
            dst,
            port,
            protocol,
        } => {
            let mut body: Value = json!({"src": src, "dst": dst});
            if let Some(p) = port {
                body["port"] = json!(p);
            }
            if let Some(p) = protocol {
                let lower = p.to_lowercase();
                if !["tcp", "udp", "any"].contains(&lower.as_str()) {
                    bail!("unknown protocol `{}`", p);
                }
                body["protocol"] = json!(lower);
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/policy/trace",
            )
            .with_body(body))
        }
        PolicyCmd::Validate { file } => {
            if file.is_empty() {
                bail!("policy file required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/policy/validate",
            )
            .with_body(json!({"file": file})))
        }
    }
}

fn prepare_endpoint(cmd: &EndpointCmd) -> Result<PreparedRequest> {
    match cmd {
        EndpointCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/cilium/v1/endpoints",
        )),
        EndpointCmd::Get { id } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/cilium/v1/endpoints/{}", id),
        )),
        EndpointCmd::Log { id } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/cilium/v1/endpoints/{}/log", id),
        )),
        EndpointCmd::Disconnect { id } => Ok(PreparedRequest::new(
            HttpVerb::Post,
            format!("/api/compat/cilium/v1/endpoints/{}/disconnect", id),
        )
        .with_body(json!({}))),
        EndpointCmd::Config { id, kv } => {
            let mut data = serde_json::Map::new();
            for entry in kv {
                let (k, v) = entry
                    .split_once('=')
                    .ok_or_else(|| anyhow::anyhow!("expected key=value, got `{}`", entry))?;
                data.insert(k.to_string(), json!(v));
            }
            Ok(PreparedRequest::new(
                HttpVerb::Patch,
                format!("/api/compat/cilium/v1/endpoints/{}/config", id),
            )
            .with_body(Value::Object(data)))
        }
    }
}

fn prepare_identity(cmd: &IdentityCmd) -> Result<PreparedRequest> {
    match cmd {
        IdentityCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/cilium/v1/identities",
        )),
        IdentityCmd::Get { id } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/cilium/v1/identities/{}", id),
        )),
    }
}

fn prepare_service(cmd: &ServiceCmd) -> Result<PreparedRequest> {
    match cmd {
        ServiceCmd::List => Ok(PreparedRequest::new(
            HttpVerb::Get,
            "/api/compat/cilium/v1/services",
        )),
        ServiceCmd::Get { id } => Ok(PreparedRequest::new(
            HttpVerb::Get,
            format!("/api/compat/cilium/v1/services/{}", id),
        )),
        ServiceCmd::Update { id, backends } => {
            let backends: Vec<String> = backends
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if backends.is_empty() {
                bail!("at least one backend required");
            }
            Ok(PreparedRequest::new(
                HttpVerb::Put,
                format!("/api/compat/cilium/v1/services/{}", id),
            )
            .with_body(json!({"backends": backends})))
        }
        ServiceCmd::Delete { id } => Ok(PreparedRequest::new(
            HttpVerb::Delete,
            format!("/api/compat/cilium/v1/services/{}", id),
        )),
    }
}

fn prepare_bpf(cmd: &BpfCmd) -> Result<PreparedRequest> {
    match cmd {
        BpfCmd::Lb { cmd } => match cmd {
            BpfLbCmd::List => Ok(PreparedRequest::new(
                HttpVerb::Get,
                "/api/compat/cilium/v1/bpf/lb",
            )),
            BpfLbCmd::Flush => Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/bpf/lb/flush",
            )
            .with_body(json!({}))),
        },
        BpfCmd::Tunnel { cmd } => match cmd {
            BpfTunnelCmd::List => Ok(PreparedRequest::new(
                HttpVerb::Get,
                "/api/compat/cilium/v1/bpf/tunnel",
            )),
            BpfTunnelCmd::Flush => Ok(PreparedRequest::new(
                HttpVerb::Post,
                "/api/compat/cilium/v1/bpf/tunnel/flush",
            )
            .with_body(json!({}))),
        },
        BpfCmd::Endpoint { cmd } => match cmd {
            BpfEndpointCmd::List => Ok(PreparedRequest::new(
                HttpVerb::Get,
                "/api/compat/cilium/v1/bpf/endpoint",
            )),
        },
    }
}

fn parse_duration_seconds(s: &str) -> Result<u64> {
    if s.is_empty() {
        bail!("empty duration");
    }
    let split = s
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let (num, suffix) = s.split_at(split);
    let n: u64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number `{}`", num))?;
    let secs = match suffix {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        other => bail!("unknown duration suffix `{}`", other),
    };
    if secs == 0 {
        bail!("duration must be positive");
    }
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_default() {
        let r = prepare(&CiliumCmd::Status {
            verbose: false,
            brief: false,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/cilium/v1/status");
    }

    #[test]
    fn status_verbose() {
        let r = prepare(&CiliumCmd::Status {
            verbose: true,
            brief: false,
        })
        .unwrap();
        assert!(r.path.contains("verbose=true"));
    }

    #[test]
    fn status_brief() {
        let r = prepare(&CiliumCmd::Status {
            verbose: false,
            brief: true,
        })
        .unwrap();
        assert!(r.path.contains("brief=true"));
    }

    #[test]
    fn status_verbose_brief_mutex() {
        assert!(prepare(&CiliumCmd::Status {
            verbose: true,
            brief: true,
        })
        .is_err());
    }

    #[test]
    fn connectivity_test_default() {
        let r = prepare(&CiliumCmd::Connectivity {
            cmd: ConnectivityCmd::Test {
                scope: "all".into(),
                namespace: None,
                force_deploy: false,
            },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/cilium/v1/connectivity/test");
        assert_eq!(r.body.unwrap()["scope"], "all");
    }

    #[test]
    fn connectivity_scopes_round_trip() {
        for s in CONN_SCOPES {
            let r = prepare(&CiliumCmd::Connectivity {
                cmd: ConnectivityCmd::Test {
                    scope: (*s).into(),
                    namespace: None,
                    force_deploy: false,
                },
            });
            assert!(r.is_ok());
        }
    }

    #[test]
    fn connectivity_unknown_scope_rejected() {
        assert!(prepare(&CiliumCmd::Connectivity {
            cmd: ConnectivityCmd::Test {
                scope: "spider".into(),
                namespace: None,
                force_deploy: false,
            },
        })
        .is_err());
    }

    #[test]
    fn connectivity_perf() {
        let r = prepare(&CiliumCmd::Connectivity {
            cmd: ConnectivityCmd::Perf {
                duration: "30s".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/perf"));
        assert_eq!(r.body.unwrap()["duration"], "30s");
    }

    #[test]
    fn connectivity_perf_bad_duration() {
        assert!(prepare(&CiliumCmd::Connectivity {
            cmd: ConnectivityCmd::Perf {
                duration: "30x".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn policy_get_all() {
        let r = prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Get { id: None },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/cilium/v1/policy");
    }

    #[test]
    fn policy_get_one() {
        let r = prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Get {
                id: Some("123".into()),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/policy/123"));
    }

    #[test]
    fn policy_import() {
        let r = prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Import {
                file: "policy.yaml".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/policy/import"));
        assert_eq!(r.body.unwrap()["file"], "policy.yaml");
    }

    #[test]
    fn policy_import_empty_rejected() {
        assert!(prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Import { file: "".into() },
        })
        .is_err());
    }

    #[test]
    fn policy_delete() {
        let r = prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Delete { id: "x".into() },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn policy_trace_with_port_proto() {
        let r = prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Trace {
                src: "id:1".into(),
                dst: "id:2".into(),
                port: Some(80),
                protocol: Some("TCP".into()),
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["port"], 80);
        assert_eq!(body["protocol"], "tcp");
    }

    #[test]
    fn policy_trace_unknown_protocol_rejected() {
        assert!(prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Trace {
                src: "x".into(),
                dst: "y".into(),
                port: None,
                protocol: Some("smtp".into()),
            },
        })
        .is_err());
    }

    #[test]
    fn policy_validate() {
        let r = prepare(&CiliumCmd::Policy {
            cmd: PolicyCmd::Validate {
                file: "p.yaml".into(),
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/policy/validate"));
    }

    #[test]
    fn endpoint_list() {
        let r = prepare(&CiliumCmd::Endpoint {
            cmd: EndpointCmd::List,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/cilium/v1/endpoints");
    }

    #[test]
    fn endpoint_get() {
        let r = prepare(&CiliumCmd::Endpoint {
            cmd: EndpointCmd::Get { id: "42".into() },
        })
        .unwrap();
        assert!(r.path.ends_with("/endpoints/42"));
    }

    #[test]
    fn endpoint_log() {
        let r = prepare(&CiliumCmd::Endpoint {
            cmd: EndpointCmd::Log { id: "42".into() },
        })
        .unwrap();
        assert!(r.path.ends_with("/log"));
    }

    #[test]
    fn endpoint_disconnect() {
        let r = prepare(&CiliumCmd::Endpoint {
            cmd: EndpointCmd::Disconnect {
                id: "42".into(),
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert!(r.path.ends_with("/disconnect"));
    }

    #[test]
    fn endpoint_config_kv() {
        let r = prepare(&CiliumCmd::Endpoint {
            cmd: EndpointCmd::Config {
                id: "42".into(),
                kv: vec!["debug=true".into(), "policy-enforcement=always".into()],
            },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Patch);
        let body = r.body.unwrap();
        assert_eq!(body["debug"], "true");
        assert_eq!(body["policy-enforcement"], "always");
    }

    #[test]
    fn endpoint_config_bad_kv_rejected() {
        assert!(prepare(&CiliumCmd::Endpoint {
            cmd: EndpointCmd::Config {
                id: "42".into(),
                kv: vec!["no_equals".into()],
            },
        })
        .is_err());
    }

    #[test]
    fn identity_list() {
        let r = prepare(&CiliumCmd::Identity {
            cmd: IdentityCmd::List,
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/cilium/v1/identities");
    }

    #[test]
    fn service_update_splits_backends() {
        let r = prepare(&CiliumCmd::Service {
            cmd: ServiceCmd::Update {
                id: "1".into(),
                backends: "10.0.0.1:80, 10.0.0.2:80".into(),
            },
        })
        .unwrap();
        let body = r.body.unwrap();
        assert_eq!(body["backends"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn service_update_empty_backends_rejected() {
        assert!(prepare(&CiliumCmd::Service {
            cmd: ServiceCmd::Update {
                id: "1".into(),
                backends: "".into(),
            },
        })
        .is_err());
    }

    #[test]
    fn service_delete() {
        let r = prepare(&CiliumCmd::Service {
            cmd: ServiceCmd::Delete { id: "1".into() },
        })
        .unwrap();
        assert_eq!(r.verb, HttpVerb::Delete);
    }

    #[test]
    fn bpf_lb_list() {
        let r = prepare(&CiliumCmd::Bpf {
            cmd: BpfCmd::Lb { cmd: BpfLbCmd::List },
        })
        .unwrap();
        assert_eq!(r.path, "/api/compat/cilium/v1/bpf/lb");
    }

    #[test]
    fn bpf_lb_flush() {
        let r = prepare(&CiliumCmd::Bpf {
            cmd: BpfCmd::Lb { cmd: BpfLbCmd::Flush },
        })
        .unwrap();
        assert!(r.path.ends_with("/flush"));
    }

    #[test]
    fn bpf_tunnel_list() {
        let r = prepare(&CiliumCmd::Bpf {
            cmd: BpfCmd::Tunnel {
                cmd: BpfTunnelCmd::List,
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/bpf/tunnel"));
    }

    #[test]
    fn bpf_endpoint_list() {
        let r = prepare(&CiliumCmd::Bpf {
            cmd: BpfCmd::Endpoint {
                cmd: BpfEndpointCmd::List,
            },
        })
        .unwrap();
        assert!(r.path.ends_with("/bpf/endpoint"));
    }

    #[test]
    fn preflight() {
        let r = prepare(&CiliumCmd::Preflight).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
    }

    #[test]
    fn debuginfo() {
        let r = prepare(&CiliumCmd::Debuginfo).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn paths_use_compat_cilium_prefix() {
        let r = prepare(&CiliumCmd::Preflight).unwrap();
        assert!(r.path.starts_with("/api/compat/cilium/v1/"));
    }
}
