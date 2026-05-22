// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cluster bus message envelope + in-process gossip log.
//!
//! Mirrors the state-machine half of `src/cluster.c`'s
//! `clusterProcessPacket`. Each message advances the cluster view:
//! PING/PONG keep liveness, MEET introduces a new node, FAIL marks a
//! peer down. The on-wire serializer lives on the upstream's
//! port-16380 bus listener; cave-cache delivers these messages
//! through the same control plane as the rest of the runtime, so the
//! gossip layer here is the pure state machine — what each message
//! does to the cluster view — plus an in-memory log for observability.

use super::state::ClusterState;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GossipMessageKind {
    /// Routine liveness check.
    Ping,
    /// Reply to a Ping.
    Pong,
    /// Add a new node to the gossip set.
    Meet,
    /// Mark a peer node as FAIL.
    Fail,
    /// Pub/sub message for the cluster bus (used by Sentinel-style
    /// failure detection).
    Publish,
    /// Failover-vote request (cave-cache delegates the actual vote to
    /// cave-cluster's Raft layer; we just record the message).
    FailoverAuthRequest,
    /// Failover-vote grant.
    FailoverAuthAck,
    /// Manual slot ownership update (`UPDATE` in upstream wire form).
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GossipMessage {
    pub from_node_id: String,
    pub to_node_id: Option<String>, // None == broadcast
    pub kind: GossipMessageKind,
    pub sender_addr: String,
    pub sender_epoch: u64,
    pub timestamp_unix: i64,
    /// Optional payload — for UPDATE this is the slot range, for
    /// PUBLISH it's the user payload.
    pub payload: Vec<u8>,
}

/// In-process gossip bus. Real Redis runs a TCP listener on the
/// cluster bus port; cave-cache uses the same control plane as the
/// rest of the runtime and serializes message effects into this
/// structure.
#[derive(Debug)]
pub struct GossipBus {
    log: VecDeque<GossipMessage>,
    log_capacity: usize,
    /// Sent / received counters (the cluster_stats_messages_* fields
    /// from CLUSTER INFO).
    sent: u64,
    received: u64,
}

impl GossipBus {
    pub fn new() -> Self {
        Self {
            log: VecDeque::new(),
            log_capacity: 512,
            sent: 0,
            received: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            log: VecDeque::new(),
            log_capacity: capacity,
            sent: 0,
            received: 0,
        }
    }

    /// Append a message we are about to send.
    pub fn record_send(&mut self, msg: GossipMessage) {
        self.sent += 1;
        self.push(msg);
    }

    /// Append a message we just received. Returns the effect a caller
    /// should apply to the cluster view.
    pub fn record_receive(&mut self, msg: GossipMessage, state: &mut ClusterState) -> GossipEffect {
        self.received += 1;
        let effect = compute_effect(&msg, state);
        match &effect {
            GossipEffect::AddNode(node) => {
                state.nodes.insert(node.id.clone(), node.clone());
            }
            GossipEffect::MarkFail(node_id) => {
                if let Some(n) = state.nodes.get_mut(node_id) {
                    // We do not delete the node — it stays in the
                    // table marked failed. Upstream uses bitflags;
                    // we keep the addr but flip is_master to false
                    // as a small visible marker.
                    n.is_master = false;
                }
            }
            GossipEffect::BumpEpoch(new_epoch) => {
                if *new_epoch > state.epoch {
                    state.epoch = *new_epoch;
                }
            }
            GossipEffect::None => {}
        }
        self.push(msg);
        effect
    }

    fn push(&mut self, msg: GossipMessage) {
        if self.log.len() == self.log_capacity {
            self.log.pop_front();
        }
        self.log.push_back(msg);
    }

    pub fn log(&self) -> &VecDeque<GossipMessage> {
        &self.log
    }

    pub fn sent_count(&self) -> u64 {
        self.sent
    }

    pub fn received_count(&self) -> u64 {
        self.received
    }
}

impl Default for GossipBus {
    fn default() -> Self {
        Self::new()
    }
}

/// What a received message tells us to do to the cluster view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GossipEffect {
    None,
    /// New node introduced via MEET or PONG-from-stranger.
    AddNode(super::state::ClusterNode),
    /// Peer marked dead via FAIL.
    MarkFail(String),
    /// Sender's epoch is newer than ours — adopt.
    BumpEpoch(u64),
}

