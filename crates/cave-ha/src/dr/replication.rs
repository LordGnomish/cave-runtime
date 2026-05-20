// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cross-region DR replication.
//!
//! The DR replicator runs on the primary leader and streams committed log
//! entries to a remote DR site over a persistent TCP connection.
//! In async mode (default for DR), replication does not block primary commits.
//! In sync mode, commits wait for DR ack (RPO = 0, RTO cost higher).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock, mpsc};
use tracing::{debug, info, warn};

use crate::config::DrConfig;
use crate::error::{HaError, HaResult};
use crate::metrics::Metrics;
use crate::raft::log::LogEntry;
use crate::raft::types::LogIndex;

/// A batch of log entries sent to the DR site.
#[derive(Debug, Serialize, Deserialize)]
pub struct DrBatch {
    pub from_index: LogIndex,
    pub entries: Vec<LogEntry>,
    pub commit_index: LogIndex,
}

/// Acknowledgement from DR site.
#[derive(Debug, Serialize, Deserialize)]
pub struct DrAck {
    pub last_applied: LogIndex,
    pub ok: bool,
    pub error: Option<String>,
}

/// Status of the DR replication channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrStatus {
    pub remote_addr: String,
    pub connected: bool,
    pub async_mode: bool,
    /// Last index confirmed by DR site.
    pub dr_last_applied: LogIndex,
    /// Primary's current commit index.
    pub primary_commit: LogIndex,
    /// Lag in entries.
    pub lag_entries: u64,
    /// Estimated RPO (seconds of data at risk).
    pub estimated_rpo_seconds: f64,
    /// Last successful replication time.
    pub last_sync: Option<chrono::DateTime<chrono::Utc>>,
}

/// Streams committed log entries to the DR site.
pub struct DrReplicator {
    config: DrConfig,
    /// Entries pending replication.
    pending: Arc<Mutex<VecDeque<LogEntry>>>,
    dr_applied: Arc<RwLock<LogIndex>>,
    primary_commit: Arc<RwLock<LogIndex>>,
    connected: Arc<RwLock<bool>>,
    metrics: Arc<Metrics>,
}

impl DrReplicator {
    pub fn new(config: DrConfig, metrics: Arc<Metrics>) -> Self {
        Self {
            config,
            pending: Arc::new(Mutex::new(VecDeque::new())),
            dr_applied: Arc::new(RwLock::new(0)),
            primary_commit: Arc::new(RwLock::new(0)),
            connected: Arc::new(RwLock::new(false)),
            metrics,
        }
    }

    /// Enqueue a newly committed entry for DR replication.
    pub async fn enqueue(&self, entry: LogEntry, commit_index: LogIndex) {
        *self.primary_commit.write().await = commit_index;
        self.pending.lock().await.push_back(entry);
    }

    /// Get current DR status.
    pub async fn status(&self) -> DrStatus {
        let dr_applied = *self.dr_applied.read().await;
        let primary_commit = *self.primary_commit.read().await;
        let lag = primary_commit.saturating_sub(dr_applied);
        let estimated_rpo = lag as f64 * 0.1; // ~100ms per entry estimate.
        DrStatus {
            remote_addr: self.config.remote_addr.clone(),
            connected: *self.connected.read().await,
            async_mode: self.config.async_mode,
            dr_last_applied: dr_applied,
            primary_commit,
            lag_entries: lag,
            estimated_rpo_seconds: estimated_rpo,
            last_sync: None, // TODO: track last sync time.
        }
    }

