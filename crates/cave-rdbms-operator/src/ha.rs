// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::{PgError, PgResult};
use crate::types::{ReplicaInfo, ReplicationState};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FailoverEvent {
    pub id: String,
    pub primary_id: String,
    pub new_primary_id: String,
    pub reason: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    pub automatic: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplicationHealth {
    pub primary_id: String,
    pub replica_count: usize,
    pub healthy_replicas: usize,
    pub max_lag_bytes: i64,
    pub max_lag_seconds: f64,
    pub is_healthy: bool,
}

pub struct HaController {
    replicas: Arc<RwLock<HashMap<String, ReplicaInfo>>>,
    failover_history: Arc<RwLock<Vec<FailoverEvent>>>,
}

impl Default for HaController {
    fn default() -> Self {
        Self::new()
    }
}

impl HaController {
    pub fn new() -> Self {
        HaController {
            replicas: Arc::new(RwLock::new(HashMap::new())),
            failover_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn register_replica(&self, replica: ReplicaInfo) -> PgResult<()> {
        let mut replicas = self.replicas.write().unwrap();
        replicas.insert(replica.instance_id.clone(), replica);
        Ok(())
    }

    pub fn update_replica_lag(
        &self,
        instance_id: &str,
        lag_bytes: i64,
        lag_seconds: f64,
    ) -> PgResult<()> {
        let mut replicas = self.replicas.write().unwrap();
        let replica = replicas.get_mut(instance_id).ok_or_else(|| {
            PgError::ReplicationError(format!("replica not found: {}", instance_id))
        })?;
        replica.lag_bytes = lag_bytes;
        replica.lag_seconds = lag_seconds;
        Ok(())
    }

    pub fn get_replica(&self, instance_id: &str) -> PgResult<ReplicaInfo> {
        let replicas = self.replicas.read().unwrap();
        replicas
            .get(instance_id)
            .cloned()
            .ok_or_else(|| PgError::ReplicationError(format!("replica not found: {}", instance_id)))
    }

    pub fn list_replicas(&self, primary_id: &str) -> Vec<ReplicaInfo> {
        let replicas = self.replicas.read().unwrap();
        replicas
            .values()
            .filter(|r| r.primary_id == primary_id)
            .cloned()
            .collect()
    }

    /// Trigger failover: promote the replica with the least lag.
    pub fn trigger_failover(&self, primary_id: &str, reason: &str) -> PgResult<FailoverEvent> {
        let start = std::time::Instant::now();
        let replicas = self.replicas.read().unwrap();

        let best_replica = replicas
            .values()
            .filter(|r| r.primary_id == primary_id && r.state == ReplicationState::Streaming)
            .min_by_key(|r| r.lag_bytes);

        let new_primary_id = match best_replica {
            Some(r) => r.instance_id.clone(),
            None => {
                // Fall back to any replica
                let any = replicas
                    .values()
                    .find(|r| r.primary_id == primary_id)
                    .ok_or_else(|| {
                        PgError::ReplicationError(format!(
                            "no replicas available for failover from {}",
                            primary_id
                        ))
                    })?;
                any.instance_id.clone()
            }
        };
        drop(replicas);

        let duration_ms = start.elapsed().as_millis() as u64;
        let event = FailoverEvent {
            id: Uuid::new_v4().to_string(),
            primary_id: primary_id.to_string(),
            new_primary_id: new_primary_id,
            reason: reason.to_string(),
            timestamp: Utc::now(),
            duration_ms,
            automatic: false,
        };

        let mut history = self.failover_history.write().unwrap();
        history.push(event.clone());

        Ok(event)
    }

    /// Determine if pg_rewind is needed for a returning failed primary.
    pub fn pg_rewind_needed(&self, _instance_id: &str, _new_primary_id: &str) -> bool {
        // In a real implementation, compare WAL timelines.
        // For in-memory tests, always return true (conservative).
        true
    }

    pub fn check_replication_health(&self, primary_id: &str) -> ReplicationHealth {
        let replicas = self.replicas.read().unwrap();
        let replica_list: Vec<&ReplicaInfo> = replicas
            .values()
            .filter(|r| r.primary_id == primary_id)
            .collect();

        let replica_count = replica_list.len();
        let healthy_replicas = replica_list
            .iter()
            .filter(|r| r.state == ReplicationState::Streaming)
            .count();

        let max_lag_bytes = replica_list.iter().map(|r| r.lag_bytes).max().unwrap_or(0);
        let max_lag_seconds = replica_list
            .iter()
            .map(|r| r.lag_seconds)
            .fold(0.0f64, f64::max);

        let is_healthy = healthy_replicas > 0 && max_lag_bytes < 100 * 1024 * 1024; // 100MB threshold

        ReplicationHealth {
            primary_id: primary_id.to_string(),
            replica_count,
            healthy_replicas,
            max_lag_bytes,
            max_lag_seconds,
            is_healthy,
        }
    }

    pub fn failover_history(&self) -> Vec<FailoverEvent> {
        let history = self.failover_history.read().unwrap();
        history.clone()
    }
}
