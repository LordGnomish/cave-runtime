//! Production runtime mode driven by `<data_dir>/cluster.json`.
//!
//! When `cave-runtime serve` is launched on a data directory that was
//! provisioned with `cave-runtime cluster init`, this module:
//!
//! * loads the cluster manifest (kubeconfig path, advertise address, etc.),
//! * restores the cave-etcd KvStore from the latest snapshot on disk,
//! * spawns a dedicated TLS listener for cave-etcd on port 2379,
//! * spawns a dedicated TLS listener for cave-apiserver on port 6443
//!   (with `/healthz` and `/api/v1/bootstrap/join`),
//! * persists an etcd snapshot on Ctrl-C / SIGTERM.
//!
//! The unified runtime HTTP server on port 8080 (portal, admin endpoints,
//! and the legacy merged routers) continues to run side-by-side.

use anyhow::{anyhow, Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_server::tls_rustls::RustlsConfig;
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use serde::{Deserialize, Serialize};
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tracing::{info, warn};

use crate::cluster::ClusterManifest;

const ETCD_PORT: u16 = 2379;
const APISERVER_PORT: u16 = 6443;

/// Loaded cluster context that drives the production-mode listeners.
#[derive(Clone)]
pub struct ClusterRuntime {
    pub manifest: ClusterManifest,
    pub data_dir: PathBuf,
    pub etcd_store: Arc<cave_etcd::store::KvStore>,
    pub apiserver_store: Arc<cave_apiserver::store::ResourceStore>,
    pub bootstrap_tokens: Vec<String>,
}

impl ClusterRuntime {
    /// Locate `cluster.json` in (priority order):
    /// 1. explicit `data_dir` arg
    /// 2. `CAVE_DATA_DIR` env var
    /// 3. `$HOME/.cave/`
    /// Returns `Ok(None)` when no manifest is found — caller falls back to
    /// development mode.
    pub async fn load(data_dir: Option<&Path>) -> Result<Option<Self>> {
        let candidates = candidate_dirs(data_dir);
        for dd in candidates {
            let manifest_path = dd.join("cluster.json");
            if !manifest_path.exists() {
                continue;
            }
            let raw = fs::read_to_string(&manifest_path)
                .await
                .with_context(|| format!("read {}", manifest_path.display()))?;
            let manifest: ClusterManifest =
                serde_json::from_str(&raw).context("parse cluster.json")?;

            let etcd_store = restore_etcd_store(&dd).await?;
            let apiserver_store = Arc::new(cave_apiserver::store::ResourceStore::new());
            let bootstrap_tokens = load_bootstrap_tokens(&dd).await.unwrap_or_default();

            info!(
                data_dir = %dd.display(),
                cluster = %manifest.cluster_name,
                tokens = bootstrap_tokens.len(),
                "production-mode cluster runtime loaded"
            );
            return Ok(Some(Self {
                manifest,
                data_dir: dd,
                etcd_store,
                apiserver_store,
                bootstrap_tokens,
            }));
        }
        Ok(None)
    }

    /// Build the TLS-terminated etcd listener and the apiserver listener,
    /// returning JoinHandles for each.
    pub async fn spawn_listeners(self) -> Result<Vec<tokio::task::JoinHandle<Result<()>>>> {
        // Install a process-wide rustls CryptoProvider once.
        install_default_crypto_provider();

        let etcd_cfg = load_rustls_config(
            &self.data_dir.join("pki/etcd.crt"),
            &self.data_dir.join("pki/etcd.key"),
        )
        .await?;
        let api_cfg = load_rustls_config(
            &self.data_dir.join("pki/apiserver.crt"),
            &self.data_dir.join("pki/apiserver.key"),
        )
        .await?;

        let advertise_ip = parse_listen_ip(&self.manifest.advertise_address);
        let etcd_addr = SocketAddr::new(advertise_ip, ETCD_PORT);
        let api_addr = SocketAddr::new(advertise_ip, APISERVER_PORT);

        let etcd_router = etcd_router(self.etcd_store.clone());
        let api_router = apiserver_router(self.apiserver_store.clone(), self.bootstrap_tokens.clone());

        let etcd_handle = tokio::spawn(async move {
            info!(addr = %etcd_addr, "cave-etcd TLS listener starting");
            axum_server::bind_rustls(etcd_addr, etcd_cfg)
                .serve(etcd_router.into_make_service())
                .await
                .map_err(|e| anyhow!("etcd listener: {e}"))
        });
        let api_handle = tokio::spawn(async move {
            info!(addr = %api_addr, "cave-apiserver TLS listener starting");
            axum_server::bind_rustls(api_addr, api_cfg)
                .serve(api_router.into_make_service())
                .await
                .map_err(|e| anyhow!("apiserver listener: {e}"))
        });

        // Background: snapshot the etcd store every 60s.
        let dd = self.data_dir.clone();
        let store_for_snapshot = self.etcd_store.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(60));
            tick.tick().await; // skip the immediate first tick
            loop {
                tick.tick().await;
                if let Err(e) = persist_etcd_snapshot(&dd, &store_for_snapshot).await {
                    warn!(error = %e, "periodic etcd snapshot failed");
                }
            }
        });

        Ok(vec![etcd_handle, api_handle])
    }

    /// Flush a final etcd snapshot to disk (called from the SIGINT handler).
    pub async fn shutdown_persist(&self) -> Result<()> {
        persist_etcd_snapshot(&self.data_dir, &self.etcd_store).await
    }
}

