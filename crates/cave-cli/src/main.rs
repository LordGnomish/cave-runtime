//! `cavectl` binary — clap surface for the library's `native::*` and
//! `compat::*` modules. Per ADR-RUNTIME-CLI-CONSOLIDATION-001 (and
//! Burak's Integration Sprint C mandate): one binary, two coexisting
//! surfaces (native + compat), each delegating to the same shared
//! `PreparedRequest` shape that the lib already produces.
//!
//! No stubs: every accepted subcommand calls a real `prepare()` from
//! the lib, builds an HTTP request via `reqwest`, sends it to the
//! configured server, parses the response, and renders it according
//! to `--output`.

use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use reqwest::{Method, StatusCode};
use serde_json::Value;

use cavectl::{
    compat::{argocd, cilium, etcdctl, harbor, helm, kubectl, strimzi, talosctl, vault},
    native::{self, HttpVerb, PreparedRequest},
};

// ── Top-level CLI ────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "cavectl",
    about = "CAVE Runtime CLI — single binary, native + compatibility surfaces (ADR-RUNTIME-CLI-CONSOLIDATION-001)",
    version
)]
struct Cli {
    /// API server base URL.
    #[arg(long, env = "CAVE_SERVER", default_value = "http://localhost:3000", global = true)]
    server: String,

    /// Bearer token for auth.
    #[arg(long, env = "CAVE_TOKEN", global = true)]
    token: Option<String>,

    /// Output format: `table` (default), `json`, `yaml`, `name`, `wide`.
    #[arg(short = 'o', long, global = true, default_value = "table")]
    output: String,

    #[command(subcommand)]
    cmd: TopCmd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Table,
    Json,
    Yaml,
    Name,
    Wide,
}

impl OutputFormat {
    fn parse(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "table" => Ok(OutputFormat::Table),
            "json" => Ok(OutputFormat::Json),
            "yaml" | "yml" => Ok(OutputFormat::Yaml),
            "name" => Ok(OutputFormat::Name),
            "wide" => Ok(OutputFormat::Wide),
            other => anyhow::bail!(
                "unknown output format `{}`; want one of table|json|yaml|name|wide",
                other
            ),
        }
    }
}

#[derive(Subcommand, Debug)]
enum TopCmd {
    // ── Native verbs ────────────────────────────────────────────────────────

    /// Deploy a Cave module.
    Deploy(native::deploy::DeployArgs),

    /// Get Cave resources (tenants/modules/deployments/pods/secrets/...).
    Get(native::get::GetArgs),

    /// Describe a Cave resource (rich detail + reconcile diffs).
    Describe(native::describe::DescribeArgs),

    /// Stream logs from a crate or pod.
    Logs(native::logs::LogsArgs),

    /// Federated event stream.
    Events(native::events::EventsArgs),

    /// Manage tenant secrets (list/get/add/rotate/delete).
    Secrets {
        #[command(subcommand)]
        cmd: native::secrets::SecretsCmd,
    },

    /// Toggle / set / list feature flags.
    Flag {
        #[command(subcommand)]
        cmd: native::flag::FlagCmd,
    },

    /// Run / list / abort chaos experiments.
    Chaos {
        #[command(subcommand)]
        cmd: native::chaos::ChaosCmd,
    },

    /// Module / dependency graph.
    Topology(native::topology::TopologyArgs),

    // ── Compat shims ────────────────────────────────────────────────────────

    /// kubectl-compatible CLI shim.
    Kubectl {
        #[command(subcommand)]
        cmd: KubectlCmd,
    },

    /// etcdctl-compatible CLI shim.
    Etcdctl {
        #[command(subcommand)]
        cmd: etcdctl::EtcdctlCmd,
    },

    /// cilium CLI shim.
    Cilium {
        #[command(subcommand)]
        cmd: cilium::CiliumCmd,
    },

    /// helm v3 CLI shim.
    Helm {
        #[command(subcommand)]
        cmd: helm::HelmCmd,
    },

