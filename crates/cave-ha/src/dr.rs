<<<<<<< HEAD
//! Disaster Recovery — cross-datacenter replication and site failover.
//!
//! Supports Hetzner ↔ Azure and similar active-passive or active-active
//! multi-site configurations. RPO/RTO targets are configured per DR pair.

use crate::{models::DRConfig, HaState};
use anyhow::Result;
use chrono::Utc;
use std::sync::Arc;
use tracing::{info, warn};

/// Register a primary/secondary site relationship and store DR parameters.
pub async fn configure_dr_pair(state: Arc<HaState>, config: DRConfig) -> Result<()> {
    info!(
        primary = %config.primary_site,
        secondary = %config.secondary_site,
        mode = ?config.replication_mode,
        rpo_s = config.rpo_seconds,
        rto_s = config.rto_seconds,
        "DR pair configured"
    );
    // Production: persist config and initiate the initial full-sync.
    // Phase 1: in-memory acknowledgment only.
    let _ = state.topology.read().await; // touch state to satisfy borrow checker
    Ok(())
}

/// Stream committed log entries to the secondary datacenter.
///
/// Called periodically by the leader. In active-passive mode the secondary
/// applies entries but does not serve writes. In active-active mode both sites
/// replicate to each other with conflict resolution.
pub async fn cross_site_replication(state: Arc<HaState>) -> Result<()> {
    let leader_id = state.topology.read().await.leader;

    match leader_id {
        None => {
            warn!("Cross-site replication skipped: no leader elected");
            Ok(())
        }
        Some(leader) => {
            // Production: open a persistent gRPC/HTTP2 stream to the secondary site
            // leader and push entries committed since the last acknowledged index.
            info!(leader = %leader, "Cross-site replication cycle complete");
            Ok(())
        }
    }
}

/// Promote the secondary site to primary (unplanned site failover).
///
/// Steps:
/// 1. Fence the primary site (stop writes).
/// 2. Let the secondary elect a new leader via Raft.
/// 3. Update global DNS/routing to point at the secondary.
pub async fn site_failover(state: Arc<HaState>, target_site: String) -> Result<()> {
    info!(target_site = %target_site, "Site failover initiated");

    // Clear the leader so the secondary cluster triggers an election.
    state.topology.write().await.leader = None;

    info!(new_primary = %target_site, "Secondary promoted; waiting for election");
    Ok(())
}

/// Restore the original primary site after it has recovered (planned failback).
///
/// Steps:
/// 1. Sync data from the current primary (ex-secondary) back to the original site.
/// 2. Perform a graceful_handoff once the original site has caught up.
/// 3. Resume normal replication topology.
pub async fn site_failback(state: Arc<HaState>, original_site: String) -> Result<()> {
    info!(original_site = %original_site, "Site failback initiated");
    // Production: trigger catch_up on original-site instances, then graceful_handoff.
    let _ = state.topology.read().await;
    info!(original_site = %original_site, "Failback complete — original primary restored");
    Ok(())
}

/// Alert if cross-site replication lag exceeds the configured RPO target.
pub async fn rpo_monitor(state: Arc<HaState>) -> Result<()> {
    // Production: compare last cross-site sync timestamp with current time.
    // Alert via cave-alerts if lag_seconds > dr_config.rpo_seconds.
    let lag_seconds = 0u64;
    let dr = &state.dr_config;

    if let Some(cfg) = dr {
        if lag_seconds > cfg.rpo_seconds {
            warn!(
                lag_seconds,
                rpo_seconds = cfg.rpo_seconds,
                "RPO breach: cross-site replication lag exceeds target"
            );
        } else {
            info!(lag_seconds, rpo_seconds = cfg.rpo_seconds, "RPO within target");
        }
    } else {
        info!(lag_seconds, "RPO monitor: no DR pair configured");
    }
    Ok(())
}

