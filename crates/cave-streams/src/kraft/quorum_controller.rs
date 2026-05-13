//! `QuorumController` — the state machine that owns the
//! metadata log. Accepts client requests (create topic, register
//! broker, set config), validates them against the current
//! materialised state, and emits the corresponding records into
//! the log.
//!
//! Mirrors `org.apache.kafka.controller.QuorumController` from
//! upstream — the surface, not the raft replication. cave-streams
//! delegates the actual append/commit to the existing
//! `MetadataLog`; integrating with cave-etcd's raft is a tracked
//! follow-up (see the module doc on [`super`]).

use std::sync::Arc;

use uuid::Uuid;

use super::epoch::{ControllerEpoch, VoterSet};
use super::metadata::{
    BrokerRegistration, ClusterMetadata, ConfigRecord, MetadataRecord, PartitionRecord,
    TopicRecord,
};
use super::metadata_log::MetadataLog;

/// Requests the controller accepts from clients (admin API +
/// broker self-registration).
#[derive(Debug, Clone)]
pub enum ControllerRequest {
    CreateTopic {
        name: String,
        partition_count: i32,
        replication_factor: i16,
    },
    DeleteTopic {
        name: String,
    },
    RegisterBroker {
        broker_id: i32,
        host: String,
        port: u16,
    },
    UnregisterBroker {
        broker_id: i32,
    },
    SetConfig {
        scope: String,
        target: String,
        key: String,
        value: Option<String>,
    },
}

/// Responses the controller emits — caller correlates these
/// with the request via the offset returned in the success path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerResponse {
    Ok {
        /// First offset in the batch this request produced.
        offset: u64,
        /// Number of records the request emitted (1 for
        /// register-broker, 1+partition_count for create-topic).
        record_count: usize,
    },
    /// The request can't be applied — pre-condition violated.
    /// `reason` matches Kafka's `*Exception` message style.
    Rejected {
        reason: String,
    },
    /// Caller wasn't the elected leader. Includes the current
    /// leader id (if any) so the caller can redirect.
    NotLeader {
        current_leader: Option<i32>,
    },
}

/// The state machine itself. `&self` everywhere; interior
/// mutability via `MetadataLog`'s `RwLock`.
pub struct QuorumController {
    /// This node's broker ID — used for leader-check.
    self_id: i32,
    /// Voter set + epoch tracker.
    voters: std::sync::RwLock<VoterSet>,
    /// The compacted metadata log.
    log: Arc<MetadataLog>,
}

impl QuorumController {
    /// New controller with the given voter set. `MetadataLog`
    /// starts empty; caller may pre-populate (e.g. recovery
    /// from snapshot) by appending to it before any
    /// `submit_request` call.
    pub fn new(self_id: i32, voters: VoterSet, log: Arc<MetadataLog>) -> Self {
        Self {
            self_id,
            voters: std::sync::RwLock::new(voters),
            log,
        }
    }

    /// Test/debug helper: force-elect this node as leader at
    /// `epoch`. In production this fires off the back of the
    /// raft transport receiving an `EndQuorumEpoch` RPC. The
    /// transport isn't wired yet — see the module doc.
    pub fn force_become_leader(&self, epoch: ControllerEpoch) -> Result<(), String> {
        let mut v = self.voters.write().expect("poisoned");
        v.elect(self.self_id, epoch)
    }

    /// Read-only view of the materialised state.
    pub fn snapshot(&self) -> ClusterMetadata {
        self.log.snapshot()
    }

    /// Current epoch.
    pub fn epoch(&self) -> ControllerEpoch {
        self.voters.read().expect("poisoned").epoch()
    }

    /// Is this node the elected leader?
    pub fn is_leader(&self) -> bool {
        self.voters.read().expect("poisoned").leader() == Some(self.self_id)
    }