    /// Argo CD CLI shim.
    Argocd {
        #[command(subcommand)]
        cmd: argocd::ArgoCdCmd,
    },

    /// Vault / OpenBao CLI shim.
    Vault {
        #[command(subcommand)]
        cmd: vault::VaultCmd,
    },

    /// Harbor / OCI registry CLI shim.
    Harbor {
        #[command(subcommand)]
        cmd: harbor::HarborCmd,
    },

    /// Strimzi (Kafka on K8s) CLI shim.
    Strimzi {
        #[command(subcommand)]
        cmd: strimzi::StrimziCmd,
    },

    /// Talos Linux runtime ops CLI shim.
    Talosctl {
        #[command(subcommand)]
        cmd: talosctl::TalosCmd,
    },

    // ── Utility ─────────────────────────────────────────────────────────────

    /// Print a shell completion script.
    Completion(CompletionArgs),
}

// ── kubectl subcommand bridge ────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
enum KubectlCmd {
    Get(kubectl::get::KubectlGetArgs),
    Describe(kubectl::describe::KubectlDescribeArgs),
    Create(kubectl::create::KubectlCreateArgs),
    Apply(kubectl::apply::KubectlApplyArgs),
    Delete(kubectl::delete::KubectlDeleteArgs),
    Exec(kubectl::exec::KubectlExecArgs),
    Logs(kubectl::logs::KubectlLogsArgs),
    #[command(name = "port-forward")]
    PortForward(kubectl::port_forward::KubectlPortForwardArgs),
    Cp(kubectl::cp::KubectlCpArgs),
}

// ── Completion args ──────────────────────────────────────────────────────────

#[derive(Args, Debug)]
struct CompletionArgs {
    #[arg(value_enum)]
    shell: ShellArg,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum ShellArg {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

impl From<ShellArg> for cavectl::shell::Shell {
    fn from(s: ShellArg) -> Self {
        match s {
            ShellArg::Bash => cavectl::shell::Shell::Bash,
            ShellArg::Zsh => cavectl::shell::Shell::Zsh,
            ShellArg::Fish => cavectl::shell::Shell::Fish,
            ShellArg::Powershell => cavectl::shell::Shell::PowerShell,
        }
    }
}

// ── entrypoint ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ {:#}", e);
            ExitCode::from(1)
        }
    }
}

async fn dispatch(cli: Cli) -> Result<()> {
    let client = reqwest::Client::new();

    // Handle non-HTTP utility commands first.
    if let TopCmd::Completion(c) = &cli.cmd {
        let script = cavectl::shell::generate(c.shell.into(), "cavectl")?;
        println!("{}", script);
        return Ok(());
    }

    let output = OutputFormat::parse(&cli.output)?;

    // Build the PreparedRequest. `prepare()` is the lib's contract;
    // every shim returns one. Read-from-disk side effects (apply/create
    // manifests) happen here at the boundary.
    let req = build_request(&cli.cmd)?;

    execute(&client, &cli.server, cli.token.as_deref(), req, output).await
}

fn build_request(cmd: &TopCmd) -> Result<PreparedRequest> {
    Ok(match cmd {
        // Native
        TopCmd::Deploy(args) => native::deploy::prepare(args)?,
        TopCmd::Get(args) => native::get::prepare(args)?,
        TopCmd::Describe(args) => native::describe::prepare(args)?,
        TopCmd::Logs(args) => native::logs::prepare(args)?,
        TopCmd::Events(args) => native::events::prepare(args)?,
        TopCmd::Secrets { cmd } => native::secrets::prepare(cmd)?,
        TopCmd::Flag { cmd } => native::flag::prepare(cmd)?,
        TopCmd::Chaos { cmd } => native::chaos::prepare(cmd)?,
        TopCmd::Topology(args) => native::topology::prepare(args)?,

        // Compat
        TopCmd::Kubectl { cmd } => build_kubectl(cmd)?,
        TopCmd::Etcdctl { cmd } => etcdctl::prepare(cmd)?,
        TopCmd::Cilium { cmd } => cilium::prepare(cmd)?,
        TopCmd::Helm { cmd } => helm::prepare(cmd)?,
        TopCmd::Argocd { cmd } => argocd::prepare(cmd)?,
        TopCmd::Vault { cmd } => vault::prepare(cmd)?,
        TopCmd::Harbor { cmd } => harbor::prepare(cmd)?,
        TopCmd::Strimzi { cmd } => strimzi::prepare(cmd)?,
        TopCmd::Talosctl { cmd } => talosctl::prepare(cmd)?,

        TopCmd::Completion(_) => unreachable!("handled above"),
    })
}