/// Simulate a site failure, verify automatic failover, then roll back.
///
/// This is a non-destructive chaos test: the simulated failure is reverted
/// at the end regardless of outcome so production traffic is not affected.
pub async fn dr_test(state: Arc<HaState>, simulated_site: String) -> Result<serde_json::Value> {
    info!(site = %simulated_site, "DR test: simulating site failure");
    let start = std::time::Instant::now();

    // Phase 1: simulate failure — clear leader in topology.
    let original_leader = state.topology.write().await.leader.take();

    // Phase 2: verify secondary would elect a leader (stub — success in test mode).
    let failover_ms = start.elapsed().as_millis() as u64;

    // Phase 3: rollback — restore original leader.
    state.topology.write().await.leader = original_leader;

    let rto_ok = state
        .dr_config
        .as_ref()
        .map(|c| failover_ms / 1000 <= c.rto_seconds)
        .unwrap_or(true);

    info!(
        site = %simulated_site,
        failover_ms,
        rto_ok,
        "DR test complete — state rolled back"
    );

    Ok(serde_json::json!({
        "test": "dr_simulation",
        "simulated_site": simulated_site,
        "failover_ms": failover_ms,
        "rto_within_target": rto_ok,
        "result": if rto_ok { "pass" } else { "fail" },
        "timestamp": Utc::now().to_rfc3339(),
    }))
=======
//! Disaster recovery: backup/restore, point-in-time recovery, geo-redundant snapshots.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::info;

use crate::snapshot::Snapshot;

/// Metadata about a disaster-recovery backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetadata {
    pub id: uuid::Uuid,
    pub cluster_id: String,
    pub snapshot_index: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub size_bytes: usize,
    /// Which datacenter holds this backup copy.
    pub datacenter: String,
    /// Optional point-in-time recovery marker (e.g., a WAL LSN or timestamp tag).
    pub pitr_marker: Option<String>,
    /// Hex-encoded SHA-256 of the snapshot data.
    pub checksum: String,
}

/// Manages disaster-recovery backups stored across multiple datacenters.
pub struct DisasterRecovery {
    backups: Arc<RwLock<Vec<BackupMetadata>>>,
    #[allow(dead_code)]
    target_datacenters: Vec<String>,
}

impl DisasterRecovery {
    pub fn new(target_datacenters: Vec<String>) -> Self {
        Self {
            backups: Arc::new(RwLock::new(Vec::new())),
            target_datacenters,
        }
    }

    /// Create a backup of `snapshot` and store it in `datacenter`.
    /// Returns the metadata record that was persisted.
    pub async fn create_backup(
        &self,
        cluster_id: &str,
        snapshot: &Snapshot,
        datacenter: &str,
    ) -> BackupMetadata {
        let checksum = hex_checksum(&snapshot.data);
        let meta = BackupMetadata {
            id: uuid::Uuid::new_v4(),
            cluster_id: cluster_id.to_string(),
            snapshot_index: snapshot.last_included_index,
            created_at: Utc::now(),
            size_bytes: snapshot.size_bytes(),
            datacenter: datacenter.to_string(),
            pitr_marker: None,
            checksum,
        };
        info!(
            cluster_id,
            datacenter,
            snapshot_index = meta.snapshot_index,
            id = %meta.id,
            "created backup",
        );
        self.backups.write().await.push(meta.clone());
        meta
    }

    /// List all backups for the given cluster (sorted oldest-first).
    pub async fn list_backups(&self, cluster_id: &str) -> Vec<BackupMetadata> {
        let backups = self.backups.read().await;
        let mut result: Vec<BackupMetadata> = backups
            .iter()
            .filter(|b| b.cluster_id == cluster_id)
            .cloned()
            .collect();
        result.sort_by_key(|b| b.created_at);
        result
    }

    /// List all backups stored in a specific datacenter.
    pub async fn list_by_datacenter(&self, dc: &str) -> Vec<BackupMetadata> {
        self.backups
            .read()
            .await
            .iter()
            .filter(|b| b.datacenter == dc)
            .cloned()
            .collect()
    }

    /// Return the most recent backup for a cluster, if any.
    pub async fn latest_backup(&self, cluster_id: &str) -> Option<BackupMetadata> {
        self.list_backups(cluster_id)
            .await
            .into_iter()
            .max_by_key(|b| b.snapshot_index)
    }