    /// Submit a request to the controller. Returns immediately —
    /// in production the response would arrive after raft
    /// commit; cave-streams single-node mode commits inline.
    pub fn submit_request(&self, req: ControllerRequest) -> ControllerResponse {
        if !self.is_leader() {
            return ControllerResponse::NotLeader {
                current_leader: self.voters.read().expect("poisoned").leader(),
            };
        }
        let epoch = self.epoch();
        match req {
            ControllerRequest::CreateTopic {
                name,
                partition_count,
                replication_factor,
            } => self.create_topic(epoch, name, partition_count, replication_factor),
            ControllerRequest::DeleteTopic { name } => self.delete_topic(epoch, name),
            ControllerRequest::RegisterBroker {
                broker_id,
                host,
                port,
            } => self.register_broker(epoch, broker_id, host, port),
            ControllerRequest::UnregisterBroker { broker_id } => {
                self.unregister_broker(epoch, broker_id)
            }
            ControllerRequest::SetConfig {
                scope,
                target,
                key,
                value,
            } => self.set_config(epoch, scope, target, key, value),
        }
    }

    fn create_topic(
        &self,
        epoch: ControllerEpoch,
        name: String,
        partition_count: i32,
        replication_factor: i16,
    ) -> ControllerResponse {
        if name.is_empty() {
            return ControllerResponse::Rejected {
                reason: "topic name must not be empty".into(),
            };
        }
        if partition_count <= 0 {
            return ControllerResponse::Rejected {
                reason: format!("partition count must be > 0 (got {partition_count})"),
            };
        }
        if replication_factor <= 0 {
            return ControllerResponse::Rejected {
                reason: format!("replication factor must be > 0 (got {replication_factor})"),
            };
        }
        let snap = self.log.snapshot();
        if snap.topics.contains_key(&name) {
            return ControllerResponse::Rejected {
                reason: format!("topic already exists: {name}"),
            };
        }
        let live_brokers: Vec<i32> = snap.brokers.values().filter(|b| !b.fenced).map(|b| b.broker_id).collect();
        if (replication_factor as usize) > live_brokers.len().max(1) {
            return ControllerResponse::Rejected {
                reason: format!(
                    "not enough live brokers: have {}, need {}",
                    live_brokers.len(),
                    replication_factor
                ),
            };
        }

        let topic_id = Uuid::new_v4();
        let mut batch = Vec::with_capacity(1 + partition_count as usize);
        batch.push(MetadataRecord::Topic {
            epoch,
            record: TopicRecord {
                name: name.clone(),
                topic_id,
                partition_count,
                replication_factor,
            },
        });
        for p in 0..partition_count {
            // Naive round-robin replica placement — exact
            // mirror of Kafka's `AdminUtils.assignReplicasToBrokers`
            // is its own port. Single-node case (no live brokers
            // yet) falls back to `self_id`.
            let leader = if live_brokers.is_empty() {
                self.self_id
            } else {
                live_brokers[(p as usize) % live_brokers.len()]
            };
            batch.push(MetadataRecord::Partition {
                epoch,
                record: PartitionRecord {
                    topic: name.clone(),
                    partition_id: p,
                    leader,
                    isr: vec![leader],
                    replicas: vec![leader],
                    leader_epoch: 0,
                },
            });
        }
        let count = batch.len();
        let entries = self.log.append_batch(batch);
        ControllerResponse::Ok {
            offset: entries[0].offset,
            record_count: count,
        }
    }

    fn delete_topic(&self, epoch: ControllerEpoch, name: String) -> ControllerResponse {
        let snap = self.log.snapshot();
        if !snap.topics.contains_key(&name) {
            return ControllerResponse::Rejected {
                reason: format!("topic does not exist: {name}"),
            };
        }
        let entry = self
            .log
            .append(MetadataRecord::TopicRemoved { epoch, name });
        ControllerResponse::Ok {
            offset: entry.offset,
            record_count: 1,
        }
    }

    fn register_broker(
        &self,
        epoch: ControllerEpoch,
        broker_id: i32,
        host: String,
        port: u16,
    ) -> ControllerResponse {
        if host.is_empty() {
            return ControllerResponse::Rejected {
                reason: "broker host must not be empty".into(),
            };
        }
        let entry = self.log.append(MetadataRecord::Broker {
            epoch,
            record: BrokerRegistration {
                broker_id,
                host,
                port,
                incarnation_id: Uuid::new_v4(),
                fenced: false,
            },
        });
        ControllerResponse::Ok {
            offset: entry.offset,
            record_count: 1,
        }
    }

