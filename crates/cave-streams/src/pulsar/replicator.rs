// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
// Source: apache/pulsar@1940aebc6ade10050399cd65f870353eedf80008
//   pulsar-broker/src/main/java/org/apache/pulsar/broker/service/persistent/PersistentReplicator.java
//   pulsar-broker/src/main/java/org/apache/pulsar/broker/service/AbstractReplicator.java
//   pulsar-broker/src/main/java/org/apache/pulsar/broker/service/BrokerService.java#getReplicationClient

//! Pulsar geo-replication.
//!
//! For every `(source_topic, remote_cluster)` Pulsar runs a
//! `PersistentReplicator` — a managed-cursor-driven consumer that
//! tails the topic and re-publishes to the same topic on the remote
//! cluster.  A producer-side flag (`replicate_to`) can restrict
//! replication to a subset of clusters.
//!
//! cave-streams implements the same semantics in-process:
//! - [`ReplicationClusterSet`] — the set of clusters the topic is
//!   eligible to replicate to (from `replication_clusters` topic
//!   policy).
//! - [`Replicator`] per (topic, remote) — owns the cursor that points
//!   into the source topic backlog + a "remote producer" buffer the
//!   tests can drain to simulate the wire side.
//! - Producer-side `replicate_to` filter ([`OutboundMessage`]):
//!   replicators only consume the entries flagged for their remote.
//!
//! Plumbing into [`crate::pulsar_dispatch`]: the dispatcher already
//! routes messages to consumers; to support replication a `Topic`
//! grows a `replicators: Vec<Replicator>` and feeds every committed
//! entry into each replicator's `enqueue()`.  That wiring is a
//! follow-up; this module ships the state machine.

use crate::error::{StreamsError, StreamsResult};
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// Cluster name (Pulsar admin: `pulsar-admin clusters create`).
pub type ClusterName = String;

/// Topic-policy field `replication_clusters` — the set of clusters
/// that a topic replicates to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplicationClusterSet {
    clusters: BTreeSet<ClusterName>,
}

impl ReplicationClusterSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_iter<I: IntoIterator<Item = ClusterName>>(iter: I) -> Self {
        let clusters = iter.into_iter().collect();
        Self { clusters }
    }

    pub fn add(&mut self, c: impl Into<ClusterName>) {
        self.clusters.insert(c.into());
    }

    pub fn remove(&mut self, c: &str) {
        self.clusters.remove(c);
    }

    pub fn contains(&self, c: &str) -> bool {
        self.clusters.contains(c)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ClusterName> {
        self.clusters.iter()
    }

    pub fn len(&self) -> usize {
        self.clusters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clusters.is_empty()
    }
}

/// A published message annotated with its producer-side
/// `replicate_to` filter.  Pulsar `MessageMetadata.replicate_to` lists
/// clusters the producer wants the message to go to; empty list →
/// inherit topic policy.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub local_id: u64,
    pub payload: Vec<u8>,
    pub replicate_to: Option<Vec<ClusterName>>,
}

impl OutboundMessage {
    pub fn new(local_id: u64, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            local_id,
            payload: payload.into(),
            replicate_to: None,
        }
    }

    pub fn restrict(mut self, clusters: Vec<ClusterName>) -> Self {
        self.replicate_to = Some(clusters);
        self
    }

    /// Decide if this message should be replicated to `target`.  When
    /// `replicate_to` is absent, the topic policy applies (caller
    /// passes `topic_policy.contains(target)`).
    pub fn replicates_to(&self, target: &str, topic_eligible: bool) -> bool {
        match &self.replicate_to {
            Some(list) => list.iter().any(|c| c == target),
            None => topic_eligible,
        }
    }
}

/// A single (source_topic, remote_cluster) replicator.
pub struct Replicator {
    pub source_topic: String,
    pub remote_cluster: ClusterName,
    inner: Mutex<ReplicatorInner>,
}

#[derive(Debug, Default)]
struct ReplicatorInner {
    /// `__replication.<cluster>` cursor (next local_id to replicate).
    cursor: u64,
    /// Messages handed off to the remote — drained by tests / wire.
    remote_inbox: Vec<OutboundMessage>,
    paused: bool,
}

impl Replicator {
    pub fn new(source_topic: impl Into<String>, remote: impl Into<ClusterName>) -> Self {
        Self {
            source_topic: source_topic.into(),
            remote_cluster: remote.into(),
            inner: Mutex::new(ReplicatorInner::default()),
        }
    }

    pub fn cursor(&self) -> u64 {
        self.inner.lock().unwrap().cursor
    }

    pub fn is_paused(&self) -> bool {
        self.inner.lock().unwrap().paused
    }

    pub fn pause(&self) {
        self.inner.lock().unwrap().paused = true;
    }

    pub fn resume(&self) {
        self.inner.lock().unwrap().paused = false;
    }

