// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Strategy & Status ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradeStrategy {
    RollingUpdate,
    InPlace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradeStatus {
    Pending,
    InProgress,
    Paused,
    Completed,
    Failed(String),
}

// ── Plan ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradePlan {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub from_version: String,
    pub to_version: String,
    pub strategy: UpgradeStrategy,
    pub status: UpgradeStatus,
    /// Maximum number of nodes being upgraded simultaneously.
    pub max_unavailable: usize,
    pub nodes_pending: Vec<Uuid>,
    pub nodes_upgrading: Vec<Uuid>,
    pub nodes_completed: Vec<Uuid>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl UpgradePlan {
    pub fn new(cluster_id: Uuid, from: &str, to: &str, node_ids: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            cluster_id,
            from_version: from.to_string(),
            to_version: to.to_string(),
            strategy: UpgradeStrategy::RollingUpdate,
            status: UpgradeStatus::Pending,
            max_unavailable: 1,
            nodes_pending: node_ids,
            nodes_upgrading: Vec::new(),
            nodes_completed: Vec::new(),
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        }
    }

    /// Percentage of nodes that have finished upgrading.
    pub fn progress_percent(&self) -> f64 {
        let total =
            self.nodes_pending.len() + self.nodes_upgrading.len() + self.nodes_completed.len();
        if total == 0 {
            return 100.0;
        }
        self.nodes_completed.len() as f64 / total as f64 * 100.0
    }

    pub fn is_complete(&self) -> bool {
        self.nodes_pending.is_empty() && self.nodes_upgrading.is_empty()
    }

    /// Returns the next batch of nodes to begin upgrading, limited to `max_unavailable`.
    pub fn next_batch(&self) -> Vec<Uuid> {
        let slots = self.max_unavailable.saturating_sub(self.nodes_upgrading.len());
        self.nodes_pending.iter().take(slots).copied().collect()
    }
}

// ── Manager ───────────────────────────────────────────────────────────────────

pub struct UpgradeManager {
    plans: Arc<RwLock<HashMap<Uuid, UpgradePlan>>>,
}

