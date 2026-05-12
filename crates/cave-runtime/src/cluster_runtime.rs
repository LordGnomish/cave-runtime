//! Production runtime mode driven by `<data_dir>/cluster.json`.
//!
//! When `cave-runtime serve` is launched on a data directory that was
//! provisioned with `cave-runtime cluster init`, this module:
//!
//! * loads the cluster manifest (kubeconfig path, advertise address, etc.),
//! * restores the cave-etcd KvStore from the latest snapshot on disk and
//!   replays any WAL entries written since the snapshot,
//! * spawns a dedicated TLS listener for cave-etcd on port 2379 — every PUT
//!   is fsync'd to the WAL by the watch-subscriber task in the background,
//! * spawns a dedicated TLS listener for cave-apiserver on port 6443 with
//!   the bootstrap endpoints (`/api/v1/bootstrap/{ca,join}`) and the CSR
//!   controller (`POST /api/v1/certificatesigningrequests` which signs
//!   bootstrap-token-authenticated CSRs against the cluster CA),
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
    pub ca_cert_pem: String,
    pub ca_key_pem: String,
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

            // Load the cluster CA so the CSR controller can sign kubelet leaves.
            // The init step always writes pki/ca.{crt,key}; if either is
            // missing we still come up, but the CSR endpoint will return 503.
            let ca_cert_pem = fs::read_to_string(dd.join("pki/ca.crt"))
                .await
                .unwrap_or_default();
            let ca_key_pem = fs::read_to_string(dd.join("pki/ca.key"))
                .await
                .unwrap_or_default();

            info!(
                data_dir = %dd.display(),
                cluster = %manifest.cluster_name,
                tokens = bootstrap_tokens.len(),
                ca_loaded = !ca_cert_pem.is_empty() && !ca_key_pem.is_empty(),
                bootstrap_strategy = %manifest.bootstrap_strategy,
                peers = manifest.peers.len(),
                "production-mode cluster runtime loaded"
            );
            if manifest.bootstrap_strategy != "single" || !manifest.peers.is_empty() {
                warn!(
                    bootstrap_strategy = %manifest.bootstrap_strategy,
                    peers = manifest.peers.len(),
                    "cluster.json declares multi-node settings, but multi-node Raft is not yet wired \
                     — falling back to single-node MVP. Peer-replication is a known follow-up."
                );
            }
            return Ok(Some(Self {
                manifest,
                data_dir: dd,
                etcd_store,
                apiserver_store,
                bootstrap_tokens,
                ca_cert_pem,
                ca_key_pem,
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
        let api_router = apiserver_router(
            self.apiserver_store.clone(),
            self.bootstrap_tokens.clone(),
            self.ca_cert_pem.clone(),
            self.ca_key_pem.clone(),
        );

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

        // Background: WAL writer — subscribes to the store's broadcast and
        // appends every Put/Delete event to disk with fsync.
        let wal_path = self.data_dir.join("etcd/wal.log");
        let wal_rx = self.etcd_store.subscribe();
        tokio::spawn(async move {
            if let Err(e) = wal::run_writer(wal_path, wal_rx).await {
                warn!(error = %e, "etcd WAL writer task ended");
            }
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
    let etcd_dir = data_dir.join("etcd");
    let snap_path = etcd_dir.join("snapshot.bin");
    let wal_path = etcd_dir.join("wal.log");

    let (store, snap_rev) = if snap_path.exists() {
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
                (store, header.revision)
            }
            Err(e) => {
                warn!(error = %e, "etcd snapshot restore failed — starting empty");
                (cave_etcd::store::KvStore::new(), 0)
            }
        }
    } else {
        (cave_etcd::store::KvStore::new(), 0)
    };

    // WAL replay — pick up everything written after the snapshot's revision.
    if wal_path.exists() {
        let bytes = fs::read(&wal_path)
            .await
            .with_context(|| format!("read {}", wal_path.display()))?;
        let replayed = wal::replay_into(&bytes, &store, snap_rev);
        if replayed > 0 {
            info!(
                replayed,
                snap_rev,
                wal_bytes = bytes.len(),
                "cave-etcd WAL replayed"
            );
        }
    }

    Ok(Arc::new(store))
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
    ca_cert_pem: Arc<String>,
    ca_key_pem: Arc<String>,
    csrs: Arc<dashmap::DashMap<String, CsrRecord>>,
}

#[derive(Clone, Serialize)]
pub struct CsrRecord {
    pub name: String,
    pub node_name: String,
    pub usage: String,
    pub status: String, // "Approved" | "Denied"
    pub certificate: Option<String>,
    pub created_at: String,
}

fn apiserver_router(
    resources: Arc<cave_apiserver::store::ResourceStore>,
    bootstrap_tokens: Vec<String>,
    ca_cert_pem: String,
    ca_key_pem: String,
) -> Router {
    let listener_state = ApiserverListenerState {
        resources: resources.clone(),
        bootstrap_tokens: Arc::new(bootstrap_tokens),
        ca_cert_pem: Arc::new(ca_cert_pem),
        ca_key_pem: Arc::new(ca_key_pem),
        csrs: Arc::new(dashmap::DashMap::new()),
    };

    // cave-apiserver already mounts `/healthz` and `/readyz` — only add
    // `/livez` and the bootstrap-/CSR-related endpoints here.
    cave_apiserver::router(resources)
        .route("/livez", get(healthz))
        .route(
            "/api/v1/bootstrap/ca",
            get(bootstrap_ca).with_state(listener_state.clone()),
        )
        .route(
            "/api/v1/bootstrap/join",
            post(bootstrap_join).with_state(listener_state.clone()),
        )
        .route(
            "/api/v1/certificatesigningrequests",
            post(submit_csr).with_state(listener_state.clone()),
        )
        .route(
            "/api/v1/certificatesigningrequests/{name}",
            get(get_csr).with_state(listener_state),
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
        message: "node registered; submit a CSR to /api/v1/certificatesigningrequests for a kubelet leaf cert".into(),
    }))
}

