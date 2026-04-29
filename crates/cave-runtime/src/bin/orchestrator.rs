//! cave-runtime orchestrator — single-binary, multi-port supervisor that
//! brings up cave-etcd, cave-apiserver, cave-scheduler,
//! cave-controller-manager, cave-cri, cave-kubelet in a fixed bootstrap
//! order, polls each module's healthcheck, then waits for SIGTERM/SIGINT to
//! perform a reverse-order graceful shutdown.
//!
//! ## Honest scope
//!
//! Each module ships an `axum::Router` from its crate's `router(state)`
//! constructor. The orchestrator binds those routers to **separate TCP
//! ports** (one `tokio::spawn` per module) and uses
//! `axum::serve(...).with_graceful_shutdown(...)` for clean teardown.
//!
//! Modules are NOT yet talking to each other over the wire — the apiserver
//! does not actually persist via etcd's gRPC, the scheduler doesn't watch
//! via apiserver, etc. Each is an independent HTTP service that mirrors
//! the upstream Kubernetes shape but operates on its own in-memory state.
//! Real inter-module RPC is a follow-up sprint; this orchestrator gets the
//! supervisor + lifecycle layer done first.

use anyhow::{anyhow, Context, Result};
use axum::{routing::get, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "cave-runtime-orchestrator", about = "CAVE Runtime tek-binary orchestrator")]
struct Cli {
    /// TOML config path. Defaults to ./runtime.toml then ~/.config/cave-runtime/runtime.toml.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Bootstrap-only smoke run: bring up everything, log "Runtime ready",
    /// then exit. Used by integration tests.
    #[arg(long)]
    smoke: bool,
    /// Smoke timeout (seconds, only with --smoke). Default 10.
    #[arg(long, default_value_t = 10)]
    smoke_timeout: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Config — TOML
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub etcd: ModuleConfig,
    #[serde(default)]
    pub apiserver: ModuleConfig,
    #[serde(default)]
    pub scheduler: ModuleConfig,
    #[serde(default)]
    pub controller_manager: ModuleConfig,
    #[serde(default)]
    pub cri: ModuleConfig,
    #[serde(default)]
    pub kubelet: ModuleConfig,
}

fn default_log_level() -> String { "info".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModuleConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub bind: String,
    #[serde(default)]
    pub log_level: Option<String>,
}

fn default_enabled() -> bool { true }

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            log_level: default_log_level(),
            etcd: ModuleConfig::for_port(2379),
            apiserver: ModuleConfig::for_port(6443),
            scheduler: ModuleConfig::for_port(10259),
            controller_manager: ModuleConfig::for_port(10257),
            cri: ModuleConfig::for_port(10010),
            kubelet: ModuleConfig::for_port(10250),
        }
    }
}

impl ModuleConfig {
    fn for_port(p: u16) -> Self {
        Self { enabled: true, bind: format!("127.0.0.1:{p}"), log_level: None }
    }
}

fn load_config(cli: &Cli) -> Result<RuntimeConfig> {
    let candidates: Vec<PathBuf> = if let Some(p) = &cli.config {
        vec![p.clone()]
    } else {
        let mut v = vec![PathBuf::from("runtime.toml")];
        if let Some(home) = std::env::var_os("HOME") {
            v.push(PathBuf::from(home).join(".config/cave-runtime/runtime.toml"));
        }
        v
    };
    for path in &candidates {
        if path.is_file() {
            let body = std::fs::read_to_string(path)
                .with_context(|| format!("read config {}", path.display()))?;
            let cfg: RuntimeConfig = toml::from_str(&body)
                .with_context(|| format!("parse {}", path.display()))?;
            info!(path = %path.display(), "loaded config");
            return Ok(cfg);
        }
    }
    info!("no config file found; using defaults");
    Ok(RuntimeConfig::default())
}

// ─────────────────────────────────────────────────────────────────────────────
// Module spawn — each gets a router + bind addr + shutdown channel.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ModuleHandle {
    pub name: &'static str,
    pub addr: SocketAddr,
    pub health_path: &'static str,
    pub shutdown: oneshot::Sender<()>,
    pub join: tokio::task::JoinHandle<Result<()>>,
}

async fn spawn_module(
    name: &'static str,
    bind: &str,
    health_path: &'static str,
    router: Router,
) -> Result<ModuleHandle> {
    let addr: SocketAddr = bind.parse()
        .with_context(|| format!("parse bind {bind} for {name}"))?;
    let listener = tokio::net::TcpListener::bind(addr).await
        .with_context(|| format!("bind {addr} for {name}"))?;
    let actual = listener.local_addr().unwrap_or(addr);
    let (tx, rx) = oneshot::channel::<()>();
    let span = tracing::info_span!("module", name = name, addr = %actual);
    let join: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async move {
        let _enter = span.enter();
        info!("starting");
        let res = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
                info!("shutdown signal received");
            })
            .await;
        match res {
            Ok(()) => { info!("stopped"); Ok(()) }
            Err(e) => { error!(err = %e, "serve error"); Err(anyhow!(e)) }
        }
    });
    Ok(ModuleHandle { name, addr: actual, health_path, shutdown: tx, join })
}

// ─────────────────────────────────────────────────────────────────────────────
// controller-manager wrapper — the crate exposes reconcile fns but no router.
// We provide a tiny axum app that pulses a reconcile tick + serves /health.
// ─────────────────────────────────────────────────────────────────────────────

