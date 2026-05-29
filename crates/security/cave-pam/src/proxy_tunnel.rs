// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Access proxy tunnel state tracking.
//!
//! The Teleport proxy mediates every connection via a reverse tunnel: the
//! Teleport agent on each node establishes a persistent reverse tunnel to the
//! proxy, and the proxy multiplexes user sessions over it. This module tracks
//! the lifecycle state of those tunnels.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Domain types ──────────────────────────────────────────────────────────────

/// Protocol type of the tunnelled connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelKind {
    /// OpenSSH session (port 22 or custom).
    Ssh,
    /// Database proxy (Postgres, MySQL, MongoDB, etc.).
    Database,
    /// Kubernetes API proxy (kubectl).
    Kubernetes,
    /// HTTP application proxy.
    Application,
    /// Windows RDP proxy.
    Rdp,
}

/// Lifecycle state of a tunnel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelState {
    /// Tunnel is established and data can flow.
    Active,
    /// Tunnel has been cleanly closed.
    Closed,
    /// Tunnel was lost due to a network error.
    Error,
}

/// Parameters for opening a new tunnel entry.
#[derive(Debug, Clone)]
pub struct OpenTunnel {
    /// The PAM session this tunnel belongs to.
    pub session_id: Uuid,
    /// User on whose behalf the tunnel is opened.
    pub user_id: Uuid,
    /// Node being proxied to.
    pub node_id: Uuid,
    /// Network address of the target (host:port).
    pub target_addr: String,
    /// Protocol being tunnelled.
    pub kind: TunnelKind,
}

/// A tracked tunnel record.
#[derive(Debug, Clone)]
pub struct TunnelRecord {
    /// Unique tunnel ID.
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub node_id: Uuid,
    pub target_addr: String,
    pub kind: TunnelKind,
    pub state: TunnelState,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    /// Bytes transferred through this tunnel (updated by the proxy).
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the tunnel registry.
#[derive(Debug, PartialEq, Clone)]
pub enum TunnelError {
    NotFound,
    AlreadyClosed,
}

impl std::fmt::Display for TunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "tunnel not found"),
            Self::AlreadyClosed => write!(f, "tunnel is already closed"),
        }
    }
}

impl std::error::Error for TunnelError {}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Thread-safe registry of proxy tunnels.
#[derive(Debug, Default)]
pub struct TunnelRegistry {
    tunnels: Arc<RwLock<HashMap<Uuid, TunnelRecord>>>,
}

impl TunnelRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new active tunnel. Returns its ID.
    pub fn open(&self, req: OpenTunnel) -> Uuid {
        let id = Uuid::new_v4();
        let record = TunnelRecord {
            id,
            session_id: req.session_id,
            user_id: req.user_id,
            node_id: req.node_id,
            target_addr: req.target_addr,
            kind: req.kind,
            state: TunnelState::Active,
            opened_at: Utc::now(),
            closed_at: None,
            bytes_sent: 0,
            bytes_recv: 0,
        };
        self.tunnels.write().unwrap().insert(id, record);
        id
    }

    /// Mark a tunnel as cleanly closed.
    pub fn close(&self, id: &Uuid) -> Result<(), TunnelError> {
        let mut map = self.tunnels.write().unwrap();
        let t = map.get_mut(id).ok_or(TunnelError::NotFound)?;
        if t.state == TunnelState::Closed {
            return Err(TunnelError::AlreadyClosed);
        }
        t.state = TunnelState::Closed;
        t.closed_at = Some(Utc::now());
        Ok(())
    }

    /// Mark a tunnel as errored (connection lost).
    pub fn error(&self, id: &Uuid) -> Result<(), TunnelError> {
        let mut map = self.tunnels.write().unwrap();
        let t = map.get_mut(id).ok_or(TunnelError::NotFound)?;
        t.state = TunnelState::Error;
        t.closed_at = Some(Utc::now());
        Ok(())
    }

    /// Update byte counters for a tunnel.
    pub fn update_bytes(&self, id: &Uuid, sent: u64, recv: u64) -> Result<(), TunnelError> {
        let mut map = self.tunnels.write().unwrap();
        let t = map.get_mut(id).ok_or(TunnelError::NotFound)?;
        t.bytes_sent += sent;
        t.bytes_recv += recv;
        Ok(())
    }

    /// Retrieve a tunnel by ID.
    pub fn get(&self, id: &Uuid) -> Option<TunnelRecord> {
        self.tunnels.read().unwrap().get(id).cloned()
    }

    /// Return all currently active tunnels.
    pub fn list_active(&self) -> Vec<TunnelRecord> {
        self.tunnels
            .read()
            .unwrap()
            .values()
            .filter(|t| t.state == TunnelState::Active)
            .cloned()
            .collect()
    }

    /// Return count of active tunnels.
    pub fn active_count(&self) -> usize {
        self.tunnels
            .read()
            .unwrap()
            .values()
            .filter(|t| t.state == TunnelState::Active)
            .count()
    }

    /// Return all active tunnels for a given user.
    pub fn tunnels_for_user(&self, user_id: &Uuid) -> Vec<TunnelRecord> {
        self.tunnels
            .read()
            .unwrap()
            .values()
            .filter(|t| &t.user_id == user_id && t.state == TunnelState::Active)
            .cloned()
            .collect()
    }

    /// Return all active tunnels pointing to a given node.
    pub fn tunnels_for_node(&self, node_id: &Uuid) -> Vec<TunnelRecord> {
        self.tunnels
            .read()
            .unwrap()
            .values()
            .filter(|t| &t.node_id == node_id && t.state == TunnelState::Active)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_twice_errors() {
        let reg = TunnelRegistry::new();
        let id = reg.open(OpenTunnel {
            session_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            node_id: Uuid::new_v4(),
            target_addr: "srv:22".to_string(),
            kind: TunnelKind::Ssh,
        });
        reg.close(&id).unwrap();
        assert_eq!(reg.close(&id).unwrap_err(), TunnelError::AlreadyClosed);
    }

    #[test]
    fn byte_counter_accumulates() {
        let reg = TunnelRegistry::new();
        let id = reg.open(OpenTunnel {
            session_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            node_id: Uuid::new_v4(),
            target_addr: "db:5432".to_string(),
            kind: TunnelKind::Database,
        });
        reg.update_bytes(&id, 100, 200).unwrap();
        reg.update_bytes(&id, 50, 75).unwrap();
        let t = reg.get(&id).unwrap();
        assert_eq!(t.bytes_sent, 150);
        assert_eq!(t.bytes_recv, 275);
    }

    #[test]
    fn errored_tunnel_excluded_from_active() {
        let reg = TunnelRegistry::new();
        let id = reg.open(OpenTunnel {
            session_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            node_id: Uuid::new_v4(),
            target_addr: "app:443".to_string(),
            kind: TunnelKind::Application,
        });
        reg.error(&id).unwrap();
        assert_eq!(reg.active_count(), 0);
    }
}
