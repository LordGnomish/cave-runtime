// SPDX-License-Identifier: AGPL-3.0-or-later
//! RDBMS-operator wrap — native CloudNativePG-style cluster admin UI.
//!
//! Replaces the CloudNativePG web console. Platform admins manage Postgres
//! clusters, watch primary/replica state, trigger failovers, and inspect
//! backup history through cave-portal-api; writes happen through this
//! view's native form. **No** redirect to a CNPG dashboard exists.
//!
//! Panels (per ADR-147 portal contract):
//!   * `dashboard` — cluster list + per-state counters + Raft / leader status
//!   * `clusters` — CRUD form for Cluster CRDs (name, version, instances, storage)
//!   * `backups` — list + restore-to-PITR form
//!   * `failover_history` — chronological event feed

use super::ViewPersona;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Cluster state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterState {
    Creating,
    Running,
    Stopped,
    Failed,
    Promoting,
    Restarting,
    Deleting,
}

impl ClusterState {
    pub fn label(&self) -> &'static str {
        match self {
            ClusterState::Creating => "creating",
            ClusterState::Running => "running",
            ClusterState::Stopped => "stopped",
            ClusterState::Failed => "failed",
            ClusterState::Promoting => "promoting",
            ClusterState::Restarting => "restarting",
            ClusterState::Deleting => "deleting",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicaSyncState {
    Streaming,
    CatchingUp,
    Paused,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cluster {
    pub id: String,
    pub name: String,
    pub tenant: String,
    pub version: String,
    pub instances: u32,
    pub state: ClusterState,
    pub primary_id: String,
    pub storage_gib: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplicaInfo {
    pub instance_id: String,
    pub primary_id: String,
    pub sync_state: ReplicaSyncState,
    pub lag_bytes: i64,
    pub lag_seconds: f64,
    /// `true` when this instance is currently serving as primary.
    pub is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    pub cluster_id: String,
    pub kind: BackupKind,
    pub status: BackupStatus,
    pub size_bytes: u64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub wal_start_lsn: Option<String>,
    pub wal_end_lsn: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    Full,
    Incremental,
    Wal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailoverEvent {
    pub id: String,
    pub cluster_id: String,
    pub old_primary_id: String,
    pub new_primary_id: String,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: u64,
    pub automatic: bool,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RdbmsOperatorError {
    #[error("cluster {0:?} not found")]
    ClusterNotFound(String),
    #[error("cluster {0:?} already exists")]
    ClusterExists(String),
    #[error("invalid cluster spec: {0}")]
    InvalidSpec(String),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("failover blocked: {0}")]
    FailoverBlocked(String),
    #[error("backup {0:?} not found")]
    BackupNotFound(String),
}

// ── Plugin state ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct RdbmsOperatorPlugin {
    clusters: Vec<Cluster>,
    replicas: Vec<ReplicaInfo>,
    backups: Vec<BackupRecord>,
    failovers: Vec<FailoverEvent>,
}

impl RdbmsOperatorPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_cluster(&mut self, cluster: Cluster) -> Result<(), RdbmsOperatorError> {
        validate_name(&cluster.name)?;
        if cluster.instances == 0 {
            return Err(RdbmsOperatorError::InvalidSpec(
                "instances must be >= 1".into(),
            ));
        }
        if cluster.storage_gib == 0 {
            return Err(RdbmsOperatorError::InvalidSpec(
                "storage_gib must be >= 1".into(),
            ));
        }
        if self.clusters.iter().any(|c| c.name == cluster.name) {
            return Err(RdbmsOperatorError::ClusterExists(cluster.name));
        }
        self.clusters.push(cluster);
        Ok(())
    }

    pub fn register_replica(&mut self, replica: ReplicaInfo) {
        self.replicas.push(replica);
    }

    pub fn register_backup(&mut self, backup: BackupRecord) {
        self.backups.push(backup);
    }

    pub fn record_failover(&mut self, event: FailoverEvent) {
        self.failovers.push(event);
    }

    /// Dashboard panel — counts by state plus per-cluster lag.
    pub fn dashboard(&self) -> DashboardPanel {
        let mut by_state = StateCounts::default();
        for c in &self.clusters {
            match c.state {
                ClusterState::Creating => by_state.creating += 1,
                ClusterState::Running => by_state.running += 1,
                ClusterState::Stopped => by_state.stopped += 1,
                ClusterState::Failed => by_state.failed += 1,
                ClusterState::Promoting => by_state.promoting += 1,
                ClusterState::Restarting => by_state.restarting += 1,
                ClusterState::Deleting => by_state.deleting += 1,
            }
        }
        let max_lag_bytes = self.replicas.iter().map(|r| r.lag_bytes).max().unwrap_or(0);
        let max_lag_seconds = self
            .replicas
            .iter()
            .map(|r| r.lag_seconds)
            .fold(0.0_f64, f64::max);
        let healthy_replicas = self
            .replicas
            .iter()
            .filter(|r| r.sync_state == ReplicaSyncState::Streaming)
            .count();
        DashboardPanel {
            cluster_total: self.clusters.len(),
            by_state,
            replica_total: self.replicas.len(),
            healthy_replicas,
            max_lag_bytes,
            max_lag_seconds,
            failover_count_24h: self.failover_count_within(60 * 60 * 24),
        }
    }

    pub fn list_clusters(&self, persona: ViewPersona, tenant: &str) -> Vec<&Cluster> {
        self.clusters
            .iter()
            .filter(|c| persona == ViewPersona::Admin || c.tenant == tenant)
            .collect()
    }

    pub fn list_replicas(&self, cluster_id: &str) -> Vec<&ReplicaInfo> {
        self.replicas
            .iter()
            .filter(|r| r.primary_id == cluster_id || r.instance_id == cluster_id)
            .collect()
    }

    pub fn list_backups(&self, cluster_id: &str) -> Vec<&BackupRecord> {
        self.backups
            .iter()
            .filter(|b| b.cluster_id == cluster_id)
            .collect()
    }

    pub fn failover_history(&self, cluster_id: Option<&str>) -> Vec<&FailoverEvent> {
        self.failovers
            .iter()
            .filter(|e| cluster_id.map_or(true, |id| e.cluster_id == id))
            .collect()
    }

    /// Trigger a logical failover. Tenants cannot promote; admins/operators can.
    pub fn trigger_failover(
        &mut self,
        cluster_id: &str,
        target_replica: &str,
        persona: ViewPersona,
        actor: &str,
        reason: &str,
    ) -> Result<FailoverEvent, RdbmsOperatorError> {
        if persona == ViewPersona::Tenant {
            return Err(RdbmsOperatorError::Forbidden(persona.label()));
        }
        let cluster = self
            .clusters
            .iter_mut()
            .find(|c| c.id == cluster_id)
            .ok_or_else(|| RdbmsOperatorError::ClusterNotFound(cluster_id.into()))?;
        if cluster.state == ClusterState::Promoting {
            return Err(RdbmsOperatorError::FailoverBlocked(
                "cluster is already promoting".into(),
            ));
        }
        let replica = self
            .replicas
            .iter()
            .find(|r| r.instance_id == target_replica && r.primary_id == cluster_id)
            .ok_or_else(|| {
                RdbmsOperatorError::FailoverBlocked(format!(
                    "replica {target_replica} not registered for cluster {cluster_id}"
                ))
            })?;
        if replica.sync_state == ReplicaSyncState::Disconnected {
            return Err(RdbmsOperatorError::FailoverBlocked(
                "target replica is disconnected".into(),
            ));
        }
        let event = FailoverEvent {
            id: format!("fo-{}", self.failovers.len() + 1),
            cluster_id: cluster_id.to_string(),
            old_primary_id: cluster.primary_id.clone(),
            new_primary_id: target_replica.to_string(),
            reason: format!("{}: {}", actor, reason),
            timestamp: Utc::now(),
            duration_ms: 0,
            automatic: false,
        };
        cluster.primary_id = target_replica.to_string();
        cluster.state = ClusterState::Promoting;
        self.failovers.push(event.clone());
        Ok(event)
    }

    /// Restore a backup to a logical point-in-time. Tenants are blocked.
    pub fn restore_backup(
        &self,
        backup_id: &str,
        target_time: DateTime<Utc>,
        persona: ViewPersona,
    ) -> Result<RestorePlan, RdbmsOperatorError> {
        if persona == ViewPersona::Tenant {
            return Err(RdbmsOperatorError::Forbidden(persona.label()));
        }
        let base = self
            .backups
            .iter()
            .find(|b| b.id == backup_id)
            .ok_or_else(|| RdbmsOperatorError::BackupNotFound(backup_id.into()))?;
        if base.status != BackupStatus::Completed {
            return Err(RdbmsOperatorError::FailoverBlocked(
                "base backup is not completed".into(),
            ));
        }
        if target_time < base.started_at {
            return Err(RdbmsOperatorError::FailoverBlocked(
                "target_time precedes the base backup".into(),
            ));
        }
        Ok(RestorePlan {
            backup_id: backup_id.to_string(),
            cluster_id: base.cluster_id.clone(),
            target_time,
            replays_wal: base.wal_end_lsn.is_some(),
        })
    }

    fn failover_count_within(&self, secs: i64) -> usize {
        let now = Utc::now();
        self.failovers
            .iter()
            .filter(|e| (now - e.timestamp).num_seconds() <= secs)
            .count()
    }
}

// ── View-model panels ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StateCounts {
    pub creating: u32,
    pub running: u32,
    pub stopped: u32,
    pub failed: u32,
    pub promoting: u32,
    pub restarting: u32,
    pub deleting: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardPanel {
    pub cluster_total: usize,
    pub by_state: StateCounts,
    pub replica_total: usize,
    pub healthy_replicas: usize,
    pub max_lag_bytes: i64,
    pub max_lag_seconds: f64,
    pub failover_count_24h: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePlan {
    pub backup_id: String,
    pub cluster_id: String,
    pub target_time: DateTime<Utc>,
    /// `true` when the plan needs to replay WAL beyond the base backup.
    pub replays_wal: bool,
}

// ── Validation ───────────────────────────────────────────────────────────────

fn validate_name(name: &str) -> Result<(), RdbmsOperatorError> {
    if name.is_empty() {
        return Err(RdbmsOperatorError::InvalidSpec("empty name".into()));
    }
    if name.len() > 63 {
        return Err(RdbmsOperatorError::InvalidSpec("name too long".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(RdbmsOperatorError::InvalidSpec(format!(
            "invalid char in name: {name:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample_cluster(name: &str, tenant: &str) -> Cluster {
        Cluster {
            id: format!("cl-{name}"),
            name: name.into(),
            tenant: tenant.into(),
            version: "16.2".into(),
            instances: 3,
            state: ClusterState::Running,
            primary_id: format!("inst-{name}-0"),
            storage_gib: 100,
            created_at: Utc::now(),
        }
    }

    fn sample_replica(cluster_id: &str, idx: u32, lag_bytes: i64) -> ReplicaInfo {
        ReplicaInfo {
            instance_id: format!("inst-{cluster_id}-{idx}"),
            primary_id: cluster_id.into(),
            sync_state: ReplicaSyncState::Streaming,
            lag_bytes,
            lag_seconds: 0.5,
            is_primary: idx == 0,
        }
    }

    #[test]
    fn register_cluster_persists_and_dedups() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("prod", "acme")).unwrap();
        let dup = p.register_cluster(sample_cluster("prod", "acme"));
        assert!(matches!(dup, Err(RdbmsOperatorError::ClusterExists(_))));
        assert_eq!(p.list_clusters(ViewPersona::Admin, "any").len(), 1);
    }

    #[test]
    fn register_cluster_validates_name_and_spec() {
        let mut p = RdbmsOperatorPlugin::new();
        let mut bad = sample_cluster("", "acme");
        assert!(matches!(
            p.register_cluster(bad.clone()),
            Err(RdbmsOperatorError::InvalidSpec(_))
        ));
        bad.name = "x".repeat(70);
        assert!(matches!(
            p.register_cluster(bad.clone()),
            Err(RdbmsOperatorError::InvalidSpec(_))
        ));
        bad.name = "ok!".into();
        assert!(matches!(
            p.register_cluster(bad.clone()),
            Err(RdbmsOperatorError::InvalidSpec(_))
        ));
        bad.name = "ok".into();
        bad.instances = 0;
        assert!(matches!(
            p.register_cluster(bad.clone()),
            Err(RdbmsOperatorError::InvalidSpec(_))
        ));
        bad.instances = 1;
        bad.storage_gib = 0;
        assert!(matches!(
            p.register_cluster(bad),
            Err(RdbmsOperatorError::InvalidSpec(_))
        ));
    }

    #[test]
    fn list_clusters_scopes_to_tenant_for_non_admin() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("a", "acme")).unwrap();
        p.register_cluster(sample_cluster("b", "globex")).unwrap();
        assert_eq!(p.list_clusters(ViewPersona::Admin, "anything").len(), 2);
        assert_eq!(p.list_clusters(ViewPersona::Tenant, "acme").len(), 1);
        assert_eq!(p.list_clusters(ViewPersona::Operator, "globex").len(), 1);
    }

    #[test]
    fn dashboard_aggregates_states_and_lag() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("a", "acme")).unwrap();
        let mut b = sample_cluster("b", "acme");
        b.state = ClusterState::Failed;
        p.register_cluster(b).unwrap();
        p.register_replica(sample_replica("cl-a", 1, 10));
        p.register_replica(sample_replica("cl-a", 2, 200_000));
        let panel = p.dashboard();
        assert_eq!(panel.cluster_total, 2);
        assert_eq!(panel.by_state.running, 1);
        assert_eq!(panel.by_state.failed, 1);
        assert_eq!(panel.replica_total, 2);
        assert_eq!(panel.healthy_replicas, 2);
        assert_eq!(panel.max_lag_bytes, 200_000);
    }

    #[test]
    fn trigger_failover_promotes_target_and_records_event() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("a", "acme")).unwrap();
        p.register_replica(sample_replica("cl-a", 1, 10));
        let event = p
            .trigger_failover("cl-a", "inst-cl-a-1", ViewPersona::Admin, "alice", "test")
            .unwrap();
        assert_eq!(event.new_primary_id, "inst-cl-a-1");
        assert!(!event.automatic);
        assert_eq!(p.failover_history(Some("cl-a")).len(), 1);
        let cluster = p.list_clusters(ViewPersona::Admin, "any")[0];
        assert_eq!(cluster.primary_id, "inst-cl-a-1");
        assert_eq!(cluster.state, ClusterState::Promoting);
    }

    #[test]
    fn trigger_failover_rejects_tenant_persona() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("a", "acme")).unwrap();
        p.register_replica(sample_replica("cl-a", 1, 10));
        let err = p
            .trigger_failover("cl-a", "inst-cl-a-1", ViewPersona::Tenant, "bob", "go")
            .unwrap_err();
        assert!(matches!(err, RdbmsOperatorError::Forbidden(_)));
    }

    #[test]
    fn trigger_failover_rejects_disconnected_replica() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("a", "acme")).unwrap();
        let mut r = sample_replica("cl-a", 1, 10);
        r.sync_state = ReplicaSyncState::Disconnected;
        p.register_replica(r);
        let err = p
            .trigger_failover("cl-a", "inst-cl-a-1", ViewPersona::Admin, "x", "y")
            .unwrap_err();
        assert!(matches!(err, RdbmsOperatorError::FailoverBlocked(_)));
    }

    #[test]
    fn restore_backup_validates_target_time_and_persona() {
        let mut p = RdbmsOperatorPlugin::new();
        p.register_cluster(sample_cluster("a", "acme")).unwrap();
        let started = Utc::now();
        p.register_backup(BackupRecord {
            id: "bk1".into(),
            cluster_id: "cl-a".into(),
            kind: BackupKind::Full,
            status: BackupStatus::Completed,
            size_bytes: 1024,
            started_at: started,
            completed_at: Some(started + Duration::seconds(60)),
            wal_start_lsn: Some("0/1".into()),
            wal_end_lsn: Some("0/9".into()),
        });
        // Tenant blocked.
        let err = p
            .restore_backup("bk1", started + Duration::seconds(120), ViewPersona::Tenant)
            .unwrap_err();
        assert!(matches!(err, RdbmsOperatorError::Forbidden(_)));
        // Past base time blocked.
        let err = p
            .restore_backup("bk1", started - Duration::seconds(1), ViewPersona::Admin)
            .unwrap_err();
        assert!(matches!(err, RdbmsOperatorError::FailoverBlocked(_)));
        // Happy path.
        let plan = p
            .restore_backup("bk1", started + Duration::seconds(120), ViewPersona::Admin)
            .unwrap();
        assert_eq!(plan.cluster_id, "cl-a");
        assert!(plan.replays_wal);
    }

    #[test]
    fn list_backups_scopes_to_cluster() {
        let mut p = RdbmsOperatorPlugin::new();
        for cluster in ["cl-a", "cl-b"] {
            p.register_backup(BackupRecord {
                id: format!("bk-{cluster}"),
                cluster_id: cluster.into(),
                kind: BackupKind::Full,
                status: BackupStatus::Completed,
                size_bytes: 1024,
                started_at: Utc::now(),
                completed_at: None,
                wal_start_lsn: None,
                wal_end_lsn: None,
            });
        }
        assert_eq!(p.list_backups("cl-a").len(), 1);
        assert_eq!(p.list_backups("cl-c").len(), 0);
    }

    #[test]
    fn failover_history_filters_by_cluster() {
        let mut p = RdbmsOperatorPlugin::new();
        p.record_failover(FailoverEvent {
            id: "1".into(),
            cluster_id: "a".into(),
            old_primary_id: "x".into(),
            new_primary_id: "y".into(),
            reason: "alice: ok".into(),
            timestamp: Utc::now(),
            duration_ms: 0,
            automatic: false,
        });
        p.record_failover(FailoverEvent {
            id: "2".into(),
            cluster_id: "b".into(),
            old_primary_id: "x".into(),
            new_primary_id: "y".into(),
            reason: "bob: ok".into(),
            timestamp: Utc::now(),
            duration_ms: 0,
            automatic: true,
        });
        assert_eq!(p.failover_history(Some("a")).len(), 1);
        assert_eq!(p.failover_history(None).len(), 2);
    }
}