    /// Feed a freshly-committed local entry into the replicator.  The
    /// replicator decides whether to forward (based on the topic
    /// policy + per-message override) and bumps the `__replication`
    /// cursor either way.
    ///
    /// Returns `Ok(true)` when the message was forwarded.
    pub fn enqueue(
        &self,
        msg: &OutboundMessage,
        topic_policy: &ReplicationClusterSet,
    ) -> StreamsResult<bool> {
        let mut inner = self.inner.lock().unwrap();
        if inner.paused {
            return Err(StreamsError::Internal(format!(
                "replicator {}→{} is paused",
                self.source_topic, self.remote_cluster
            )));
        }
        if msg.local_id < inner.cursor {
            // Already replicated — idempotent.
            return Ok(false);
        }
        let eligible = topic_policy.contains(&self.remote_cluster);
        let send = msg.replicates_to(&self.remote_cluster, eligible);
        if send {
            inner.remote_inbox.push(msg.clone());
        }
        inner.cursor = msg.local_id + 1;
        Ok(send)
    }

    /// Drain the buffered remote inbox — caller is the wire-side
    /// adapter to the remote cluster.
    pub fn drain_remote_inbox(&self) -> Vec<OutboundMessage> {
        let mut inner = self.inner.lock().unwrap();
        std::mem::take(&mut inner.remote_inbox)
    }

    pub fn remote_inbox_len(&self) -> usize {
        self.inner.lock().unwrap().remote_inbox.len()
    }

    /// Acknowledge replication of a remote ack — advance the cursor
    /// if the ack is strictly higher.  No-op otherwise.
    pub fn ack_replicated(&self, up_to: u64) {
        let mut inner = self.inner.lock().unwrap();
        if up_to > inner.cursor {
            inner.cursor = up_to;
        }
    }
}

/// Bundle of replicators per source topic — one per remote cluster.
pub struct TopicReplicators {
    pub topic: String,
    pub policy: ReplicationClusterSet,
    replicators: Mutex<HashMap<ClusterName, Replicator>>,
}

impl TopicReplicators {
    pub fn new(topic: impl Into<String>, policy: ReplicationClusterSet) -> Self {
        Self {
            topic: topic.into(),
            policy,
            replicators: Mutex::new(HashMap::new()),
        }
    }

    /// Ensure a replicator exists for `remote` (idempotent).
    pub fn ensure(&self, remote: &str) {
        let mut r = self.replicators.lock().unwrap();
        if !r.contains_key(remote) {
            r.insert(
                remote.to_string(),
                Replicator::new(self.topic.clone(), remote.to_string()),
            );
        }
    }

    /// Returns the number of replicators currently attached.
    pub fn count(&self) -> usize {
        self.replicators.lock().unwrap().len()
    }

    /// Snapshot of remote cluster names served by this topic.
    pub fn remotes(&self) -> Vec<ClusterName> {
        let mut out: Vec<_> = self
            .replicators
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        out.sort();
        out
    }

    /// Broadcast `msg` to every replicator.  Returns count forwarded.
    pub fn fanout(&self, msg: &OutboundMessage) -> StreamsResult<usize> {
        let r = self.replicators.lock().unwrap();
        let mut forwarded = 0;
        for (_, rep) in r.iter() {
            if rep.enqueue(msg, &self.policy)? {
                forwarded += 1;
            }
        }
        Ok(forwarded)
    }

    pub fn replicator_cursor(&self, remote: &str) -> Option<u64> {
        self.replicators.lock().unwrap().get(remote).map(|r| r.cursor())
    }