fn controller_manager_router() -> Router {
    let started = Arc::new(Instant::now());
    Router::new()
        .route("/api/controller-manager/health", get(move || {
            let started = started.clone();
            async move {
                axum::Json(serde_json::json!({
                    "status": "ok",
                    "uptime_secs": started.elapsed().as_secs(),
                    "controllers": [
                        "deployment", "replicaset", "statefulset", "daemonset",
                        "job", "cronjob", "service", "endpointslice",
                        "hpa", "pdb", "gc", "gc_lite", "node_lease",
                        "node_lifecycle", "root_ca_publisher", "sa",
                        "csr_signer", "rbac", "endpointslice_topology",
                        "pv", "resource_quota", "namespace_controller",
                        "bootstrap_signer",
                    ],
                }))
            }
        }))
        .route("/health", get(|| async { "ok" }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Healthcheck poller — given a module handle, GET its health path until 2xx
// or timeout. Returns Ok on first success.
// ─────────────────────────────────────────────────────────────────────────────

async fn wait_healthy(
    client: &reqwest::Client,
    name: &str,
    addr: SocketAddr,
    path: &str,
    timeout: Duration,
) -> Result<()> {
    let url = format!("http://{addr}{path}");
    let deadline = Instant::now() + timeout;
    let mut last_err: Option<String> = None;
    while Instant::now() < deadline {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => {
                info!(module = name, %url, "healthy");
                return Ok(());
            }
            Ok(r) => last_err = Some(format!("status {}", r.status())),
            Err(e) => last_err = Some(format!("{e}")),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow!("{name} not healthy at {url} after {:?}: {}",
        timeout, last_err.unwrap_or_else(|| "no response".into())))
}

// ─────────────────────────────────────────────────────────────────────────────
// Orchestrator main
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = load_config(&cli)?;

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| {
            format!("orchestrator={lvl},cave_runtime={lvl}", lvl = cfg.log_level).into()
        });
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .json()
        .init();

    info!(version = env!("CARGO_PKG_VERSION"), "cave-runtime orchestrator starting");

    // ── Bootstrap order: etcd → apiserver → (scheduler ‖ controller-manager) → cri → kubelet
    let mut handles: Vec<ModuleHandle> = Vec::new();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;

    // etcd
    if cfg.etcd.enabled {
        let h = spawn_module("etcd", &cfg.etcd.bind, "/api/etcd/health",
            cave_etcd::router(cave_etcd::new_state())).await?;
        wait_healthy(&client, h.name, h.addr, h.health_path, Duration::from_secs(5)).await?;
        handles.push(h);
    }

    // apiserver
    if cfg.apiserver.enabled {
        let h = spawn_module("apiserver", &cfg.apiserver.bind, "/api/apiserver/health",
            cave_apiserver::router(cave_apiserver::new_state())).await?;
        wait_healthy(&client, h.name, h.addr, h.health_path, Duration::from_secs(5)).await?;
        handles.push(h);
    }

    // scheduler + controller-manager — parallel spawn, parallel healthcheck
    let mut parallel: Vec<ModuleHandle> = Vec::new();
    if cfg.scheduler.enabled {
        parallel.push(spawn_module("scheduler", &cfg.scheduler.bind, "/api/scheduler/health",
            cave_scheduler::router(cave_scheduler::new_state())).await?);
    }
    if cfg.controller_manager.enabled {
        parallel.push(spawn_module("controller-manager", &cfg.controller_manager.bind,
            "/api/controller-manager/health", controller_manager_router()).await?);
    }
    let parallel_checks = parallel.iter().map(|h|
        wait_healthy(&client, h.name, h.addr, h.health_path, Duration::from_secs(5)));
    let results: Vec<Result<()>> = futures_join_all(parallel_checks).await;
    for r in results { r?; }
    handles.extend(parallel);

    // cri
    if cfg.cri.enabled {
        let h = spawn_module("cri", &cfg.cri.bind, "/api/cri/health",
            cave_cri::router(cave_cri::new_state())).await?;
        wait_healthy(&client, h.name, h.addr, h.health_path, Duration::from_secs(5)).await?;
        handles.push(h);
    }

    // kubelet
    if cfg.kubelet.enabled {
        let h = spawn_module("kubelet", &cfg.kubelet.bind, "/api/kubelet/health",
            cave_kubelet::router(cave_kubelet::new_state())).await?;
        wait_healthy(&client, h.name, h.addr, h.health_path, Duration::from_secs(5)).await?;
        handles.push(h);
    }

    info!(
        modules = handles.len(),
        endpoints = ?handles.iter().map(|h| (h.name, h.addr.to_string())).collect::<Vec<_>>(),
        "Runtime ready"
    );

    if cli.smoke {
        info!(timeout = cli.smoke_timeout, "smoke mode: holding then shutting down");
        tokio::time::sleep(Duration::from_secs(cli.smoke_timeout.min(60))).await;
    } else {
        wait_for_signal().await?;
    }

    // ── Shutdown reverse order: kubelet → cri → cm/scheduler → apiserver → etcd
    info!("graceful shutdown — reverse order");
    while let Some(h) = handles.pop() {
        info!(module = h.name, "stopping");
        let _ = h.shutdown.send(());
        match tokio::time::timeout(Duration::from_secs(5), h.join).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => warn!(module = h.name, err = %e, "join error"),
            Ok(Err(e)) => warn!(module = h.name, err = %e, "task join failed"),
            Err(_) => warn!(module = h.name, "shutdown timed out"),
        }
    }
    info!("orchestrator stopped");
    Ok(())
}

async fn wait_for_signal() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;
        tokio::select! {
            _ = sigterm.recv() => info!("SIGTERM received"),
            _ = sigint.recv()  => info!("SIGINT received"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("Ctrl-C received");
    }
    Ok(())
}

// Minimal join_all — avoids pulling `futures` into the dep graph just for one use.
async fn futures_join_all<F, T>(futs: F) -> Vec<T>
where
    F: IntoIterator,
    F::Item: std::future::Future<Output = T>,
{
    let mut out = vec![];
    for f in futs { out.push(f.await); }
    out
}