fn compute_effect(msg: &GossipMessage, state: &ClusterState) -> GossipEffect {
    if msg.sender_epoch > state.epoch {
        return GossipEffect::BumpEpoch(msg.sender_epoch);
    }
    match msg.kind {
        GossipMessageKind::Meet => {
            if !state.nodes.contains_key(&msg.from_node_id) && msg.from_node_id != state.myself_id {
                let node = super::state::ClusterNode {
                    id: msg.from_node_id.clone(),
                    addr: msg.sender_addr.clone(),
                    is_master: true,
                    master_id: None,
                    config_epoch: msg.sender_epoch,
                    slots: Vec::new(),
                };
                return GossipEffect::AddNode(node);
            }
        }
        GossipMessageKind::Fail => {
            return GossipEffect::MarkFail(String::from_utf8_lossy(&msg.payload).into_owned());
        }
        _ => {}
    }
    GossipEffect::None
}

/// Build a "now" timestamp as unix seconds. Exposed so callers can
/// stamp messages consistently.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::state::ClusterState;

    fn msg(from: &str, kind: GossipMessageKind, payload: &[u8]) -> GossipMessage {
        GossipMessage {
            from_node_id: from.into(),
            to_node_id: None,
            kind,
            sender_addr: format!("10.0.0.{from}:6379"),
            sender_epoch: 0,
            timestamp_unix: 0,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn record_send_increments_counter() {
        let mut bus = GossipBus::new();
        bus.record_send(msg("1", GossipMessageKind::Ping, b""));
        bus.record_send(msg("1", GossipMessageKind::Pong, b""));
        assert_eq!(bus.sent_count(), 2);
        assert_eq!(bus.log().len(), 2);
    }

    #[test]
    fn meet_message_adds_node() {
        let mut bus = GossipBus::new();
        let mut state = ClusterState::new();
        let m = msg("9", GossipMessageKind::Meet, b"");
        let eff = bus.record_receive(m, &mut state);
        match eff {
            GossipEffect::AddNode(_) => {}
            other => panic!("expected AddNode got {other:?}"),
        }
        assert!(state.nodes.contains_key("9"));
        assert_eq!(state.nodes["9"].addr, "10.0.0.9:6379");
    }

    #[test]
    fn meet_for_self_is_ignored() {
        let mut bus = GossipBus::new();
        let mut state = ClusterState::new();
        let mut m = msg("any", GossipMessageKind::Meet, b"");
        m.from_node_id = state.myself_id.clone();
        let eff = bus.record_receive(m, &mut state);
        assert_eq!(eff, GossipEffect::None);
        assert!(state.nodes.is_empty());
    }

    #[test]
    fn fail_message_marks_peer() {
        let mut bus = GossipBus::new();
        let mut state = ClusterState::new();
        // Pre-seed a peer.
        state.nodes.insert(
            "peer".into(),
            super::super::state::ClusterNode {
                id: "peer".into(),
                addr: "10.0.0.7:6379".into(),
                is_master: true,
                master_id: None,
                config_epoch: 1,
                slots: Vec::new(),
            },
        );
        let m = msg("3", GossipMessageKind::Fail, b"peer");
        let eff = bus.record_receive(m, &mut state);
        assert!(matches!(eff, GossipEffect::MarkFail(id) if id == "peer"));
        assert!(!state.nodes["peer"].is_master);
    }

    #[test]
    fn higher_epoch_bumps_state_epoch() {
        let mut bus = GossipBus::new();
        let mut state = ClusterState::new();
        state.epoch = 1;
        let mut m = msg("3", GossipMessageKind::Ping, b"");
        m.sender_epoch = 9;
        let eff = bus.record_receive(m, &mut state);
        assert_eq!(eff, GossipEffect::BumpEpoch(9));
        assert_eq!(state.epoch, 9);
    }

    #[test]
    fn equal_or_lower_epoch_does_not_bump() {
        let mut bus = GossipBus::new();
        let mut state = ClusterState::new();
        state.epoch = 5;
        let mut m = msg("3", GossipMessageKind::Ping, b"");
        m.sender_epoch = 5;
        let eff = bus.record_receive(m, &mut state);
        assert_eq!(eff, GossipEffect::None);
        assert_eq!(state.epoch, 5);
    }

    #[test]
    fn bus_log_respects_capacity() {
        let mut bus = GossipBus::with_capacity(3);
        for i in 0..5 {
            bus.record_send(msg(&i.to_string(), GossipMessageKind::Ping, b""));
        }
        assert_eq!(bus.log().len(), 3);
        // Oldest two evicted; remaining are 2, 3, 4.
        let ids: Vec<_> = bus.log().iter().map(|m| m.from_node_id.clone()).collect();
        assert_eq!(ids, vec!["2", "3", "4"]);
    }

    #[test]
    fn received_counter_distinct_from_sent() {
        let mut bus = GossipBus::new();
        let mut state = ClusterState::new();
        bus.record_send(msg("1", GossipMessageKind::Ping, b""));
        bus.record_receive(msg("2", GossipMessageKind::Pong, b""), &mut state);
        assert_eq!(bus.sent_count(), 1);
        assert_eq!(bus.received_count(), 1);
    }
}
