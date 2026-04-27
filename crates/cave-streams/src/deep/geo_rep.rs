//! Pulsar-style geo-replication state machine — PIP-13 / PIP-321.
//!
//! Tracks per-cluster cursors and replication state. Each cluster has a
//! local cursor (last replicated message id) per source topic; when the
//! source cursor advances, the replicator queues the new messages for
//! delivery to each peer cluster, retrying on transient failures.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GeoRepError {
    #[error("cluster not registered: {0}")]
    UnknownCluster(String),
    #[error("topic not configured for replication: {0}")]
    UnknownTopic(String),
    #[error("source cursor regression: {topic} {from} → {to}")]
    CursorRegression { topic: String, from: u64, to: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationDirection {
    Outbound,
    Inbound,
    Bidirectional,
}

#[derive(Debug, Clone)]
pub struct ClusterPeer {
    pub name: String,
    pub url: String,
    pub direction: ReplicationDirection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationCursor {
    pub source_topic: String,
    pub local_offset: u64,
    pub last_attempt_offset: u64,
    pub remote_acked_offset: u64,
    pub backlog: u64,
}

#[derive(Debug, Default)]
pub struct GeoReplicator {
    /// peer_name → ClusterPeer
    peers: BTreeMap<String, ClusterPeer>,
    /// (peer_name, topic) → cursor
    cursors: BTreeMap<(String, String), ReplicationCursor>,
    /// topics enrolled for replication
    topics: BTreeSet<String>,
}

impl GeoReplicator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_peer(&mut self, peer: ClusterPeer) {
        self.peers.insert(peer.name.clone(), peer);
    }

    pub fn remove_peer(&mut self, name: &str) -> bool {
        let removed = self.peers.remove(name).is_some();
        if removed {
            self.cursors.retain(|(p, _), _| p != name);
        }
        removed
    }

    pub fn add_topic(&mut self, topic: &str) {
        self.topics.insert(topic.to_string());
        // Initialize a cursor for each peer.
        for peer in self.peers.keys() {
            self.cursors
                .entry((peer.clone(), topic.to_string()))
                .or_insert_with(|| ReplicationCursor {
                    source_topic: topic.into(),
                    local_offset: 0,
                    last_attempt_offset: 0,
                    remote_acked_offset: 0,
                    backlog: 0,
                });
        }
    }

    pub fn remove_topic(&mut self, topic: &str) -> bool {
        let removed = self.topics.remove(topic);
        if removed {
            self.cursors.retain(|(_, t), _| t != topic);
        }
        removed
    }

    /// Advance the local source cursor (called whenever the producer commits
    /// new records).
    pub fn advance_local(&mut self, topic: &str, new_offset: u64) -> Result<(), GeoRepError> {
        if !self.topics.contains(topic) {
            return Err(GeoRepError::UnknownTopic(topic.into()));
        }
        for ((_, t), cursor) in self.cursors.iter_mut() {
            if t == topic {
                if new_offset < cursor.local_offset {
                    return Err(GeoRepError::CursorRegression {
                        topic: topic.into(),
                        from: cursor.local_offset,
                        to: new_offset,
                    });
                }
                cursor.local_offset = new_offset;
                cursor.backlog = cursor.local_offset - cursor.remote_acked_offset;
            }
        }
        Ok(())
    }

    /// Returns the next batch the replicator should attempt for a peer/topic.
    pub fn next_batch(&self, peer_name: &str, topic: &str, max: u64) -> Option<(u64, u64)> {
        let cursor = self.cursors.get(&(peer_name.into(), topic.into()))?;
        if cursor.last_attempt_offset >= cursor.local_offset {
            return None;
        }
        let from = cursor.last_attempt_offset;
        let to = std::cmp::min(from + max, cursor.local_offset);
        Some((from, to))
    }

    pub fn record_attempt(&mut self, peer_name: &str, topic: &str, up_to: u64) -> Result<(), GeoRepError> {
        let cursor = self
            .cursors
            .get_mut(&(peer_name.into(), topic.into()))
            .ok_or_else(|| GeoRepError::UnknownCluster(peer_name.into()))?;
        cursor.last_attempt_offset = up_to;
        Ok(())
    }

    pub fn record_ack(&mut self, peer_name: &str, topic: &str, up_to: u64) -> Result<(), GeoRepError> {
        let cursor = self
            .cursors
            .get_mut(&(peer_name.into(), topic.into()))
            .ok_or_else(|| GeoRepError::UnknownCluster(peer_name.into()))?;
        cursor.remote_acked_offset = up_to;
        cursor.backlog = cursor.local_offset.saturating_sub(cursor.remote_acked_offset);
        Ok(())
    }

    /// Force re-attempt of any unacked offsets (e.g. after a transient
    /// network failure).
    pub fn retry_unacked(&mut self, peer_name: &str, topic: &str) -> Result<(), GeoRepError> {
        let cursor = self
            .cursors
            .get_mut(&(peer_name.into(), topic.into()))
            .ok_or_else(|| GeoRepError::UnknownCluster(peer_name.into()))?;
        cursor.last_attempt_offset = cursor.remote_acked_offset;
        Ok(())
    }

    pub fn cursor(&self, peer_name: &str, topic: &str) -> Option<&ReplicationCursor> {
        self.cursors.get(&(peer_name.into(), topic.into()))
    }

    pub fn total_backlog(&self) -> u64 {
        self.cursors.values().map(|c| c.backlog).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(name: &str) -> ClusterPeer {
        ClusterPeer {
            name: name.into(),
            url: format!("pulsar://{}.example.com:6650", name),
            direction: ReplicationDirection::Outbound,
        }
    }

    #[test]
    fn add_peer_then_topic_initializes_cursor() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        assert!(g.cursor("eu-west", "orders").is_some());
    }

    #[test]
    fn advance_local_unknown_topic_errors() {
        let mut g = GeoReplicator::new();
        let err = g.advance_local("missing", 100).unwrap_err();
        assert!(matches!(err, GeoRepError::UnknownTopic(_)));
    }

    #[test]
    fn advance_local_updates_backlog() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.advance_local("orders", 100).unwrap();
        assert_eq!(g.cursor("eu-west", "orders").unwrap().backlog, 100);
    }

    #[test]
    fn cursor_regression_errors() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.advance_local("orders", 100).unwrap();
        let err = g.advance_local("orders", 50).unwrap_err();
        assert!(matches!(err, GeoRepError::CursorRegression { .. }));
    }

    #[test]
    fn next_batch_capped_at_max() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.advance_local("orders", 1000).unwrap();
        let (from, to) = g.next_batch("eu-west", "orders", 200).unwrap();
        assert_eq!(from, 0);
        assert_eq!(to, 200);
    }

    #[test]
    fn next_batch_none_when_caught_up() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.advance_local("orders", 100).unwrap();
        g.record_attempt("eu-west", "orders", 100).unwrap();
        assert!(g.next_batch("eu-west", "orders", 100).is_none());
    }

    #[test]
    fn record_ack_drains_backlog() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.advance_local("orders", 100).unwrap();
        g.record_attempt("eu-west", "orders", 100).unwrap();
        g.record_ack("eu-west", "orders", 100).unwrap();
        assert_eq!(g.cursor("eu-west", "orders").unwrap().backlog, 0);
    }

    #[test]
    fn retry_unacked_rewinds_attempt() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.advance_local("orders", 100).unwrap();
        g.record_attempt("eu-west", "orders", 100).unwrap();
        // Network blip — never acked.
        g.retry_unacked("eu-west", "orders").unwrap();
        assert_eq!(g.cursor("eu-west", "orders").unwrap().last_attempt_offset, 0);
    }

    #[test]
    fn remove_peer_drops_cursors() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.remove_peer("eu-west");
        assert!(g.cursor("eu-west", "orders").is_none());
    }

    #[test]
    fn remove_topic_drops_cursors() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_topic("orders");
        g.remove_topic("orders");
        assert!(g.cursor("eu-west", "orders").is_none());
    }

    #[test]
    fn total_backlog_sums_across_peers() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu"));
        g.add_peer(peer("us"));
        g.add_topic("orders");
        g.advance_local("orders", 100).unwrap();
        // 2 peers × 100 = 200 backlog.
        assert_eq!(g.total_backlog(), 200);
    }

    #[test]
    fn record_attempt_unknown_peer_errors() {
        let mut g = GeoReplicator::new();
        let err = g.record_attempt("missing", "orders", 0).unwrap_err();
        assert!(matches!(err, GeoRepError::UnknownCluster(_)));
    }

    #[test]
    fn replication_direction_enum_variants() {
        let _ = ReplicationDirection::Outbound;
        let _ = ReplicationDirection::Inbound;
        let _ = ReplicationDirection::Bidirectional;
    }

    #[test]
    fn topic_added_after_peer_initializes_cursor_for_existing_peers() {
        let mut g = GeoReplicator::new();
        g.add_peer(peer("eu-west"));
        g.add_peer(peer("us-east"));
        g.add_topic("orders");
        assert!(g.cursor("eu-west", "orders").is_some());
        assert!(g.cursor("us-east", "orders").is_some());
    }
}
