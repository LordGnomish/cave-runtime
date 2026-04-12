//! cave-ha — High Availability and Disaster Recovery using Raft consensus.
//!
//! Provides:
//! - Full Raft consensus engine (leader election, log replication, snapshots)
//! - Cross-datacenter replication with configurable consistency levels
//! - Automatic and manual failover with split-brain protection
//! - Cluster health monitoring
//! - Disaster recovery: backup/restore, PITR, geo-redundant snapshots

pub mod dr;
pub mod failover;
pub mod health;
pub mod raft;
pub mod replication;
pub mod snapshot;

pub use raft::{LogEntry, NodeId, RaftMessage, RaftNode, RaftRole};
pub use snapshot::{Snapshot, SnapshotManager};
pub use replication::{ConsistencyLevel, CrossDcReplicator, ReplicationConfig, ReplicationStatus};
pub use failover::{FailoverEvent, FailoverManager, FailoverReason};
pub use health::{ClusterHealth, ClusterHealthMonitor, NodeHealth, NodeHealthStatus, ResourceUsage};
pub use dr::{BackupMetadata, DisasterRecovery};