    fn unregister_broker(&self, epoch: ControllerEpoch, broker_id: i32) -> ControllerResponse {
        let entry = self
            .log
            .append(MetadataRecord::BrokerUnregistered { epoch, broker_id });
        ControllerResponse::Ok {
            offset: entry.offset,
            record_count: 1,
        }
    }

    fn set_config(
        &self,
        epoch: ControllerEpoch,
        scope: String,
        target: String,
        key: String,
        value: Option<String>,
    ) -> ControllerResponse {
        if scope.is_empty() || target.is_empty() || key.is_empty() {
            return ControllerResponse::Rejected {
                reason: "config scope/target/key must all be non-empty".into(),
            };
        }
        let entry = self.log.append(MetadataRecord::Config {
            epoch,
            record: ConfigRecord {
                scope,
                target,
                key,
                value,
            },
        });
        ControllerResponse::Ok {
            offset: entry.offset,
            record_count: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_leader_controller(self_id: i32) -> QuorumController {
        let log = Arc::new(MetadataLog::new());
        let voters = VoterSet::new([self_id]);
        let c = QuorumController::new(self_id, voters, log);
        c.force_become_leader(ControllerEpoch(1)).unwrap();
        c
    }

    #[test]
    fn not_leader_rejected_until_elected() {
        let log = Arc::new(MetadataLog::new());
        let voters = VoterSet::new([1, 2]);
        let c = QuorumController::new(1, voters, log);
        let r = c.submit_request(ControllerRequest::CreateTopic {
            name: "t".into(),
            partition_count: 1,
            replication_factor: 1,
        });
        assert!(matches!(r, ControllerResponse::NotLeader { .. }));
        assert!(!c.is_leader());
    }

    #[test]
    fn force_become_leader_advances_epoch() {
        let c = make_leader_controller(1);
        assert!(c.is_leader());
        assert_eq!(c.epoch(), ControllerEpoch(1));
    }

    #[test]
    fn create_topic_emits_topic_plus_partitions() {
        let c = make_leader_controller(1);
        let r = c.submit_request(ControllerRequest::CreateTopic {
            name: "orders".into(),
            partition_count: 3,
            replication_factor: 1,
        });
        match r {
            ControllerResponse::Ok { offset, record_count } => {
                assert_eq!(offset, 0);
                assert_eq!(record_count, 4); // 1 topic + 3 partitions
            }
            other => panic!("unexpected: {other:?}"),
        }
        let snap = c.snapshot();
        assert!(snap.topics.contains_key("orders"));
        assert_eq!(snap.partitions.len(), 3);
        for p in 0..3 {
            assert_eq!(snap.partitions[&("orders".into(), p)].leader, 1);
        }
    }

    #[test]
    fn create_topic_rejects_empty_name() {
        let c = make_leader_controller(1);
        assert!(matches!(
            c.submit_request(ControllerRequest::CreateTopic {
                name: "".into(),
                partition_count: 1,
                replication_factor: 1,
            }),
            ControllerResponse::Rejected { .. }
        ));
    }

    #[test]
    fn create_topic_rejects_duplicate() {
        let c = make_leader_controller(1);
        let _ = c.submit_request(ControllerRequest::CreateTopic {
            name: "orders".into(),
            partition_count: 1,
            replication_factor: 1,
        });
        let r2 = c.submit_request(ControllerRequest::CreateTopic {
            name: "orders".into(),
            partition_count: 1,
            replication_factor: 1,
        });
        assert!(matches!(r2, ControllerResponse::Rejected { .. }));
    }

    #[test]
    fn create_topic_rejects_insufficient_brokers() {
        let c = make_leader_controller(1);
        // No brokers registered yet — RF=3 with no live brokers
        // falls back to "max(live, 1)" = 1, so RF=3 must reject.
        let r = c.submit_request(ControllerRequest::CreateTopic {
            name: "t".into(),
            partition_count: 1,
            replication_factor: 3,
        });
        assert!(matches!(r, ControllerResponse::Rejected { .. }));
    }

    #[test]
    fn delete_topic_emits_tombstone() {
        let c = make_leader_controller(1);
        c.submit_request(ControllerRequest::CreateTopic {
            name: "orders".into(),
            partition_count: 1,
            replication_factor: 1,
        });
        let r = c.submit_request(ControllerRequest::DeleteTopic {
            name: "orders".into(),
        });
        assert!(matches!(r, ControllerResponse::Ok { .. }));
        let snap = c.snapshot();
        assert!(!snap.topics.contains_key("orders"));
    }

    #[test]
    fn delete_unknown_topic_rejected() {
        let c = make_leader_controller(1);
        let r = c.submit_request(ControllerRequest::DeleteTopic {
            name: "missing".into(),
        });
        assert!(matches!(r, ControllerResponse::Rejected { .. }));
    }

    #[test]
    fn register_broker_appends() {
        let c = make_leader_controller(1);
        c.submit_request(ControllerRequest::RegisterBroker {
            broker_id: 2,
            host: "host-b".into(),
            port: 9092,
        });
        let snap = c.snapshot();
        assert!(snap.brokers.contains_key(&2));
        assert_eq!(snap.brokers[&2].host, "host-b");
    }

    #[test]
    fn unregister_broker_clears_entry() {
        let c = make_leader_controller(1);
        c.submit_request(ControllerRequest::RegisterBroker {
            broker_id: 2,
            host: "h".into(),
            port: 9092,
        });
        c.submit_request(ControllerRequest::UnregisterBroker { broker_id: 2 });
        let snap = c.snapshot();
        assert!(!snap.brokers.contains_key(&2));
    }

    #[test]
    fn replica_placement_round_robin_across_live_brokers() {
        let c = make_leader_controller(1);
        // Register 3 brokers.
        for (id, host) in [(1, "h1"), (2, "h2"), (3, "h3")] {
            c.submit_request(ControllerRequest::RegisterBroker {
                broker_id: id,
                host: host.into(),
                port: 9092,
            });
        }
        c.submit_request(ControllerRequest::CreateTopic {
            name: "t".into(),
            partition_count: 6,
            replication_factor: 1,
        });
        let snap = c.snapshot();
        // Each broker should lead 2 partitions (6 / 3) — exact
        // round-robin order depends on BTreeMap iteration, but
        // the count should be even.
        let mut counts = std::collections::HashMap::new();
        for p in 0..6 {
            *counts
                .entry(snap.partitions[&("t".into(), p)].leader)
                .or_insert(0u32) += 1;
        }
        assert_eq!(counts.len(), 3, "all 3 brokers should lead at least one");
        for c in counts.values() {
            assert_eq!(*c, 2);
        }
    }

    #[test]
    fn config_set_then_unset() {
        let c = make_leader_controller(1);
        c.submit_request(ControllerRequest::SetConfig {
            scope: "broker".into(),
            target: "1".into(),
            key: "log.retention.ms".into(),
            value: Some("86400000".into()),
        });
        let snap = c.snapshot();
        assert_eq!(
            snap.configs.get(&("broker".into(), "1".into(), "log.retention.ms".into())),
            Some(&"86400000".to_string())
        );
        c.submit_request(ControllerRequest::SetConfig {
            scope: "broker".into(),
            target: "1".into(),
            key: "log.retention.ms".into(),
            value: None,
        });
        assert!(c.snapshot().configs.is_empty());
    }

    #[test]
    fn config_rejects_empty_components() {
        let c = make_leader_controller(1);
        assert!(matches!(
            c.submit_request(ControllerRequest::SetConfig {
                scope: "".into(),
                target: "1".into(),
                key: "k".into(),
                value: Some("v".into()),
            }),
            ControllerResponse::Rejected { .. }
        ));
    }
}