fn candidate_dirs(explicit: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = explicit {
        out.push(p.to_path_buf());
    }
    if let Ok(env_dir) = std::env::var("CAVE_DATA_DIR") {
        out.push(PathBuf::from(env_dir));
    }
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join(".cave"));
    }
    out
}

fn parse_listen_ip(advertise: &str) -> std::net::IpAddr {
    advertise
        .split(':')
        .next()
        .and_then(|h| h.parse().ok())
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)))
}

fn install_default_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // ring provider is selected by the `ring` feature in our rustls dep.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

async fn load_rustls_config(cert_path: &Path, key_path: &Path) -> Result<RustlsConfig> {
    install_default_crypto_provider();
    let cert_pem = fs::read(cert_path)
        .await
        .with_context(|| format!("read {}", cert_path.display()))?;
    let key_pem = fs::read(key_path)
        .await
        .with_context(|| format!("read {}", key_path.display()))?;

    let cert_chain: Vec<_> = certs(&mut BufReader::new(&cert_pem[..]))
        .collect::<std::io::Result<Vec<_>>>()
        .context("parse cert PEM")?;
    let mut keys: Vec<_> = pkcs8_private_keys(&mut BufReader::new(&key_pem[..]))
        .collect::<std::io::Result<Vec<_>>>()
        .context("parse key PEM")?;
    if keys.is_empty() {
        return Err(anyhow!(
            "no PKCS#8 private key found in {}",
            key_path.display()
        ));
    }
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(keys.remove(0));

    let server_cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("rustls ServerConfig build")?;
    Ok(RustlsConfig::from_config(Arc::new(server_cfg)))
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

async fn restore_etcd_store(data_dir: &Path) -> Result<Arc<cave_etcd::store::KvStore>> {
    let snap_path = data_dir.join("etcd").join("snapshot.bin");
    if snap_path.exists() {
        let bytes = fs::read(&snap_path)
            .await
            .with_context(|| format!("read {}", snap_path.display()))?;
        match cave_etcd::snap_db::restore_into_store(&bytes) {
            Ok((store, header)) => {
                info!(
                    revision = header.revision,
                    bytes = bytes.len(),
                    "cave-etcd restored from snapshot"
                );
                return Ok(Arc::new(store));
            }
            Err(e) => {
                warn!(error = %e, "etcd snapshot restore failed — starting empty");
            }
        }
    }
    Ok(Arc::new(cave_etcd::store::KvStore::new()))
}

pub async fn persist_etcd_snapshot(
    data_dir: &Path,
    store: &cave_etcd::store::KvStore,
) -> Result<()> {
    let etcd_dir = data_dir.join("etcd");
    fs::create_dir_all(&etcd_dir).await.ok();
    // cluster_id 0 is a placeholder — single-node MVP doesn't run Raft yet.
    let bytes = cave_etcd::snap_db::save_from_store(store, 0);
    let snap_path = etcd_dir.join("snapshot.bin");
    let tmp_path = etcd_dir.join("snapshot.bin.tmp");
    fs::write(&tmp_path, &bytes).await?;
    fs::rename(&tmp_path, &snap_path).await?;
    info!(bytes = bytes.len(), path = %snap_path.display(), "cave-etcd snapshot persisted");
    Ok(())
}

async fn load_bootstrap_tokens(data_dir: &Path) -> Result<Vec<String>> {
    let path = data_dir.join("bootstrap-tokens.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path).await?;
    let parsed: BootstrapTokenFile = serde_json::from_str(&raw)?;
    Ok(parsed.tokens.into_iter().map(|t| t.token).collect())
}

#[derive(Serialize, Deserialize)]
pub struct BootstrapTokenFile {
    pub tokens: Vec<BootstrapTokenEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct BootstrapTokenEntry {
    pub token: String,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Listener routers
// ---------------------------------------------------------------------------

fn etcd_router(state: Arc<cave_etcd::store::KvStore>) -> Router {
    cave_etcd::router(state).route("/healthz", get(|| async { "ok\n" }))
}

#[derive(Clone)]
struct ApiserverListenerState {
    resources: Arc<cave_apiserver::store::ResourceStore>,
    bootstrap_tokens: Arc<Vec<String>>,
}

fn apiserver_router(
    resources: Arc<cave_apiserver::store::ResourceStore>,
    bootstrap_tokens: Vec<String>,
) -> Router {
    let listener_state = ApiserverListenerState {
        resources: resources.clone(),
        bootstrap_tokens: Arc::new(bootstrap_tokens),
    };

    // cave-apiserver already mounts `/healthz` and `/readyz` — only add
    // `/livez` and the bootstrap-join endpoint here.
    cave_apiserver::router(resources)
        .route("/livez", get(healthz))
        .route(
            "/api/v1/bootstrap/join",
            post(bootstrap_join).with_state(listener_state),
        )
}

async fn healthz() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
pub struct BootstrapJoinRequest {
    pub token: String,
    pub node_name: String,
}

#[derive(Serialize)]
pub struct BootstrapJoinResponse {
    pub status: String,
    pub node_name: String,
    pub message: String,
}

async fn bootstrap_join(
    State(state): State<ApiserverListenerState>,
    Json(req): Json<BootstrapJoinRequest>,
) -> Result<Json<BootstrapJoinResponse>, (StatusCode, String)> {
    if !state.bootstrap_tokens.iter().any(|t| t == &req.token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid bootstrap token".into()));
    }
    if req.node_name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "node_name required".into()));
    }
    // MVP: register the node via the resource store. A real implementation
    // would mint a per-node TLS leaf from the cluster CA here; today we
    // accept the token and return success so the worker can keep going.
    let node = cave_apiserver::resources::Resource::Node(cave_apiserver::resources::Node {
        api_version: "v1".into(),
        kind: "Node".into(),
        metadata: cave_apiserver::resources::ObjectMeta::new(&req.node_name, ""),
        spec: cave_apiserver::resources::NodeSpec::default(),
        status: cave_apiserver::resources::NodeStatus::default(),
    });
    let _ = state.resources.create(node);
    info!(node = %req.node_name, "bootstrap-token accepted");
    Ok(Json(BootstrapJoinResponse {
        status: "accepted".into(),
        node_name: req.node_name,
        message: "node registered; full kubelet certificate issuance is pending the kubelet CSR controller".into(),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::init;
    use tempfile::TempDir;

    #[tokio::test]
    async fn load_returns_none_when_no_manifest() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("empty");
        let res = ClusterRuntime::load(Some(&dd)).await.unwrap();
        assert!(res.is_none());
    }

    #[tokio::test]
    async fn load_succeeds_after_init() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "load-test", "127.0.0.1:6443").unwrap();
        let rt = ClusterRuntime::load(Some(&dd))
            .await
            .unwrap()
            .expect("manifest must load");
        assert_eq!(rt.manifest.cluster_name, "load-test");
        assert_eq!(rt.bootstrap_tokens.len(), 1);
    }

    #[tokio::test]
    async fn etcd_snapshot_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        init(&dd, "snap-test", "127.0.0.1:6443").unwrap();
        let rt = ClusterRuntime::load(Some(&dd)).await.unwrap().unwrap();

        // Put one key, persist, reload, verify.
        let put_req = cave_etcd::models::PutRequest {
            key: cave_etcd::b64::encode(b"hello"),
            value: cave_etcd::b64::encode(b"world"),
            lease: None,
            prev_kv: false,
        };
        rt.etcd_store.put(&put_req);
        rt.shutdown_persist().await.unwrap();
        assert!(dd.join("etcd/snapshot.bin").exists());

        let rt2 = ClusterRuntime::load(Some(&dd)).await.unwrap().unwrap();
        let range_req = cave_etcd::models::RangeRequest {
            key: cave_etcd::b64::encode(b"hello"),
            range_end: None,
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        };
        let resp = rt2.etcd_store.range(&range_req).expect("range");
        assert_eq!(resp.count, 1, "snapshot must round-trip the key");
    }

    #[tokio::test]
    async fn bootstrap_join_rejects_bad_token() {
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["good-token-1234567890".to_string()]),
        };
        let resp = bootstrap_join(
            State(listener_state.clone()),
            Json(BootstrapJoinRequest {
                token: "bad".into(),
                node_name: "worker-1".into(),
            }),
        )
        .await;
        match resp {
            Err((status, _)) => assert_eq!(status, StatusCode::UNAUTHORIZED),
            Ok(_) => panic!("must reject"),
        }
    }

    #[tokio::test]
    async fn bootstrap_join_accepts_good_token() {
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["good-token-1234567890".to_string()]),
        };
        let resp = bootstrap_join(
            State(listener_state),
            Json(BootstrapJoinRequest {
                token: "good-token-1234567890".into(),
                node_name: "worker-1".into(),
            }),
        )
        .await
        .expect("must accept");
        assert_eq!(resp.status, "accepted");
        assert_eq!(resp.node_name, "worker-1");
    }

    #[tokio::test]
    async fn bootstrap_join_requires_node_name() {
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["good-token-1234567890".to_string()]),
        };
        let resp = bootstrap_join(
            State(listener_state),
            Json(BootstrapJoinRequest {
                token: "good-token-1234567890".into(),
                node_name: "".into(),
            }),
        )
        .await;
        match resp {
            Err((status, _)) => assert_eq!(status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("must reject empty node name"),
        }
    }
}