impl UpgradeManager {
    pub fn new() -> Self {
        Self { plans: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// Store a new plan. Returns the plan id.
    pub async fn create_plan(&self, plan: UpgradePlan) -> Uuid {
        let id = plan.id;
        let mut guard = self.plans.write().await;
        guard.insert(id, plan);
        id
    }

    /// Transition the plan to `InProgress` and move the first batch to upgrading.
    pub async fn start(&self, plan_id: Uuid) -> Result<(), String> {
        let mut guard = self.plans.write().await;
        let plan =
            guard.get_mut(&plan_id).ok_or_else(|| format!("plan {plan_id} not found"))?;

        if plan.status != UpgradeStatus::Pending && plan.status != UpgradeStatus::Paused {
            return Err(format!("plan is not in Pending/Paused state: {:?}", plan.status));
        }

        plan.status = UpgradeStatus::InProgress;
        if plan.started_at.is_none() {
            plan.started_at = Some(Utc::now());
        }

        // Kick off the first batch.
        let batch = plan.next_batch();
        for node_id in batch {
            plan.nodes_pending.retain(|id| *id != node_id);
            if !plan.nodes_upgrading.contains(&node_id) {
                plan.nodes_upgrading.push(node_id);
            }
        }

        tracing::info!(plan_id = %plan_id, "upgrade plan started");
        Ok(())
    }

    /// Mark a node as being actively drained (moves it into `nodes_upgrading`).
    pub async fn drain_node(&self, plan_id: Uuid, node_id: Uuid) -> Result<(), String> {
        let mut guard = self.plans.write().await;
        let plan =
            guard.get_mut(&plan_id).ok_or_else(|| format!("plan {plan_id} not found"))?;

        // Move node from pending → upgrading if not already there.
        plan.nodes_pending.retain(|id| *id != node_id);
        if !plan.nodes_upgrading.contains(&node_id) {
            plan.nodes_upgrading.push(node_id);
        }
        Ok(())
    }

    /// Mark a node upgrade as completed and advance the batch.
    pub async fn complete_node(&self, plan_id: Uuid, node_id: Uuid) -> Result<(), String> {
        let mut guard = self.plans.write().await;
        let plan =
            guard.get_mut(&plan_id).ok_or_else(|| format!("plan {plan_id} not found"))?;

        plan.nodes_upgrading.retain(|id| *id != node_id);
        if !plan.nodes_completed.contains(&node_id) {
            plan.nodes_completed.push(node_id);
        }

        // Advance: pull next node into upgrading.
        let next_batch = plan.next_batch();
        for nid in next_batch {
            plan.nodes_pending.retain(|id| *id != nid);
            plan.nodes_upgrading.push(nid);
        }

        // Check if we're done.
        if plan.is_complete() {
            plan.status = UpgradeStatus::Completed;
            plan.completed_at = Some(Utc::now());
        }

        Ok(())
    }

    pub async fn pause(&self, plan_id: Uuid) -> Result<(), String> {
        let mut guard = self.plans.write().await;
        let plan =
            guard.get_mut(&plan_id).ok_or_else(|| format!("plan {plan_id} not found"))?;
        plan.status = UpgradeStatus::Paused;
        Ok(())
    }

    pub async fn resume(&self, plan_id: Uuid) -> Result<(), String> {
        // Delegate to start which handles Paused → InProgress.
        self.start(plan_id).await
    }

    pub async fn get_plan(&self, plan_id: Uuid) -> Option<UpgradePlan> {
        let guard = self.plans.read().await;
        guard.get(&plan_id).cloned()
    }
}

impl Default for UpgradeManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn three_node_plan() -> UpgradePlan {
        let nodes: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        UpgradePlan::new(Uuid::new_v4(), "v1.28.0", "v1.29.0", nodes)
    }

    #[tokio::test]
    async fn test_create_plan() {
        let mgr = UpgradeManager::new();
        let plan = three_node_plan();
        let plan_id = mgr.create_plan(plan).await;
        let stored = mgr.get_plan(plan_id).await.unwrap();
        assert_eq!(stored.status, UpgradeStatus::Pending);
        assert_eq!(stored.nodes_pending.len(), 3);
    }

    #[tokio::test]
    async fn test_start_moves_to_in_progress() {
        let mgr = UpgradeManager::new();
        let plan = three_node_plan();
        let plan_id = mgr.create_plan(plan).await;
        mgr.start(plan_id).await.unwrap();
        let stored = mgr.get_plan(plan_id).await.unwrap();
        assert_eq!(stored.status, UpgradeStatus::InProgress);
        // max_unavailable=1, so 1 node in upgrading, 2 still pending.
        assert_eq!(stored.nodes_upgrading.len(), 1);
        assert_eq!(stored.nodes_pending.len(), 2);
    }

    #[tokio::test]
    async fn test_drain_node() {
        let mgr = UpgradeManager::new();
        let plan = three_node_plan();
        let target = plan.nodes_pending[0];
        let plan_id = mgr.create_plan(plan).await;
        mgr.drain_node(plan_id, target).await.unwrap();
        let stored = mgr.get_plan(plan_id).await.unwrap();
        assert!(stored.nodes_upgrading.contains(&target));
        assert!(!stored.nodes_pending.contains(&target));
    }

    #[tokio::test]
    async fn test_complete_node_and_progress_percent() {
        let mgr = UpgradeManager::new();
        let plan = three_node_plan();
        let plan_id = mgr.create_plan(plan).await;
        mgr.start(plan_id).await.unwrap();

        let upgrading = mgr.get_plan(plan_id).await.unwrap().nodes_upgrading[0];
        mgr.complete_node(plan_id, upgrading).await.unwrap();

        let stored = mgr.get_plan(plan_id).await.unwrap();
        assert_eq!(stored.nodes_completed.len(), 1);
        // progress = 1/3 ≈ 33.3%
        assert!(stored.progress_percent() > 30.0);
    }

    #[tokio::test]
    async fn test_upgrade_completes_when_all_nodes_done() {
        let mgr = UpgradeManager::new();
        let nodes: Vec<Uuid> = (0..2).map(|_| Uuid::new_v4()).collect();
        let plan = UpgradePlan::new(Uuid::new_v4(), "v1.28.0", "v1.29.0", nodes);
        let plan_id = mgr.create_plan(plan).await;
        mgr.start(plan_id).await.unwrap();

        loop {
            let stored = mgr.get_plan(plan_id).await.unwrap();
            if stored.nodes_upgrading.is_empty() {
                break;
            }
            let node = stored.nodes_upgrading[0];
            mgr.complete_node(plan_id, node).await.unwrap();
        }

        let final_plan = mgr.get_plan(plan_id).await.unwrap();
        assert_eq!(final_plan.status, UpgradeStatus::Completed);
        assert!(final_plan.is_complete());
        assert_eq!(final_plan.progress_percent(), 100.0);
    }

    #[tokio::test]
    async fn test_next_batch_respects_max_unavailable() {
        let nodes: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        let mut plan = UpgradePlan::new(Uuid::new_v4(), "v1.28.0", "v1.29.0", nodes);
        plan.max_unavailable = 2;
        // Simulate 1 already upgrading.
        let already_upgrading = plan.nodes_pending.remove(0);
        plan.nodes_upgrading.push(already_upgrading);

        let batch = plan.next_batch();
        // max_unavailable=2, already 1 upgrading → only 1 more slot.
        assert_eq!(batch.len(), 1);
    }

    #[tokio::test]
    async fn test_pause_and_resume() {
        let mgr = UpgradeManager::new();
        let plan = three_node_plan();
        let plan_id = mgr.create_plan(plan).await;
        mgr.start(plan_id).await.unwrap();
        mgr.pause(plan_id).await.unwrap();

        let paused = mgr.get_plan(plan_id).await.unwrap();
        assert_eq!(paused.status, UpgradeStatus::Paused);

        mgr.resume(plan_id).await.unwrap();
        let resumed = mgr.get_plan(plan_id).await.unwrap();
        assert_eq!(resumed.status, UpgradeStatus::InProgress);
    }
}