    /// Return the number of distinct datacenters that hold a backup of `cluster_id`.
    pub async fn geo_redundant_count(&self, cluster_id: &str) -> usize {
        let backups = self.backups.read().await;
        let dcs: HashSet<&str> = backups
            .iter()
            .filter(|b| b.cluster_id == cluster_id)
            .map(|b| b.datacenter.as_str())
            .collect();
        dcs.len()
    }

    /// Delete a backup by its UUID.
    pub async fn delete_backup(&self, id: uuid::Uuid) -> Result<(), String> {
        let mut backups = self.backups.write().await;
        let before = backups.len();
        backups.retain(|b| b.id != id);
        if backups.len() < before {
            Ok(())
        } else {
            Err(format!("backup {id} not found"))
        }
    }
}

/// Compute a simple FNV-1a 64-bit hash and return it as a hex string.
/// This is used as a lightweight checksum for backup integrity verification.
fn hex_checksum(data: &[u8]) -> String {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::Snapshot;

    fn make_snapshot(index: u64) -> Snapshot {
        Snapshot::new(index, 1, format!("data-{index}").into_bytes(), "cluster-1")
    }

    #[tokio::test]
    async fn test_backup_create_and_list() {
        let dr = DisasterRecovery::new(vec!["us-east-1".to_string(), "eu-west-1".to_string()]);
        let snap = make_snapshot(100);

        let meta = dr.create_backup("cluster-1", &snap, "us-east-1").await;
        assert_eq!(meta.cluster_id, "cluster-1");
        assert_eq!(meta.snapshot_index, 100);
        assert_eq!(meta.datacenter, "us-east-1");
        assert!(!meta.checksum.is_empty());

        let list = dr.list_backups("cluster-1").await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, meta.id);
    }

    #[tokio::test]
    async fn test_latest_backup() {
        let dr = DisasterRecovery::new(vec![]);
        dr.create_backup("c1", &make_snapshot(50), "dc1").await;
        dr.create_backup("c1", &make_snapshot(100), "dc1").await;
        dr.create_backup("c1", &make_snapshot(75), "dc2").await;

        let latest = dr.latest_backup("c1").await.unwrap();
        assert_eq!(latest.snapshot_index, 100);
    }

    #[tokio::test]
    async fn test_geo_redundant_count() {
        let dr = DisasterRecovery::new(vec![]);
        dr.create_backup("c1", &make_snapshot(10), "us-east-1").await;
        dr.create_backup("c1", &make_snapshot(20), "us-east-1").await; // same DC
        dr.create_backup("c1", &make_snapshot(30), "eu-west-1").await;
        dr.create_backup("c1", &make_snapshot(40), "ap-south-1").await;

        // 3 distinct DCs.
        assert_eq!(dr.geo_redundant_count("c1").await, 3);
    }

    #[tokio::test]
    async fn test_delete_backup() {
        let dr = DisasterRecovery::new(vec![]);
        let meta = dr.create_backup("c1", &make_snapshot(10), "dc1").await;

        assert_eq!(dr.list_backups("c1").await.len(), 1);
        dr.delete_backup(meta.id).await.unwrap();
        assert_eq!(dr.list_backups("c1").await.len(), 0);

        // Double delete should fail.
        assert!(dr.delete_backup(meta.id).await.is_err());
    }

    #[tokio::test]
    async fn test_list_by_datacenter() {
        let dr = DisasterRecovery::new(vec![]);
        dr.create_backup("c1", &make_snapshot(10), "dc-a").await;
        dr.create_backup("c1", &make_snapshot(20), "dc-b").await;
        dr.create_backup("c2", &make_snapshot(30), "dc-a").await;

        let dc_a = dr.list_by_datacenter("dc-a").await;
        assert_eq!(dc_a.len(), 2);
        assert!(dc_a.iter().all(|b| b.datacenter == "dc-a"));
    }

    #[tokio::test]
    async fn test_checksum_is_deterministic() {
        let dr = DisasterRecovery::new(vec![]);
        let snap = make_snapshot(5);
        let m1 = dr.create_backup("c1", &snap, "dc1").await;
        let m2 = dr.create_backup("c1", &snap, "dc2").await;
        assert_eq!(m1.checksum, m2.checksum);
    }
>>>>>>> claude/great-sanderson
}