fn build_kubectl(cmd: &KubectlCmd) -> Result<PreparedRequest> {
    Ok(match cmd {
        KubectlCmd::Get(args) => kubectl::get::prepare(args)?.0,
        KubectlCmd::Describe(args) => kubectl::describe::prepare(args)?,
        KubectlCmd::Create(args) => {
            let body = read_manifest(&args.filename)?;
            let manifest = kubectl::create::parse_manifest(&body)?;
            kubectl::create::prepare(args, &manifest)?
        }
        KubectlCmd::Apply(args) => {
            let body = read_manifest(&args.filename)?;
            let manifest = kubectl::create::parse_manifest(&body)?;
            kubectl::apply::prepare(args, &manifest)?
        }
        KubectlCmd::Delete(args) => kubectl::delete::prepare(args)?,
        KubectlCmd::Exec(args) => kubectl::exec::prepare(args)?,
        KubectlCmd::Logs(args) => kubectl::logs::prepare(args)?,
        KubectlCmd::PortForward(args) => kubectl::port_forward::prepare(args)?,
        KubectlCmd::Cp(args) => kubectl::cp::prepare(args)?.0,
    })
}

/// Read a manifest from a file, or stdin when `filename == "-"`.
fn read_manifest(filename: &str) -> Result<String> {
    if filename == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read manifest from stdin")?;
        Ok(buf)
    } else {
        std::fs::read_to_string(filename)
            .with_context(|| format!("failed to read manifest from {}", filename))
    }
}

// ── HTTP execute ─────────────────────────────────────────────────────────────

async fn execute(
    client: &reqwest::Client,
    server: &str,
    token: Option<&str>,
    req: PreparedRequest,
    output: OutputFormat,
) -> Result<()> {
    let url = format!("{}{}", server.trim_end_matches('/'), req.path);
    let method = match req.verb {
        HttpVerb::Get => Method::GET,
        HttpVerb::Post => Method::POST,
        HttpVerb::Put => Method::PUT,
        HttpVerb::Patch => Method::PATCH,
        HttpVerb::Delete => Method::DELETE,
    };
    let mut rb = client.request(method, &url);
    if let Some(t) = token {
        rb = rb.bearer_auth(t);
    }
    if let Some(body) = req.body {
        rb = rb.json(&body);
    }
    let resp = rb
        .send()
        .await
        .with_context(|| format!("failed to connect to {}", url))?;
    let status = resp.status();
    let body_text = resp.text().await.context("read body")?;
    if !status.is_success() {
        let extra = if body_text.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", body_text.trim())
        };
        let hint = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                " (set CAVE_TOKEN or --token)"
            }
            _ => "",
        };
        anyhow::bail!("HTTP {}{}{}", status.as_u16(), hint, extra);
    }
    if body_text.trim().is_empty() {
        return Ok(());
    }
    match serde_json::from_str::<Value>(&body_text) {
        Ok(v) => print_output(&v, output),
        Err(_) => println!("{}", body_text),
    }
    Ok(())
}

// ── Output rendering ─────────────────────────────────────────────────────────

fn print_output(v: &Value, format: OutputFormat) {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(v).unwrap_or_default());
        }
        OutputFormat::Name => print_names(v),
        OutputFormat::Wide | OutputFormat::Table => {
            println!("{}", render_table(v));
        }
    }
}