// ── CA distribution ────────────────────────────────────────────────────────

async fn bootstrap_ca(
    State(state): State<ApiserverListenerState>,
) -> Result<(StatusCode, [(axum::http::HeaderName, &'static str); 1], String), (StatusCode, String)> {
    if state.ca_cert_pem.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "cluster CA not loaded — `cluster init` did not write pki/ca.crt".into(),
        ));
    }
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/x-pem-file")],
        (*state.ca_cert_pem).clone(),
    ))
}

// ── CSR controller ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CsrRequest {
    pub token: String,
    pub node_name: String,
    pub csr_pem: String,
    pub usage: String, // "kubelet-client" — currently the only accepted usage
}

#[derive(Serialize)]
pub struct CsrResponse {
    pub name: String,
    pub status: String,
    pub certificate: String,
    pub ca: String,
}

async fn submit_csr(
    State(state): State<ApiserverListenerState>,
    Json(req): Json<CsrRequest>,
) -> Result<Json<CsrResponse>, (StatusCode, String)> {
    if !state.bootstrap_tokens.iter().any(|t| t == &req.token) {
        return Err((StatusCode::UNAUTHORIZED, "invalid bootstrap token".into()));
    }
    if req.node_name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "node_name required".into()));
    }
    if req.usage != "kubelet-client" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("usage `{}` not supported (only `kubelet-client`)", req.usage),
        ));
    }
    if state.ca_cert_pem.is_empty() || state.ca_key_pem.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "cluster CA not loaded — cannot sign CSRs".into(),
        ));
    }
    let signed_pem = sign_kubelet_csr(&state.ca_cert_pem, &state.ca_key_pem, &req.csr_pem, &req.node_name)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("sign CSR: {e}")))?;

    let csr_name = format!("csr-{}-{}", req.node_name, uuid::Uuid::new_v4().simple());
    let record = CsrRecord {
        name: csr_name.clone(),
        node_name: req.node_name.clone(),
        usage: req.usage.clone(),
        status: "Approved".into(),
        certificate: Some(signed_pem.clone()),
        created_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
    };
    state.csrs.insert(csr_name.clone(), record);
    info!(node = %req.node_name, csr = %csr_name, "CSR auto-approved + signed");
    Ok(Json(CsrResponse {
        name: csr_name,
        status: "Approved".into(),
        certificate: signed_pem,
        ca: (*state.ca_cert_pem).clone(),
    }))
}

