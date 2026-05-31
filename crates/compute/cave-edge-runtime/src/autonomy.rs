// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Local-autonomy connection state machine — KubeEdge edge autonomy.
//!
//! KubeEdge's headline property is that an edge node keeps running its
//! workloads even when the cloud control plane is unreachable. This module
//! ports that as a small heartbeat-driven state machine:
//!
//!   * EdgeHub keepalives feed `heartbeat`; `tick` evaluates the connection
//!     against a timeout. No heartbeat within the window → `Disconnected`.
//!   * While `Disconnected` the edge does **not** evict pods
//!     (`should_evict_on_disconnect` is false, `keep_pods_running` is true) —
//!     the opposite of the default node-controller behavior.
//!   * A `Disconnected → Connected` transition raises a one-shot
//!     reconcile-needed flag: the edge fell behind while offline and must
//!     resync with the cloud once the link is back.
//!
//! Pure logic — the caller supplies monotonic timestamps.

/// Cloud-link connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
}

/// Edge autonomy controller.
#[derive(Debug, Clone)]
pub struct EdgeAutonomy {
    state: ConnectionState,
    last_heartbeat: u64,
    disconnected_since: Option<u64>,
    reconcile_needed: bool,
}

impl EdgeAutonomy {
    /// Construct in the `Connected` state with an initial heartbeat at `now`.
    pub fn new(now: u64) -> Self {
        Self {
            state: ConnectionState::Connected,
            last_heartbeat: now,
            disconnected_since: None,
            reconcile_needed: false,
        }
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Record a keepalive from the cloud. If we were `Disconnected` this is a
    /// reconnect: transition to `Connected` and raise the reconcile flag.
    pub fn heartbeat(&mut self, now: u64) {
        self.last_heartbeat = now;
        if self.state == ConnectionState::Disconnected {
            self.state = ConnectionState::Connected;
            self.disconnected_since = None;
            self.reconcile_needed = true;
        }
    }

    /// Evaluate the connection at `now` against `timeout`. If currently
    /// connected and no heartbeat has arrived within the window, transition to
    /// `Disconnected`. Returns the resulting state.
    pub fn tick(&mut self, now: u64, timeout: u64) -> ConnectionState {
        if self.state == ConnectionState::Connected
            && now.saturating_sub(self.last_heartbeat) > timeout
        {
            self.state = ConnectionState::Disconnected;
            self.disconnected_since = Some(now);
        }
        self.state
    }

    /// Edge autonomy: pods keep running regardless of cloud connectivity.
    pub fn keep_pods_running(&self) -> bool {
        true
    }

    /// The edge never evicts pods merely because the cloud link dropped.
    pub fn should_evict_on_disconnect(&self) -> bool {
        false
    }

    /// One-shot read of the reconcile-on-reconnect flag (consumes it).
    pub fn take_reconcile_needed(&mut self) -> bool {
        std::mem::take(&mut self.reconcile_needed)
    }

    /// How long the node has been offline at `now`, or `None` if connected.
    pub fn offline_duration(&self, now: u64) -> Option<u64> {
        self.disconnected_since.map(|t| now.saturating_sub(t))
    }
}