    /// Run the replication loop — connect and stream entries to DR site.
    pub async fn run(self: Arc<Self>) {
        let mut backoff = Duration::from_millis(100);
        loop {
            match self.connect_and_stream().await {
                Ok(()) => {
                    info!("DR replication stream ended, reconnecting");
                    backoff = Duration::from_millis(100);
                }
                Err(e) => {
                    warn!("DR replication error: {e}, retry in {backoff:?}");
                    *self.connected.write().await = false;
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        }
    }

    async fn connect_and_stream(&self) -> HaResult<()> {
        if self.config.remote_addr.is_empty() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            return Ok(());
        }
        let mut stream = TcpStream::connect(&self.config.remote_addr)
            .await
            .map_err(|e| HaError::Dr(format!("connect DR: {e}")))?;
        info!(remote = %self.config.remote_addr, "DR connection established");
        *self.connected.write().await = true;

        let mut batch_buf = Vec::new();
        loop {
            // Collect pending entries.
            let entries: Vec<LogEntry> = {
                let mut q = self.pending.lock().await;
                q.drain(..).collect()
            };

            if !entries.is_empty() {
                let commit_index = *self.primary_commit.read().await;
                let from_index = entries.first().map(|e| e.index).unwrap_or(0);
                let batch = DrBatch {
                    from_index,
                    entries,
                    commit_index,
                };
                batch_buf.clear();
                serde_json::to_writer(&mut batch_buf, &batch)
                    .map_err(|e| HaError::Dr(format!("serialize: {e}")))?;

                let len = (batch_buf.len() as u32).to_be_bytes();
                stream
                    .write_all(&len)
                    .await
                    .map_err(|e| HaError::Dr(e.to_string()))?;
                stream
                    .write_all(&batch_buf)
                    .await
                    .map_err(|e| HaError::Dr(e.to_string()))?;

                if !self.config.async_mode {
                    // Sync mode: wait for ack.
                    let mut len_buf = [0u8; 4];
                    stream
                        .read_exact(&mut len_buf)
                        .await
                        .map_err(|e| HaError::Dr(format!("ack recv: {e}")))?;
                    let ack_len = u32::from_be_bytes(len_buf) as usize;
                    let mut ack_buf = vec![0u8; ack_len];
                    stream
                        .read_exact(&mut ack_buf)
                        .await
                        .map_err(|e| HaError::Dr(format!("ack recv: {e}")))?;
                    let ack: DrAck = serde_json::from_slice(&ack_buf)
                        .map_err(|e| HaError::Dr(format!("ack decode: {e}")))?;
                    if ack.ok {
                        *self.dr_applied.write().await = ack.last_applied;
                    }
                }

                // Update lag metric.
                let lag = self
                    .primary_commit
                    .read()
                    .await
                    .saturating_sub(*self.dr_applied.read().await);
                self.metrics.dr_lag_entries.set(lag as i64);
            } else {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

/// DR site receiver — accepts replication stream and applies to local state machine.
pub struct DrReceiver {
    listen_addr: String,
}

impl DrReceiver {
    pub fn new(listen_addr: String) -> Self {
        Self { listen_addr }
    }

    pub async fn run(self, apply_tx: mpsc::UnboundedSender<Vec<LogEntry>>) -> HaResult<()> {
        let listener = tokio::net::TcpListener::bind(&self.listen_addr).await?;
        info!(addr = %self.listen_addr, "DR receiver listening");
        loop {
            let (stream, addr) = listener.accept().await?;
            debug!(%addr, "DR primary connected");
            let tx = apply_tx.clone();
            tokio::spawn(handle_dr_connection(stream, tx));
        }
    }
}

async fn handle_dr_connection(
    mut stream: TcpStream,
    apply_tx: mpsc::UnboundedSender<Vec<LogEntry>>,
) {
    #[allow(unused_assignments)]
    let mut last_applied: LogIndex = 0;
    loop {
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 64 * 1024 * 1024 {
            break;
        }
        let mut buf = vec![0u8; len];
        if stream.read_exact(&mut buf).await.is_err() {
            break;
        }
        let batch: DrBatch = match serde_json::from_slice(&buf) {
            Ok(b) => b,
            Err(e) => {
                warn!("DR decode: {e}");
                break;
            }
        };
        last_applied = batch.commit_index;
        if apply_tx.send(batch.entries).is_err() {
            break;
        }
        // Send ack.
        let ack = DrAck {
            last_applied,
            ok: true,
            error: None,
        };
        if let Ok(ack_bytes) = serde_json::to_vec(&ack) {
            let len = (ack_bytes.len() as u32).to_be_bytes();
            let _ = stream.write_all(&len).await;
            let _ = stream.write_all(&ack_bytes).await;
        }
    }
}