fn print_names(v: &Value) {
    match v {
        Value::Array(arr) => {
            for item in arr {
                if let Some(n) = name_of(item) {
                    println!("{}", n);
                }
            }
        }
        // Some servers wrap collections in `{ items: [...] }`.
        Value::Object(map) if map.get("items").map(|i| i.is_array()).unwrap_or(false) => {
            if let Some(arr) = map.get("items").and_then(|i| i.as_array()) {
                for item in arr {
                    if let Some(n) = name_of(item) {
                        println!("{}", n);
                    }
                }
            }
        }
        _ => {
            if let Some(n) = name_of(v) {
                println!("{}", n);
            }
        }
    }
}

fn name_of(v: &Value) -> Option<String> {
    let obj = v.as_object()?;
    obj.get("name")
        .and_then(|x| x.as_str())
        .map(String::from)
        .or_else(|| {
            obj.get("metadata")
                .and_then(|m| m.as_object())
                .and_then(|m| m.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
}

fn render_table(v: &Value) -> String {
    match v {
        Value::Array(arr) if arr.is_empty() => "(empty)".to_string(),
        Value::Array(arr) => render_array(arr),
        Value::Object(map) if map.get("items").map(|i| i.is_array()).unwrap_or(false) => {
            // Conventional Kubernetes-style envelope.
            let arr = map.get("items").and_then(|i| i.as_array()).unwrap();
            if arr.is_empty() {
                "(empty)".to_string()
            } else {
                render_array(arr)
            }
        }
        Value::Object(map) if map.is_empty() => "(empty)".to_string(),
        Value::Object(map) => render_kv(map),
        Value::Null => "(null)".to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn render_array(arr: &[Value]) -> String {
    let first_obj = match arr.first() {
        Some(Value::Object(o)) => o,
        _ => {
            return arr
                .iter()
                .enumerate()
                .map(|(i, v)| format!("{}  {}", i + 1, cell_str(v)))
                .collect::<Vec<_>>()
                .join("\n");
        }
    };
    let keys: Vec<&str> = first_obj.keys().map(String::as_str).collect();
    if keys.is_empty() {
        return "(empty)".to_string();
    }
    let mut widths: Vec<usize> = keys.iter().map(|k| k.len()).collect();
    for row in arr {
        if let Value::Object(obj) = row {
            for (i, key) in keys.iter().enumerate() {
                let w = char_count(&cell_str(obj.get(*key).unwrap_or(&Value::Null))).min(60);
                widths[i] = widths[i].max(w);
            }
        }
    }
    let mut out = String::new();
    let header: String = keys
        .iter()
        .zip(&widths)
        .map(|(k, w)| format!("{:<width$}", k.to_uppercase(), width = w))
        .collect::<Vec<_>>()
        .join("  ");
    out.push_str(&header);
    out.push('\n');
    for row in arr {
        if let Value::Object(obj) = row {
            let cells: String = keys
                .iter()
                .zip(&widths)
                .map(|(k, w)| {
                    let s = cell_str(obj.get(*k).unwrap_or(&Value::Null));
                    let t = trunc(&s, 60);
                    format!("{:<width$}", t, width = w)
                })
                .collect::<Vec<_>>()
                .join("  ");
            out.push_str(&cells);
            out.push('\n');
        }
    }
    out
}

fn render_kv(map: &serde_json::Map<String, Value>) -> String {
    let key_width = map.keys().map(|k| k.len()).max().unwrap_or(0);
    map.iter()
        .map(|(k, v)| format!("{:<width$}  {}", k, cell_str(v), width = key_width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn cell_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(_) => "{…}".to_string(),
    }
}

fn char_count(s: &str) -> usize {
    s.chars().count()
}

fn trunc(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > max && max > 0 {
        let t: String = chars[..max - 1].iter().collect();
        format!("{}…", t)
    } else {
        s.to_string()
    }
}