async fn get_csr(
    State(state): State<ApiserverListenerState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<CsrRecord>, (StatusCode, String)> {
    state
        .csrs
        .get(&name)
        .map(|r| Json(r.value().clone()))
        .ok_or((StatusCode::NOT_FOUND, format!("csr `{}` not found", name)))
}

/// Parse a PEM CSR, force the subject to `system:node:<node_name>` /
/// `O=system:nodes`, sign with the cluster CA, and return the leaf cert PEM.
///
/// Forcing the subject means a worker cannot embed an arbitrary CN in its
/// CSR and trick the cluster into issuing it — the token-holder only ever
/// gets `system:node:<their declared node_name>`.
pub fn sign_kubelet_csr(
    ca_cert_pem: &str,
    ca_key_pem: &str,
    csr_pem: &str,
    node_name: &str,
) -> Result<String> {
    use rcgen::{
        CertificateParams, CertificateSigningRequestParams, DnType, ExtendedKeyUsagePurpose,
        KeyPair, KeyUsagePurpose,
    };

    let ca_kp = KeyPair::from_pem(ca_key_pem).context("parse CA private key")?;
    let ca_params =
        CertificateParams::from_ca_cert_pem(ca_cert_pem).context("parse CA cert as rcgen params")?;
    let ca_cert = ca_params
        .self_signed(&ca_kp)
        .context("reconstruct rcgen::Certificate from CA params")?;

    let mut csr =
        CertificateSigningRequestParams::from_pem(csr_pem).context("parse CSR PEM")?;
    csr.params.distinguished_name = rcgen::DistinguishedName::new();
    csr.params
        .distinguished_name
        .push(DnType::CommonName, format!("system:node:{}", node_name));
    csr.params
        .distinguished_name
        .push(DnType::OrganizationName, "system:nodes");
    csr.params.not_before = time::OffsetDateTime::now_utc();
    csr.params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365);
    csr.params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    csr.params.insert_extended_key_usage(ExtendedKeyUsagePurpose::ClientAuth);

    let signed = csr
        .signed_by(&ca_cert, &ca_kp)
        .context("CA sign of kubelet CSR")?;
    Ok(signed.pem())
}

// ---------------------------------------------------------------------------
// WAL
// ---------------------------------------------------------------------------

mod wal {
    //! Per-event append-only log for cave-etcd. Each record is:
    //!
    //! ```text
    //! u32 record_len    (covers everything after this field)
    //! u8  op_type       (0 = Put, 1 = Delete)
    //! u64 revision      (the KvStore revision that produced the event)
    //! u32 key_len
    //! u32 value_len
    //! key bytes (len = key_len)
    //! value bytes (len = value_len)
    //! ```
    //!
    //! Write path: a Tokio task subscribes to `KvStore::subscribe()` and
    //! appends + fsync's every event it sees. The PUT response is sent to
    //! the client before the event reaches the writer — this is an
    //! eventually-durable WAL, not a synchronous one. Acceptable trade-off
    //! for the MVP: the broadcast queue is in-memory and the writer drains
    //! it in microseconds.
    //!
    //! Replay: read records sequentially, stop on EOF or partial record,
    //! re-apply each (Put/Delete) to a fresh `KvStore` *unless* the event's
    //! revision is `<= snapshot_revision` (those are already represented in
    //! the snapshot we just restored from).

