//! cave-ha — High Availability and Disaster Recovery for the CAVE runtime.
//!
//! Multiple bare-metal CAVE runtime instances form a cluster. One is elected
//! **leader** via Raft consensus; the others are **followers**. The leader
//! handles all writes and replicates state to followers. If the leader fails,
//! the remaining healthy instances elect a new one automatically.
//!
//! Cross-datacenter DR is supported via active-passive or active-active
//! site pairs with configurable RPO/RTO targets.

pub mod dr;
pub mod failover;
pub mod health;
pub mod models;
pub mod raft;
pub mod replication;
pub mod routes;

use axum::Router;
use chrono::Utc;
use models::{
    ClusterTopology, DRConfig, FailoverEvent, HeartbeatConfig, HealthVote, InstanceRole,
    InstanceStatus, RaftState, ReplicationConfig, ReplicationMode, RuntimeInstance,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Shared HA state — held behind `Arc` so it can be cloned into each Axum handler.
///
/// Mutable sub-fields (Raft state, topology) are wrapped in `tokio::sync::RwLock`
/// for safe concurrent access across async tasks. Immutable config fields are
/// plain values set at startup.
pub struct HaState {
    /// Raft consensus state: term, vote, commit index.
    pub raft: RwLock<RaftState>,
    /// Live cluster topology: all instances, elected leader, quorum size.
    pub topology: RwLock<ClusterTopology>,
    /// Immutable identity of this runtime process.
    pub self_instance: RuntimeInstance,
    /// Heartbeat timing thresholds.
    pub heartbeat_config: HeartbeatConfig,
    /// Replication mode (sync/async) and follower targets.
    pub replication_config: ReplicationConfig,
    /// Optional DR site pairing (None if running single-site).
    pub dr_config: Option<DRConfig>,
    /// Ordered audit log of all failover events.
    pub failover_history: RwLock<Vec<FailoverEvent>>,
    /// Peer health votes used for consensus-based failure detection.
    pub health_votes: RwLock<Vec<HealthVote>>,
}

impl HaState {
    pub fn new(
        self_id: Uuid,
        hostname: String,
        ip: String,
        port: u16,
        datacenter: String,
    ) -> Self {
        let now = Utc::now();
        let self_instance = RuntimeInstance {
            id: self_id,
            hostname,
            ip,
            port,
            role: InstanceRole::Follower,
            status: InstanceStatus::Healthy,
            last_heartbeat: now,
            datacenter,
            started_at: now,
        };
        let topology = ClusterTopology {
            instances: vec![self_instance.clone()],
            leader: None,
            quorum_size: 1,
            split_brain_protection: true,
        };
        Self {
            raft: RwLock::new(RaftState {
                current_term: 0,
                voted_for: None,
                commit_index: 0,
                last_applied: 0,
                leader_id: None,
            }),
            topology: RwLock::new(topology),
            self_instance,
            heartbeat_config: HeartbeatConfig {
                interval_ms: 150,
                timeout_ms: 500,
                max_missed: 3,
            },
            replication_config: ReplicationConfig {
                mode: ReplicationMode::Sync,
                targets: vec![],
                lag_tolerance: 1000,
            },
            dr_config: None,
            failover_history: RwLock::new(vec![]),
            health_votes: RwLock::new(vec![]),
        }
    }
}

impl Default for HaState {
    fn default() -> Self {
        Self::new(
            Uuid::new_v4(),
            std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string()),
            "127.0.0.1".to_string(),
            8080,
            "default".to_string(),
        )
    }
}

/// Create the HA Axum router and attach the shared state.
pub fn router(state: Arc<HaState>) -> Router {
    routes::create_router(state)
}