    pub fn drain_remote_inbox(&self, remote: &str) -> Option<Vec<OutboundMessage>> {
        self.replicators
            .lock()
            .unwrap()
            .get(remote)
            .map(|r| r.drain_remote_inbox())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(clusters: &[&str]) -> ReplicationClusterSet {
        ReplicationClusterSet::from_iter(clusters.iter().map(|s| s.to_string()))
    }

    #[test]
    fn test_replicator_cursor_starts_at_zero() {
        // cite: pulsar 4.2.0 __replication cursor initial position
        // ensemble = rp-001
        let r = Replicator::new("t", "us-west");
        assert_eq!(r.cursor(), 0);
        assert_eq!(r.remote_inbox_len(), 0);
    }

    #[test]
    fn test_replicator_enqueue_forwards_when_topic_eligible() {
        // cite: pulsar 4.2.0 PersistentReplicator.startProducing
        // ensemble = rp-002
        let r = Replicator::new("t", "us-west");
        let pol = policy(&["us-west"]);
        let msg = OutboundMessage::new(0, b"hello".to_vec());
        let forwarded = r.enqueue(&msg, &pol).unwrap();
        assert!(forwarded);
        assert_eq!(r.cursor(), 1);
        assert_eq!(r.remote_inbox_len(), 1);
    }

    #[test]
    fn test_replicator_enqueue_skips_when_topic_excludes_cluster() {
        // cite: pulsar 4.2.0 replication_clusters policy filter
        // ensemble = rp-003
        let r = Replicator::new("t", "us-west");
        let pol = policy(&["eu-central"]); // us-west not in policy
        let msg = OutboundMessage::new(0, b"x".to_vec());
        let forwarded = r.enqueue(&msg, &pol).unwrap();
        assert!(!forwarded);
        // Cursor still advances (we mark the entry as processed).
        assert_eq!(r.cursor(), 1);
        assert_eq!(r.remote_inbox_len(), 0);
    }

    #[test]
    fn test_replicator_per_message_replicate_to_overrides_topic_policy() {
        // cite: pulsar 4.2.0 MessageMetadata.replicate_to filter
        // ensemble = rp-004
        let r = Replicator::new("t", "us-west");
        // Topic policy allows replication to us-west but the producer
        // restricted this message to eu-central only.
        let pol = policy(&["us-west"]);
        let msg = OutboundMessage::new(0, b"private".to_vec())
            .restrict(vec!["eu-central".into()]);
        let forwarded = r.enqueue(&msg, &pol).unwrap();
        assert!(!forwarded);
    }

    #[test]
    fn test_replicator_per_message_replicate_to_includes_cluster() {
        // cite: pulsar 4.2.0 replicate_to with target cluster wins
        // ensemble = rp-005
        let r = Replicator::new("t", "us-west");
        let pol = policy(&[]); // Topic policy empty
        let msg = OutboundMessage::new(0, b"x".to_vec())
            .restrict(vec!["us-west".into(), "ap-south".into()]);
        let forwarded = r.enqueue(&msg, &pol).unwrap();
        assert!(forwarded);
    }

    #[test]
    fn test_replicator_pause_blocks_enqueue() {
        // cite: pulsar 4.2.0 admin pause replication
        // ensemble = rp-006
        let r = Replicator::new("t", "us-west");
        r.pause();
        let pol = policy(&["us-west"]);
        let msg = OutboundMessage::new(0, b"x".to_vec());
        assert!(r.enqueue(&msg, &pol).is_err());
    }

    #[test]
    fn test_replicator_drain_remote_inbox_clears() {
        // cite: pulsar 4.2.0 once forwarded, inbox emptied on wire send
        // ensemble = rp-007
        let r = Replicator::new("t", "us-west");
        let pol = policy(&["us-west"]);
        r.enqueue(&OutboundMessage::new(0, b"a".to_vec()), &pol).unwrap();
        r.enqueue(&OutboundMessage::new(1, b"b".to_vec()), &pol).unwrap();
        let drained = r.drain_remote_inbox();
        assert_eq!(drained.len(), 2);
        assert_eq!(r.remote_inbox_len(), 0);
    }

    #[test]
    fn test_replicator_idempotent_on_replay() {
        // cite: pulsar 4.2.0 idempotent on cursor replay (broker restart)
        // ensemble = rp-008
        let r = Replicator::new("t", "us-west");
        let pol = policy(&["us-west"]);
        r.enqueue(&OutboundMessage::new(0, b"a".to_vec()), &pol).unwrap();
        // Replay the same entry — must not double-forward.
        let again = r.enqueue(&OutboundMessage::new(0, b"a".to_vec()), &pol).unwrap();
        assert!(!again);
        assert_eq!(r.cursor(), 1);
        assert_eq!(r.remote_inbox_len(), 1);
    }

    #[test]
    fn test_topic_replicators_ensure_is_idempotent() {
        // cite: pulsar 4.2.0 ensureReplicator (per cluster)
        // ensemble = rp-009
        let tr = TopicReplicators::new("t", policy(&["us-west", "eu-central"]));
        tr.ensure("us-west");
        tr.ensure("us-west");
        tr.ensure("eu-central");
        assert_eq!(tr.count(), 2);
        assert_eq!(tr.remotes(), vec!["eu-central", "us-west"]);
    }

    #[test]
    fn test_topic_replicators_fanout_counts_eligible_targets() {
        // cite: pulsar 4.2.0 broadcast committed entry to every replicator
        // ensemble = rp-010
        let tr = TopicReplicators::new("t", policy(&["us-west", "eu-central"]));
        tr.ensure("us-west");
        tr.ensure("eu-central");
        // Message restricted to us-west only.
        let msg = OutboundMessage::new(0, b"x".to_vec()).restrict(vec!["us-west".into()]);
        let forwarded = tr.fanout(&msg).unwrap();
        assert_eq!(forwarded, 1);
        let drained_uw = tr.drain_remote_inbox("us-west").unwrap();
        let drained_eu = tr.drain_remote_inbox("eu-central").unwrap();
        assert_eq!(drained_uw.len(), 1);
        assert_eq!(drained_eu.len(), 0);
    }

    #[test]
    fn test_topic_replicators_replicator_cursor_advances_per_remote() {
        // cite: pulsar 4.2.0 per-cluster __replication cursor
        // ensemble = rp-011
        let tr = TopicReplicators::new("t", policy(&["us-west", "eu-central"]));
        tr.ensure("us-west");
        tr.ensure("eu-central");
        tr.fanout(&OutboundMessage::new(0, b"a".to_vec())).unwrap();
        tr.fanout(&OutboundMessage::new(1, b"b".to_vec())).unwrap();
        assert_eq!(tr.replicator_cursor("us-west"), Some(2));
        assert_eq!(tr.replicator_cursor("eu-central"), Some(2));
    }
}