    use anyhow::{Context, Result};
    use cave_etcd::models::{EventType, PutRequest, WatchEvent};
    use cave_etcd::store::KvStore;
    use std::path::PathBuf;
    use tokio::fs::OpenOptions;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::broadcast::error::RecvError;
    use tracing::{debug, warn};

    /// Drive the WAL writer. Returns when the broadcast channel closes.
    pub async fn run_writer(
        wal_path: PathBuf,
        mut rx: tokio::sync::broadcast::Receiver<WatchEvent>,
    ) -> Result<()> {
        if let Some(parent) = wal_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)
            .await
            .with_context(|| format!("open WAL {}", wal_path.display()))?;
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let bytes = encode_record(&ev);
                    if let Err(e) = file.write_all(&bytes).await {
                        warn!(error = %e, "WAL write failed — re-opening");
                        continue;
                    }
                    if let Err(e) = file.sync_data().await {
                        warn!(error = %e, "WAL fsync failed");
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    warn!(skipped = n, "WAL subscriber lagged — events lost");
                    continue;
                }
                Err(RecvError::Closed) => {
                    debug!("WAL writer: broadcast closed, exiting");
                    return Ok(());
                }
            }
        }
    }

    /// Replay events from a WAL buffer, skipping events with `rev <= snap_rev`.
    /// Returns the number of events applied.
    pub fn replay_into(buf: &[u8], store: &KvStore, snap_rev: u64) -> usize {
        let mut applied = 0;
        let mut cursor = 0;
        while cursor + 4 <= buf.len() {
            let rec_len = u32::from_le_bytes([
                buf[cursor],
                buf[cursor + 1],
                buf[cursor + 2],
                buf[cursor + 3],
            ]) as usize;
            cursor += 4;
            if cursor + rec_len > buf.len() {
                warn!(
                    cursor,
                    rec_len,
                    buf_len = buf.len(),
                    "WAL: truncated record, stopping replay"
                );
                break;
            }
            let rec = &buf[cursor..cursor + rec_len];
            cursor += rec_len;

            if let Some((op, revision, key, value)) = decode_record(rec) {
                if revision <= snap_rev {
                    continue;
                }
                match op {
                    0 => {
                        store.put(&PutRequest {
                            key: String::from_utf8_lossy(&key).into_owned(),
                            value: String::from_utf8_lossy(&value).into_owned(),
                            lease: None,
                            prev_kv: false,
                        });
                        applied += 1;
                    }
                    1 => {
                        store.delete_range(&cave_etcd::models::DeleteRangeRequest {
                            key: String::from_utf8_lossy(&key).into_owned(),
                            range_end: None,
                            prev_kv: false,
                        });
                        applied += 1;
                    }
                    _ => warn!(op, "WAL: unknown op type, skipping"),
                }
            }
        }
        applied
    }

    fn encode_record(ev: &WatchEvent) -> Vec<u8> {
        let op: u8 = match ev.event_type {
            EventType::Put => 0,
            EventType::Delete => 1,
        };
        let rev = ev.kv.mod_revision;
        let key = &ev.kv.key;
        let value = &ev.kv.value;
        let body_len = 1 + 8 + 4 + 4 + key.len() + value.len();
        let mut out = Vec::with_capacity(4 + body_len);
        out.extend_from_slice(&(body_len as u32).to_le_bytes());
        out.push(op);
        out.extend_from_slice(&rev.to_le_bytes());
        out.extend_from_slice(&(key.len() as u32).to_le_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(value);
        out
    }

    fn decode_record(rec: &[u8]) -> Option<(u8, u64, Vec<u8>, Vec<u8>)> {
        if rec.len() < 1 + 8 + 4 + 4 {
            return None;
        }
        let op = rec[0];
        let rev = u64::from_le_bytes(rec[1..9].try_into().ok()?);
        let key_len = u32::from_le_bytes(rec[9..13].try_into().ok()?) as usize;
        let val_len = u32::from_le_bytes(rec[13..17].try_into().ok()?) as usize;
        if rec.len() < 17 + key_len + val_len {
            return None;
        }
        let key = rec[17..17 + key_len].to_vec();
        let value = rec[17 + key_len..17 + key_len + val_len].to_vec();
        Some((op, rev, key, value))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use cave_etcd::models::KeyValue;

        fn put_event(rev: u64, key: &[u8], value: &[u8]) -> WatchEvent {
            WatchEvent {
                event_type: EventType::Put,
                kv: KeyValue {
                    key: key.to_vec(),
                    value: value.to_vec(),
                    create_revision: rev,
                    mod_revision: rev,
                    version: 1,
                    lease: None,
                },
                prev_kv: None,
            }
        }

        #[test]
        fn record_roundtrips_through_decode() {
            let ev = put_event(42, b"hello", b"world");
            let bytes = encode_record(&ev);
            // strip the 4-byte length prefix to call decode_record directly
            let body = &bytes[4..];
            let (op, rev, k, v) = decode_record(body).unwrap();
            assert_eq!(op, 0);
            assert_eq!(rev, 42);
            assert_eq!(&k[..], b"hello");
            assert_eq!(&v[..], b"world");
        }

        #[test]
        fn replay_skips_events_at_or_below_snap_rev() {
            let mut buf = Vec::new();
            buf.extend(encode_record(&put_event(1, b"a", b"v1")));
            buf.extend(encode_record(&put_event(2, b"b", b"v2")));
            buf.extend(encode_record(&put_event(3, b"c", b"v3")));

            let store = KvStore::new();
            let n = replay_into(&buf, &store, 2);
            assert_eq!(n, 1, "only rev=3 should replay (rev<=2 already snapshotted)");
        }

        #[test]
        fn replay_handles_partial_trailing_record() {
            let mut buf = Vec::new();
            buf.extend(encode_record(&put_event(1, b"a", b"v1")));
            buf.extend(encode_record(&put_event(2, b"b", b"v2")));
            // truncate the last byte to simulate a partial fsync
            buf.pop();

            let store = KvStore::new();
            let n = replay_into(&buf, &store, 0);
            assert_eq!(n, 1, "only the first complete record should replay");
        }
    }
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

    fn test_listener_state(token: &str) -> ApiserverListenerState {
        ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec![token.to_string()]),
            ca_cert_pem: Arc::new(String::new()),
            ca_key_pem: Arc::new(String::new()),
            csrs: Arc::new(dashmap::DashMap::new()),
        }
    }

    #[tokio::test]
    async fn bootstrap_join_rejects_bad_token() {
        let listener_state = test_listener_state("good-token-1234567890");
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
        let resp = bootstrap_join(
            State(test_listener_state("good-token-1234567890")),
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
        let resp = bootstrap_join(
            State(test_listener_state("good-token-1234567890")),
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

    /// End-to-end: generate a worker keypair + CSR, sign it with a freshly
    /// minted cluster CA, verify the leaf carries the locked subject and
    /// chains back to the CA.
    #[test]
    fn sign_kubelet_csr_emits_valid_chained_leaf() {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
        };
        // Cluster CA
        let ca_kp = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::default();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "test cluster CA");
        ca_params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        let ca_cert = ca_params.self_signed(&ca_kp).unwrap();
        let ca_pem = ca_cert.pem();
        let ca_key_pem = ca_kp.serialize_pem();

        // Worker generates its own CSR. Deliberately requests an
        // *attacker-chosen* CN to verify the signer overrides it.
        let worker_kp = KeyPair::generate().unwrap();
        let mut worker_params = CertificateParams::default();
        worker_params
            .distinguished_name
            .push(DnType::CommonName, "system:masters"); // attempt at privilege escalation
        let csr_pem = worker_params
            .serialize_request(&worker_kp)
            .unwrap()
            .pem()
            .unwrap();

        let signed_pem = sign_kubelet_csr(&ca_pem, &ca_key_pem, &csr_pem, "worker-csr-1").unwrap();
        assert!(signed_pem.contains("BEGIN CERTIFICATE"));
        // The CN must be the locked `system:node:<name>`, not the
        // attacker-chosen `system:masters`.
        // We don't want to pull in another x509 crate just to assert this,
        // so we parse the leaf with rcgen's own params + check the DN.
        // (rcgen's `from_ca_cert_pem` will load *any* cert, not just CAs.)
        let leaf_params = rcgen::CertificateParams::from_ca_cert_pem(&signed_pem).unwrap();
        let dn_iter: Vec<_> = leaf_params.distinguished_name.iter().collect();
        let cn = dn_iter
            .iter()
            .find_map(|(t, v)| {
                if matches!(t, DnType::CommonName) {
                    match v {
                        rcgen::DnValue::PrintableString(s) => Some(s.as_str().to_string()),
                        rcgen::DnValue::Utf8String(s) => Some(s.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .unwrap_or_default();
        assert_eq!(cn, "system:node:worker-csr-1");
    }

    #[tokio::test]
    async fn submit_csr_rejects_bad_token() {
        // Mint a CA so the endpoint doesn't short-circuit on missing CA.
        let (ca_pem, ca_key_pem) = mint_test_ca();
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["good-token-1234567890".into()]),
            ca_cert_pem: Arc::new(ca_pem),
            ca_key_pem: Arc::new(ca_key_pem),
            csrs: Arc::new(dashmap::DashMap::new()),
        };
        let resp = submit_csr(
            State(listener_state),
            Json(CsrRequest {
                token: "bad".into(),
                node_name: "worker".into(),
                csr_pem: "ignored".into(),
                usage: "kubelet-client".into(),
            }),
        )
        .await;
        match resp {
            Err((status, _)) => assert_eq!(status, StatusCode::UNAUTHORIZED),
            Ok(_) => panic!("must reject bad token"),
        }
    }

    #[tokio::test]
    async fn submit_csr_returns_503_when_ca_missing() {
        let listener_state = test_listener_state("good-token-1234567890");
        let resp = submit_csr(
            State(listener_state),
            Json(CsrRequest {
                token: "good-token-1234567890".into(),
                node_name: "worker".into(),
                csr_pem: "ignored".into(),
                usage: "kubelet-client".into(),
            }),
        )
        .await;
        match resp {
            Err((status, _)) => assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE),
            Ok(_) => panic!("must 503 when CA is empty"),
        }
    }

    #[tokio::test]
    async fn submit_csr_rejects_unsupported_usage() {
        let (ca_pem, ca_key_pem) = mint_test_ca();
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["good-token-1234567890".into()]),
            ca_cert_pem: Arc::new(ca_pem),
            ca_key_pem: Arc::new(ca_key_pem),
            csrs: Arc::new(dashmap::DashMap::new()),
        };
        let resp = submit_csr(
            State(listener_state),
            Json(CsrRequest {
                token: "good-token-1234567890".into(),
                node_name: "worker".into(),
                csr_pem: "ignored".into(),
                usage: "server-auth".into(),
            }),
        )
        .await;
        match resp {
            Err((status, _)) => assert_eq!(status, StatusCode::BAD_REQUEST),
            Ok(_) => panic!("must reject non-kubelet-client usage"),
        }
    }

    #[tokio::test]
    async fn submit_csr_end_to_end() {
        use rcgen::{CertificateParams, DnType, KeyPair};
        let (ca_pem, ca_key_pem) = mint_test_ca();
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["good-token-1234567890".into()]),
            ca_cert_pem: Arc::new(ca_pem.clone()),
            ca_key_pem: Arc::new(ca_key_pem),
            csrs: Arc::new(dashmap::DashMap::new()),
        };
        // Worker-side CSR
        let kp = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, "ignored");
        let csr_pem = params.serialize_request(&kp).unwrap().pem().unwrap();

        let resp = submit_csr(
            State(listener_state.clone()),
            Json(CsrRequest {
                token: "good-token-1234567890".into(),
                node_name: "worker-e2e".into(),
                csr_pem,
                usage: "kubelet-client".into(),
            }),
        )
        .await
        .expect("CSR must be approved");
        assert_eq!(resp.status, "Approved");
        assert!(resp.certificate.contains("BEGIN CERTIFICATE"));
        assert_eq!(resp.ca, ca_pem);
        assert!(listener_state.csrs.contains_key(&resp.name));
    }

    #[tokio::test]
    async fn bootstrap_ca_returns_pem_or_503() {
        // Empty CA → 503
        let listener_state = test_listener_state("tok-1234567890ab");
        let resp = bootstrap_ca(State(listener_state)).await;
        assert!(resp.is_err());

        // Populated CA → 200 with the PEM
        let (ca_pem, _) = mint_test_ca();
        let listener_state = ApiserverListenerState {
            resources: Arc::new(cave_apiserver::store::ResourceStore::new()),
            bootstrap_tokens: Arc::new(vec!["tok-1234567890ab".into()]),
            ca_cert_pem: Arc::new(ca_pem.clone()),
            ca_key_pem: Arc::new(String::new()),
            csrs: Arc::new(dashmap::DashMap::new()),
        };
        let (status, _hdrs, body) = bootstrap_ca(State(listener_state))
            .await
            .expect("CA must be served");
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, ca_pem);
    }

    /// Simulate the crash-recovery path: writer task drains a few events
    /// from the broadcast, then we restore a fresh runtime from the on-disk
    /// snapshot + WAL.
    #[tokio::test]
    async fn wal_survives_simulated_crash() {
        let tmp = TempDir::new().unwrap();
        let dd = tmp.path().join("cluster");
        crate::cluster::init(&dd, "wal-test", "127.0.0.1:6443").unwrap();
        let rt = ClusterRuntime::load(Some(&dd)).await.unwrap().unwrap();

        // Spawn just the WAL writer, drive a few PUTs, drop the writer
        // task by dropping the store reference it subscribes through.
        let wal_path = dd.join("etcd/wal.log");
        let rx = rt.etcd_store.subscribe();
        let writer_path = wal_path.clone();
        let handle = tokio::spawn(async move { super::wal::run_writer(writer_path, rx).await });

        for i in 0..5u32 {
            rt.etcd_store.put(&cave_etcd::models::PutRequest {
                key: format!("key-{i}"),
                value: format!("val-{i}"),
                lease: None,
                prev_kv: false,
            });
        }
        // give the writer task a moment to drain
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        handle.abort();
        let _ = handle.await;

        assert!(wal_path.exists(), "WAL file must exist after writes");
        let wal_size = std::fs::metadata(&wal_path).unwrap().len();
        assert!(wal_size > 0, "WAL must have content");

        // Don't snapshot — simulate SIGKILL between snapshots. Reload.
        drop(rt);
        let rt2 = ClusterRuntime::load(Some(&dd)).await.unwrap().unwrap();
        let range = cave_etcd::models::RangeRequest {
            key: String::new(),
            range_end: Some("\u{ffff}".into()),
            limit: None,
            revision: None,
            keys_only: false,
            count_only: false,
        };
        let resp = rt2.etcd_store.range(&range).expect("range");
        assert_eq!(resp.count, 5, "all 5 keys must replay from WAL");
    }

    fn mint_test_ca() -> (String, String) {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
        };
        let kp = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, "test CA");
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        let cert = params.self_signed(&kp).unwrap();
        (cert.pem(), kp.serialize_pem())
    }
}
