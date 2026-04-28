//! `Cluster` CRD — primary + N replicas with synchronous and asynchronous
//! replication, controlled switchover, automatic failover.
//!
//! Mirrors `cloudnative-pg/cloudnative-pg/api/v1/cluster_types.go`.
//!
//! Reconciler invariants:
//!
//! 1. Exactly one instance carries `InstanceRole::Primary` when healthy.
//! 2. `Status::current_primary` matches the primary instance name.
//! 3. `pg_current_wal_lsn()` of the primary ≥ `last_received_lsn` of all
//!    synchronous replicas.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ClusterError {
    #[error("instance not found: {0}")]
    InstanceNotFound(String),
    #[error("invalid replica count: {0}")]
    InvalidReplicaCount(i32),
    #[error("primary already exists: {0}")]
    PrimaryAlreadyExists(String),
    #[error("no eligible primary candidate")]
    NoCandidate,
    #[error("invalid switchover target: {0}")]
    InvalidSwitchoverTarget(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceRole {
    Primary,
    Replica,
    Designated, // designated primary during switchover
    Joining,
    Recovering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterReplicationMode {
    /// Streaming replication, async by default.
    Streaming,
    /// Synchronous replication — primary waits for ACK from at least
    /// `min_synchronous_replicas` replicas.
    Synchronous,
    /// Quorum-based synchronous replication — wait for any N of M replicas.
    QuorumSync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterPhase {
    Creating,
    InitializingPgWal,
    Healthy,
    Upgrading,
    SwitchoverInProgress,
    FailoverInProgress,
    Hibernating,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ClusterSpec {
    pub name: String,
    pub instances: i32,
    pub image_name: String,
    pub bootstrap_database: String,
    pub bootstrap_owner: String,
    pub replication_mode: ClusterReplicationMode,
    pub min_synchronous_replicas: i32,
    pub max_synchronous_replicas: i32,
    pub storage_size_gib: i32,
    pub backup_retention_days: i32,
}

impl Default for ClusterSpec {
    fn default() -> Self {
        Self {
            name: "cluster".into(),
            instances: 3,
            image_name: "ghcr.io/cloudnative-pg/postgresql:16.2".into(),
            bootstrap_database: "app".into(),
            bootstrap_owner: "app".into(),
            replication_mode: ClusterReplicationMode::Streaming,
            min_synchronous_replicas: 0,
            max_synchronous_replicas: 0,
            storage_size_gib: 10,
            backup_retention_days: 30,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub name: String,
    pub role: InstanceRole,
    pub healthy: bool,
    pub current_lsn: u64,
    pub last_received_lsn: u64,
    pub timeline_id: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ReplicationSlot {
    pub name: String,
    pub kind: ReplicationSlotKind,
    pub active: bool,
    pub restart_lsn: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationSlotKind {
    Physical,
    Logical,
}

#[derive(Debug, Clone)]
pub struct ClusterStatus {
    pub phase: ClusterPhase,
    pub current_primary: Option<String>,
    pub target_primary: Option<String>,
    pub instances: BTreeMap<String, Instance>,
    pub replication_slots: BTreeMap<String, ReplicationSlot>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone)]
pub struct Condition {
    pub kind: String,
    pub status: bool,
    pub reason: String,
    pub last_transition: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Cluster {
    pub spec: ClusterSpec,
    pub status: ClusterStatus,
}

impl Cluster {
    pub fn new(spec: ClusterSpec) -> Self {
        Self {
            spec,
            status: ClusterStatus {
                phase: ClusterPhase::Creating,
                current_primary: None,
                target_primary: None,
                instances: BTreeMap::new(),
                replication_slots: BTreeMap::new(),
                conditions: Vec::new(),
            },
        }
    }

    pub fn add_instance(&mut self, name: &str, now: DateTime<Utc>) {
        self.status.instances.insert(
            name.to_string(),
            Instance {
                name: name.to_string(),
                role: InstanceRole::Joining,
                healthy: true,
                current_lsn: 0,
                last_received_lsn: 0,
                timeline_id: 1,
                created_at: now,
            },
        );
    }

    pub fn promote(&mut self, name: &str) -> Result<(), ClusterError> {
        // Demote any existing primary first.
        for inst in self.status.instances.values_mut() {
            if inst.role == InstanceRole::Primary {
                inst.role = InstanceRole::Replica;
            }
        }
        let inst = self
            .status
            .instances
            .get_mut(name)
            .ok_or_else(|| ClusterError::InstanceNotFound(name.into()))?;
        inst.role = InstanceRole::Primary;
        inst.timeline_id += 1;
        self.status.current_primary = Some(name.to_string());
        self.status.target_primary = None;
        Ok(())
    }

    pub fn mark_unhealthy(&mut self, name: &str) -> Result<(), ClusterError> {
        let inst = self
            .status
            .instances
            .get_mut(name)
            .ok_or_else(|| ClusterError::InstanceNotFound(name.into()))?;
        inst.healthy = false;
        Ok(())
    }

    pub fn primary(&self) -> Option<&Instance> {
        self.status
            .instances
            .values()
            .find(|i| i.role == InstanceRole::Primary)
    }

    pub fn replicas(&self) -> Vec<&Instance> {
        self.status
            .instances
            .values()
            .filter(|i| i.role == InstanceRole::Replica)
            .collect()
    }

    pub fn healthy_replicas(&self) -> Vec<&Instance> {
        self.status
            .instances
            .values()
            .filter(|i| i.role == InstanceRole::Replica && i.healthy)
            .collect()
    }

    /// Choose the next primary by `current_lsn` (most up-to-date wins).
    pub fn choose_primary_candidate(&self) -> Result<String, ClusterError> {
        let mut best: Option<&Instance> = None;
        for inst in self.status.instances.values() {
            if inst.healthy && inst.role != InstanceRole::Primary {
                match best {
                    None => best = Some(inst),
                    Some(b) if inst.current_lsn > b.current_lsn => best = Some(inst),
                    _ => {}
                }
            }
        }
        best.map(|i| i.name.clone()).ok_or(ClusterError::NoCandidate)
    }

    /// Check if synchronous-replication invariant is satisfied.
    pub fn sync_replication_ok(&self) -> bool {
        if self.spec.replication_mode == ClusterReplicationMode::Streaming {
            return true;
        }
        let need = self.spec.min_synchronous_replicas as usize;
        let primary = match self.primary() {
            Some(p) => p,
            None => return false,
        };
        let in_sync = self
            .healthy_replicas()
            .iter()
            .filter(|r| r.last_received_lsn >= primary.current_lsn)
            .count();
        in_sync >= need
    }

    pub fn create_physical_slot(&mut self, name: &str) {
        self.status.replication_slots.insert(
            name.to_string(),
            ReplicationSlot {
                name: name.to_string(),
                kind: ReplicationSlotKind::Physical,
                active: true,
                restart_lsn: 0,
            },
        );
    }

    pub fn drop_slot(&mut self, name: &str) -> bool {
        self.status.replication_slots.remove(name).is_some()
    }

    pub fn set_phase(&mut self, phase: ClusterPhase) {
        self.status.phase = phase;
    }

    pub fn set_condition(&mut self, kind: &str, status: bool, reason: &str, now: DateTime<Utc>) {
        self.status.conditions.retain(|c| c.kind != kind);
        self.status.conditions.push(Condition {
            kind: kind.into(),
            status,
            reason: reason.into(),
            last_transition: now,
        });
    }
}

// ── Reconciler decisions ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchoverDecision {
    NoOp,
    Promote(String),
    Reject(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailoverDecision {
    NoOp,
    PromoteCandidate(String),
    NoCandidate,
}

pub fn plan_switchover(cluster: &Cluster, target: &str) -> SwitchoverDecision {
    let inst = match cluster.status.instances.get(target) {
        Some(i) => i,
        None => return SwitchoverDecision::Reject(format!("unknown {}", target)),
    };
    if !inst.healthy {
        return SwitchoverDecision::Reject("target unhealthy".into());
    }
    if inst.role == InstanceRole::Primary {
        return SwitchoverDecision::NoOp;
    }
    SwitchoverDecision::Promote(target.into())
}

pub fn plan_failover(cluster: &Cluster) -> FailoverDecision {
    let primary_dead = cluster.primary().map(|p| !p.healthy).unwrap_or(true);
    if !primary_dead {
        return FailoverDecision::NoOp;
    }
    match cluster.choose_primary_candidate() {
        Ok(name) => FailoverDecision::PromoteCandidate(name),
        Err(_) => FailoverDecision::NoCandidate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster_with_3() -> Cluster {
        let now = Utc::now();
        let mut c = Cluster::new(ClusterSpec::default());
        c.add_instance("c-1", now);
        c.add_instance("c-2", now);
        c.add_instance("c-3", now);
        c
    }

    #[test]
    fn new_cluster_starts_in_creating() {
        let c = Cluster::new(ClusterSpec::default());
        assert_eq!(c.status.phase, ClusterPhase::Creating);
    }

    #[test]
    fn add_instance_marks_joining() {
        let c = cluster_with_3();
        assert_eq!(c.status.instances["c-1"].role, InstanceRole::Joining);
    }

    #[test]
    fn promote_sets_primary_and_demotes_others() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.promote("c-2").unwrap();
        assert_eq!(c.primary().unwrap().name, "c-2");
        assert_eq!(c.status.instances["c-1"].role, InstanceRole::Replica);
    }

    #[test]
    fn promote_unknown_errors() {
        let mut c = cluster_with_3();
        let err = c.promote("ghost").unwrap_err();
        assert_eq!(err, ClusterError::InstanceNotFound("ghost".into()));
    }

    #[test]
    fn promote_advances_timeline() {
        let mut c = cluster_with_3();
        let initial_tid = c.status.instances["c-1"].timeline_id;
        c.promote("c-1").unwrap();
        assert_eq!(c.status.instances["c-1"].timeline_id, initial_tid + 1);
    }

    #[test]
    fn mark_unhealthy_flips_flag() {
        let mut c = cluster_with_3();
        c.mark_unhealthy("c-1").unwrap();
        assert!(!c.status.instances["c-1"].healthy);
    }

    #[test]
    fn choose_candidate_picks_highest_lsn() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        // c-1 = primary (excluded from candidates)
        c.status.instances.get_mut("c-2").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-3").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-2").unwrap().current_lsn = 100;
        c.status.instances.get_mut("c-3").unwrap().current_lsn = 200;
        let picked = c.choose_primary_candidate().unwrap();
        assert_eq!(picked, "c-3");
    }

    #[test]
    fn choose_candidate_no_healthy_errors() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.mark_unhealthy("c-2").unwrap();
        c.mark_unhealthy("c-3").unwrap();
        let err = c.choose_primary_candidate().unwrap_err();
        assert_eq!(err, ClusterError::NoCandidate);
    }

    #[test]
    fn primary_returns_promoted_instance() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        assert_eq!(c.primary().unwrap().name, "c-1");
    }

    #[test]
    fn replicas_excludes_primary() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.status.instances.get_mut("c-2").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-3").unwrap().role = InstanceRole::Replica;
        assert_eq!(c.replicas().len(), 2);
    }

    #[test]
    fn sync_replication_streaming_always_ok() {
        let c = cluster_with_3();
        assert!(c.sync_replication_ok());
    }

    #[test]
    fn sync_replication_requires_in_sync_replicas() {
        let mut c = cluster_with_3();
        c.spec.replication_mode = ClusterReplicationMode::Synchronous;
        c.spec.min_synchronous_replicas = 2;
        c.promote("c-1").unwrap();
        c.status.instances.get_mut("c-1").unwrap().current_lsn = 100;
        c.status.instances.get_mut("c-2").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-2").unwrap().last_received_lsn = 100;
        c.status.instances.get_mut("c-3").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-3").unwrap().last_received_lsn = 50;
        assert!(!c.sync_replication_ok());
        c.status.instances.get_mut("c-3").unwrap().last_received_lsn = 100;
        assert!(c.sync_replication_ok());
    }

    #[test]
    fn create_then_drop_slot() {
        let mut c = cluster_with_3();
        c.create_physical_slot("repl_c2");
        assert!(c.status.replication_slots.contains_key("repl_c2"));
        assert!(c.drop_slot("repl_c2"));
        assert!(!c.drop_slot("repl_c2"));
    }

    #[test]
    fn switchover_to_unhealthy_rejected() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.mark_unhealthy("c-2").unwrap();
        let d = plan_switchover(&c, "c-2");
        assert!(matches!(d, SwitchoverDecision::Reject(_)));
    }

    #[test]
    fn switchover_to_unknown_rejected() {
        let c = cluster_with_3();
        let d = plan_switchover(&c, "ghost");
        assert!(matches!(d, SwitchoverDecision::Reject(_)));
    }

    #[test]
    fn switchover_to_replica_promotes() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.status.instances.get_mut("c-2").unwrap().role = InstanceRole::Replica;
        let d = plan_switchover(&c, "c-2");
        assert_eq!(d, SwitchoverDecision::Promote("c-2".into()));
    }

    #[test]
    fn switchover_to_self_noop() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        let d = plan_switchover(&c, "c-1");
        assert_eq!(d, SwitchoverDecision::NoOp);
    }

    #[test]
    fn failover_with_healthy_primary_noop() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        let d = plan_failover(&c);
        assert_eq!(d, FailoverDecision::NoOp);
    }

    #[test]
    fn failover_promotes_candidate_when_primary_dead() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.mark_unhealthy("c-1").unwrap();
        c.status.instances.get_mut("c-2").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-2").unwrap().current_lsn = 100;
        c.status.instances.get_mut("c-3").unwrap().role = InstanceRole::Replica;
        c.status.instances.get_mut("c-3").unwrap().current_lsn = 50;
        let d = plan_failover(&c);
        assert_eq!(d, FailoverDecision::PromoteCandidate("c-2".into()));
    }

    #[test]
    fn failover_no_candidate_returns_no_candidate() {
        let mut c = cluster_with_3();
        c.promote("c-1").unwrap();
        c.mark_unhealthy("c-1").unwrap();
        c.mark_unhealthy("c-2").unwrap();
        c.mark_unhealthy("c-3").unwrap();
        assert_eq!(plan_failover(&c), FailoverDecision::NoCandidate);
    }

    #[test]
    fn condition_set_replaces_same_kind() {
        let mut c = cluster_with_3();
        c.set_condition("Ready", false, "starting", Utc::now());
        c.set_condition("Ready", true, "started", Utc::now());
        assert_eq!(c.status.conditions.len(), 1);
        assert!(c.status.conditions[0].status);
    }

    #[test]
    fn phase_transition() {
        let mut c = cluster_with_3();
        c.set_phase(ClusterPhase::Healthy);
        assert_eq!(c.status.phase, ClusterPhase::Healthy);
    }
}
