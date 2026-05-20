// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pulsar geographic replication — cross-cluster topic replication.
//!
//! upstream: apache/pulsar — pulsar-broker/.../replication/
//! (ReplicatorSubscription, PersistentReplicator, GeoReplicationService)
//!
//! Pulsar geo-rep wires a *replicator subscription* on every replicated
//! topic; the broker pulls the durable cursor from the local managed
//! ledger and forwards each message to peer clusters until they
//! acknowledge it. Replication is per-topic, configurable at the
//! namespace level, and respects per-message routing rules
//! (`__producer_name__` filter, exclusion of locally produced messages,
//! and dedup by `__replicated_from__`).
//!
//! This port covers the broker-side state machine: cluster registry,
//! per-topic replicator state, message lifecycle (queued → in-flight →
//! acked → durable), and the dedup window keyed by source cluster. No
//! TLS/auth or network is hard-wired — the caller injects a
//! `ReplicationSender` so the runtime can route bytes through
//! cave-streams' own pulsar_wire layer.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

/// Logical identifier of a Pulsar cluster.
#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClusterId(pub String);

impl ClusterId {
    pub fn new(s: impl Into<String>) -> Self { ClusterId(s.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
}

#[derive(Debug, Clone)]
pub struct ClusterDescriptor {
    pub id: ClusterId,
    pub broker_url: String,
    pub broker_url_tls: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct ClusterRegistry {
    clusters: HashMap<ClusterId, ClusterDescriptor>,
}

impl ClusterRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, c: ClusterDescriptor) {
        self.clusters.insert(c.id.clone(), c);
    }

    pub fn get(&self, id: &ClusterId) -> Option<&ClusterDescriptor> {
        self.clusters.get(id)
    }

    pub fn count(&self) -> usize { self.clusters.len() }
}

/// One message destined to be replicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicatedMessage {
    /// Pulsar topic id (`persistent://tenant/ns/topic`).
    pub topic: String,
    /// Source cluster (the cluster the broker producing the message belongs to).
    pub source_cluster: ClusterId,
    /// Logical ledger+entry pair from the producing broker.
    pub message_id: (u64, u64),
    /// Original producer name — used to suppress replicating back to the source.
    pub producer_name: String,
    /// Optional `__replicated_from__` tag set when this message originated in
    /// a peer cluster.
    pub replicated_from: Option<ClusterId>,
    pub payload: Vec<u8>,
}

/// Outcome of a single replication attempt to one peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationOutcome {
    Acked,
    Rejected(String),
    NetworkError,
}

/// Pluggable transport — abstracts the underlying pulsar_wire send so
/// tests can deterministically verify routing.
pub trait ReplicationSender {
    fn send(&mut self, target: &ClusterId, msg: &ReplicatedMessage) -> ReplicationOutcome;
}

/// Per-topic replication state. Tracks queued messages, in-flight ack
/// windows, durable cursor (replicated message ids), and a dedup set
/// keyed by `(source_cluster, message_id)` so a message never crosses
/// the same edge twice.
#[derive(Default, Debug, Clone)]
pub struct PersistentReplicator {
    pub topic: String,
    pub local_cluster: ClusterId,
    pub peer_clusters: Vec<ClusterId>,
    pub queue: VecDeque<ReplicatedMessage>,
    pub in_flight: HashMap<(ClusterId, (u64, u64)), ReplicatedMessage>,
    pub durable_cursor: HashMap<ClusterId, (u64, u64)>,
    pub dedup: HashSet<(ClusterId, (u64, u64))>,
    /// Per-peer counters surfaced to /metrics.
    pub sent: HashMap<ClusterId, u64>,
    pub rejected: HashMap<ClusterId, u64>,
    pub network_errors: HashMap<ClusterId, u64>,
}

impl PersistentReplicator {
    pub fn new(topic: &str, local: ClusterId, peers: Vec<ClusterId>) -> Self {
        Self {
            topic: topic.to_string(),
            local_cluster: local,
            peer_clusters: peers,
            ..Default::default()
        }
    }

