// SPDX-License-Identifier: AGPL-3.0-or-later
//! TCP transport for production inter-node communication.
//!
//! Wire format (per frame):
//!   [u32 length (big-endian)][u32 checksum][JSON payload]
//!
//! Each established connection is maintained as a persistent session with
//! automatic reconnection on failure.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

use crate::error::{HaError, HaResult};
use crate::raft::messages::RaftMessage;
use crate::raft::types::NodeId;
use crate::transport::Transport;

/// Maps NodeId → advertised TCP address.
pub type PeerAddrs = HashMap<NodeId, String>;

/// Production TCP transport.
pub struct TcpTransport {
    from: NodeId,
    peers: Arc<PeerAddrs>,
    /// Cache of open outbound connections.
    conns: Arc<Mutex<HashMap<NodeId, mpsc::UnboundedSender<Vec<u8>>>>>,
}

impl TcpTransport {
    /// Create a transport and spawn a listener for inbound connections.
    ///
    /// `msg_tx` receives decoded messages from remote peers.
    pub async fn new(
        id: NodeId,
        listen_addr: &str,
        peers: PeerAddrs,
        msg_tx: mpsc::UnboundedSender<(NodeId, RaftMessage)>,
    ) -> HaResult<Self> {
        let listener = TcpListener::bind(listen_addr).await?;
        info!(id, addr = listen_addr, "TCP transport listening");
        tokio::spawn(accept_loop(listener, msg_tx));
        Ok(Self {
            from: id,
            peers: Arc::new(peers),
            conns: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn get_or_connect(&self, to: NodeId) -> HaResult<mpsc::UnboundedSender<Vec<u8>>> {
        let mut conns = self.conns.lock().await;
        if let Some(tx) = conns.get(&to) {
            if !tx.is_closed() {
                return Ok(tx.clone());
            }
        }
        let addr = self.peers.get(&to).ok_or_else(|| {
            HaError::Transport(format!("no address for node {to}"))
        })?;
        let stream = TcpStream::connect(addr).await.map_err(|e| {
            HaError::Transport(format!("connect to {addr}: {e}"))
        })?;
        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(write_loop(stream, rx));
        conns.insert(to, tx.clone());
        Ok(tx)
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn send(&self, to: NodeId, msg: RaftMessage) -> HaResult<()> {
        let payload = serde_json::to_vec(&msg)?;
        let frame = encode_frame(&payload);
        match self.get_or_connect(to).await {
            Ok(tx) => {
                tx.send(frame).map_err(|_| HaError::Transport("connection closed".into()))?;
            }
            Err(e) => {
                debug!(from = self.from, to, "send failed: {e}");
                // Evict stale connection.
                self.conns.lock().await.remove(&to);
            }
        }
        Ok(())
    }
}

// ── Frame encoding ────────────────────────────────────────────────────────

fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let csum = simple_checksum(payload);
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&csum.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn simple_checksum(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |acc, &b| acc.wrapping_mul(31).wrapping_add(b as u32))
}

// ── Async IO loops ────────────────────────────────────────────────────────

async fn accept_loop(
    listener: TcpListener,
    msg_tx: mpsc::UnboundedSender<(NodeId, RaftMessage)>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                debug!(addr = %addr, "new inbound connection");
                let tx = msg_tx.clone();
                tokio::spawn(read_loop(stream, tx));
            }
            Err(e) => warn!("accept error: {e}"),
        }
    }
}

async fn read_loop(
    mut stream: TcpStream,
    msg_tx: mpsc::UnboundedSender<(NodeId, RaftMessage)>,
) {
    loop {
        // Read frame header: [u32 len][u32 csum].
        let mut header = [0u8; 8];
        if stream.read_exact(&mut header).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let expected_csum = u32::from_be_bytes(header[4..8].try_into().unwrap());

        if len > 64 * 1024 * 1024 {
            warn!("oversized frame {len}");
            break;
        }
        let mut buf = vec![0u8; len];
        if stream.read_exact(&mut buf).await.is_err() {
            break;
        }
        if simple_checksum(&buf) != expected_csum {
            warn!("checksum mismatch, dropping frame");
            continue;
        }
        // Decode: envelope format is [from: u64 (8 bytes)][json msg].
        if buf.len() < 8 {
            continue;
        }
        let from = u64::from_be_bytes(buf[..8].try_into().unwrap());
        match serde_json::from_slice::<RaftMessage>(&buf[8..]) {
            Ok(msg) => {
                if msg_tx.send((from, msg)).is_err() {
                    break;
                }
            }
            Err(e) => warn!("decode error: {e}"),
        }
    }
}

async fn write_loop(
    mut stream: TcpStream,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(frame) = rx.recv().await {
        if stream.write_all(&frame).await.is_err() {
            break;
        }
    }
}