    /// Enqueue a message for replication. Returns true if the message
    /// was queued, false if it was filtered out (already replicated /
    /// loop-back / dedup hit).
    pub fn enqueue(&mut self, msg: ReplicatedMessage) -> bool {
        if msg.source_cluster != self.local_cluster {
            // Loop guard — never re-replicate a message we received.
            return false;
        }
        let key = (msg.source_cluster.clone(), msg.message_id);
        if !self.dedup.insert(key) {
            return false;
        }
        self.queue.push_back(msg);
        true
    }

    /// One drain pass: send every queued message to every peer that is
    /// not the source cluster, recording outcomes.
    pub fn drain<S: ReplicationSender>(&mut self, sender: &mut S) -> usize {
        let mut shipped = 0;
        while let Some(msg) = self.queue.pop_front() {
            for peer in &self.peer_clusters.clone() {
                if *peer == msg.source_cluster {
                    continue;
                }
                // Skip if already in-flight to this peer.
                let key = (peer.clone(), msg.message_id);
                if self.in_flight.contains_key(&key) {
                    continue;
                }
                self.in_flight.insert(key.clone(), msg.clone());
                match sender.send(peer, &msg) {
                    ReplicationOutcome::Acked => {
                        self.in_flight.remove(&key);
                        self.durable_cursor.insert(peer.clone(), msg.message_id);
                        *self.sent.entry(peer.clone()).or_insert(0) += 1;
                        shipped += 1;
                    }
                    ReplicationOutcome::Rejected(_) => {
                        self.in_flight.remove(&key);
                        *self.rejected.entry(peer.clone()).or_insert(0) += 1;
                    }
                    ReplicationOutcome::NetworkError => {
                        // Keep in_flight so retry path can pick it up.
                        *self.network_errors.entry(peer.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
        shipped
    }

    /// Retry every message currently in `in_flight` (used after a peer
    /// reconnects). Returns the number of retries that succeeded.
    pub fn retry_in_flight<S: ReplicationSender>(&mut self, sender: &mut S) -> usize {
        let entries: Vec<((ClusterId, (u64, u64)), ReplicatedMessage)> =
            self.in_flight.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut ok = 0;
        for (key, msg) in entries {
            match sender.send(&key.0, &msg) {
                ReplicationOutcome::Acked => {
                    self.in_flight.remove(&key);
                    self.durable_cursor.insert(key.0.clone(), msg.message_id);
                    *self.sent.entry(key.0.clone()).or_insert(0) += 1;
                    ok += 1;
                }
                ReplicationOutcome::Rejected(_) => {
                    self.in_flight.remove(&key);
                    *self.rejected.entry(key.0.clone()).or_insert(0) += 1;
                }
                ReplicationOutcome::NetworkError => {
                    *self.network_errors.entry(key.0.clone()).or_insert(0) += 1;
                }
            }
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(topic: &str, src: &str, id: (u64, u64), name: &str) -> ReplicatedMessage {
        ReplicatedMessage {
            topic: topic.into(),
            source_cluster: ClusterId::new(src),
            message_id: id,
            producer_name: name.into(),
            replicated_from: None,
            payload: vec![1, 2, 3],
        }
    }

    struct MockSender {
        ack_for: HashSet<(ClusterId, (u64, u64))>,
        reject_for: HashSet<(ClusterId, (u64, u64))>,
        sent: Vec<(ClusterId, ReplicatedMessage)>,
    }

    impl MockSender {
        fn new() -> Self {
            Self { ack_for: HashSet::new(), reject_for: HashSet::new(), sent: Vec::new() }
        }
        fn ack_all(mut self) -> Self { self.ack_for.clear(); self }
    }

    impl ReplicationSender for MockSender {
        fn send(&mut self, target: &ClusterId, msg: &ReplicatedMessage) -> ReplicationOutcome {
            self.sent.push((target.clone(), msg.clone()));
            let key = (target.clone(), msg.message_id);
            if self.reject_for.contains(&key) {
                ReplicationOutcome::Rejected("nope".into())
            } else if self.ack_for.is_empty() || self.ack_for.contains(&key) {
                ReplicationOutcome::Acked
            } else {
                ReplicationOutcome::NetworkError
            }
        }
    }

    #[test]
    fn registry_stores_and_retrieves_clusters() {
        let mut r = ClusterRegistry::new();
        r.register(ClusterDescriptor {
            id: ClusterId::new("us-east"),
            broker_url: "pulsar://us-east:6650".into(),
            broker_url_tls: Some("pulsar+ssl://us-east:6651".into()),
        });
        let c = r.get(&ClusterId::new("us-east")).unwrap();
        assert_eq!(c.broker_url, "pulsar://us-east:6650");
    }

    #[test]
    fn enqueue_rejects_message_from_foreign_cluster() {
        let mut rep = PersistentReplicator::new(
            "persistent://t/n/topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west")],
        );
        let m = msg("persistent://t/n/topic", "eu-west", (1, 1), "p1");
        assert!(!rep.enqueue(m));
    }

    #[test]
    fn enqueue_dedup_drops_duplicate_message_id() {
        let mut rep = PersistentReplicator::new(
            "persistent://t/n/topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west")],
        );
        let m = msg("persistent://t/n/topic", "us-east", (1, 1), "p1");
        assert!(rep.enqueue(m.clone()));
        assert!(!rep.enqueue(m));
        assert_eq!(rep.queue.len(), 1);
    }

    #[test]
    fn drain_ships_to_every_peer_except_source() {
        let mut rep = PersistentReplicator::new(
            "topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west"), ClusterId::new("ap-south")],
        );
        rep.enqueue(msg("topic", "us-east", (1, 1), "p1"));
        let mut sender = MockSender::new();
        let n = rep.drain(&mut sender);
        assert_eq!(n, 2);
        assert_eq!(sender.sent.len(), 2);
        let dests: HashSet<&ClusterId> = sender.sent.iter().map(|(c, _)| c).collect();
        assert!(dests.contains(&ClusterId::new("eu-west")));
        assert!(dests.contains(&ClusterId::new("ap-south")));
    }

    #[test]
    fn drain_skips_self_in_peer_list() {
        let mut rep = PersistentReplicator::new(
            "topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("us-east"), ClusterId::new("eu-west")],
        );
        rep.enqueue(msg("topic", "us-east", (1, 1), "p1"));
        let mut sender = MockSender::new();
        let n = rep.drain(&mut sender);
        assert_eq!(n, 1);
        assert_eq!(sender.sent[0].0.as_str(), "eu-west");
    }

    #[test]
    fn rejected_messages_record_per_peer_counter() {
        let mut rep = PersistentReplicator::new(
            "topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west")],
        );
        rep.enqueue(msg("topic", "us-east", (1, 1), "p1"));
        let mut sender = MockSender::new();
        sender.reject_for.insert((ClusterId::new("eu-west"), (1, 1)));
        rep.drain(&mut sender);
        assert_eq!(rep.rejected[&ClusterId::new("eu-west")], 1);
        assert_eq!(rep.sent.get(&ClusterId::new("eu-west")), None);
    }

    #[test]
    fn network_error_keeps_in_flight_for_retry() {
        let mut rep = PersistentReplicator::new(
            "topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west")],
        );
        rep.enqueue(msg("topic", "us-east", (1, 1), "p1"));
        let mut sender = MockSender::new();
        // ack_for has a different (cluster,id) so the actual send returns NetworkError
        sender.ack_for.insert((ClusterId::new("eu-west"), (99, 99)));
        rep.drain(&mut sender);
        assert_eq!(rep.in_flight.len(), 1);
        assert_eq!(rep.network_errors[&ClusterId::new("eu-west")], 1);
    }

    #[test]
    fn retry_in_flight_promotes_to_durable_cursor() {
        let mut rep = PersistentReplicator::new(
            "topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west")],
        );
        rep.in_flight.insert(
            (ClusterId::new("eu-west"), (1, 1)),
            msg("topic", "us-east", (1, 1), "p1"),
        );
        let mut sender = MockSender::new().ack_all();
        let ok = rep.retry_in_flight(&mut sender);
        assert_eq!(ok, 1);
        assert!(rep.in_flight.is_empty());
        assert_eq!(rep.durable_cursor[&ClusterId::new("eu-west")], (1, 1));
    }

    #[test]
    fn dedup_set_remembers_already_queued_id() {
        let mut rep = PersistentReplicator::new(
            "topic",
            ClusterId::new("us-east"),
            vec![ClusterId::new("eu-west")],
        );
        rep.enqueue(msg("topic", "us-east", (5, 7), "p1"));
        assert!(rep.dedup.contains(&(ClusterId::new("us-east"), (5, 7))));
    }
}
