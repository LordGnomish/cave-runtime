//! Rollback manager — reverses applied changes when an infrastructure apply fails.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::state::InfraResource;

// ── RollbackAction ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RollbackAction {
    /// Resource was created → destroy it.
    Destroy,
    /// Resource was updated → restore its previous spec.
    RestoreSpec(serde_json::Value),
    /// Resource was destroyed → recreate it from the saved spec.
    Recreate(serde_json::Value),
}

// ── RollbackStep ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackStep {
    pub resource_id: String,
    pub action: RollbackAction,
    pub completed: bool,
    pub error: Option<String>,
}

impl RollbackStep {
    pub fn new(resource_id: &str, action: RollbackAction) -> Self {
        Self {
            resource_id: resource_id.to_string(),
            action,
            completed: false,
            error: None,
        }
    }
}

// ── RollbackStatus ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackStatus {
    Pending,
    InProgress,
    Completed,
    PartialFailure,
    Failed,
}

// ── RollbackPlan ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackPlan {
    pub id: Uuid,
    /// The apply operation that triggered this rollback.
    pub apply_id: Uuid,
    pub tenant_id: String,
    pub steps: Vec<RollbackStep>,
    pub status: RollbackStatus,
    pub triggered_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl RollbackPlan {
    pub fn new(apply_id: Uuid, tenant_id: &str, steps: Vec<RollbackStep>) -> Self {
        Self {
            id: Uuid::new_v4(),
            apply_id,
            tenant_id: tenant_id.to_string(),
            steps,
            status: RollbackStatus::Pending,
            triggered_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Steps that have not yet completed (success or failure).
    pub fn pending_steps(&self) -> Vec<&RollbackStep> {
        self.steps.iter().filter(|s| !s.completed).collect()
    }

    pub fn all_complete(&self) -> bool {
        self.steps.iter().all(|s| s.completed)
    }

    pub fn has_errors(&self) -> bool {
        self.steps.iter().any(|s| s.error.is_some())
    }
}

// ── RollbackManager ───────────────────────────────────────────────────────────

pub struct RollbackManager {
    plans: Arc<RwLock<HashMap<Uuid, RollbackPlan>>>,
}

impl RollbackManager {
    pub fn new() -> Self {
        Self {
            plans: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store a new rollback plan and return its ID.
    pub async fn create_plan(&self, plan: RollbackPlan) -> Uuid {
        let id = plan.id;
        self.plans.write().await.insert(id, plan);
        id
    }

    /// Mark the step for `resource_id` as completed (with optional error).
    pub async fn execute_step(
        &self,
        plan_id: Uuid,
        resource_id: &str,
        success: bool,
        error: Option<String>,
    ) -> Result<(), String> {
        let mut guard = self.plans.write().await;
        let plan = guard
            .get_mut(&plan_id)
            .ok_or_else(|| format!("plan {plan_id} not found"))?;

        let step = plan
            .steps
            .iter_mut()
            .find(|s| s.resource_id == resource_id)
            .ok_or_else(|| format!("step for resource {resource_id} not found"))?;

        step.completed = true;
        if !success {
            step.error = error;
        }

        // Advance the plan status.
        plan.status = RollbackStatus::InProgress;
        Ok(())
    }

    /// Mark the entire plan as finished; status reflects whether any steps failed.
    pub async fn complete_plan(&self, plan_id: Uuid) -> Result<(), String> {
        let mut guard = self.plans.write().await;
        let plan = guard
            .get_mut(&plan_id)
            .ok_or_else(|| format!("plan {plan_id} not found"))?;

        plan.completed_at = Some(Utc::now());
        plan.status = if plan.has_errors() {
            RollbackStatus::PartialFailure
        } else {
            RollbackStatus::Completed
        };
        Ok(())
    }

    pub async fn get(&self, plan_id: Uuid) -> Option<RollbackPlan> {
        self.plans.read().await.get(&plan_id).cloned()
    }

    /// Build a rollback plan from the resources that were successfully applied,
    /// in *reverse* order (last applied is rolled back first).
    pub fn build_from_applied(
        apply_id: Uuid,
        tenant_id: &str,
        applied: &[InfraResource],
    ) -> RollbackPlan {
        let steps: Vec<RollbackStep> = applied
            .iter()
            .rev()
            .map(|r| RollbackStep::new(&r.id, RollbackAction::Destroy))
            .collect();

        RollbackPlan::new(apply_id, tenant_id, steps)
    }
}

impl Default for RollbackManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ResourceType;
    use crate::state::InfraResource;

    fn make_resource(id: &str) -> InfraResource {
        InfraResource::new(
            id,
            ResourceType::Vm,
            "mock",
            id,
            "tenant-t",
            serde_json::json!({}),
        )
    }

    #[tokio::test]
    async fn test_build_plan_from_applied() {
        let apply_id = Uuid::new_v4();
        let applied = vec![
            make_resource("r1"),
            make_resource("r2"),
            make_resource("r3"),
        ];

        let plan = RollbackManager::build_from_applied(apply_id, "tenant-t", &applied);

        assert_eq!(plan.apply_id, apply_id);
        assert_eq!(plan.steps.len(), 3);
        // Steps should be in reverse order: r3, r2, r1.
        assert_eq!(plan.steps[0].resource_id, "r3");
        assert_eq!(plan.steps[1].resource_id, "r2");
        assert_eq!(plan.steps[2].resource_id, "r1");
        assert!(matches!(plan.steps[0].action, RollbackAction::Destroy));
        assert!(!plan.all_complete());
    }

    #[tokio::test]
    async fn test_execute_step_marks_complete() {
        let mgr = RollbackManager::new();
        let apply_id = Uuid::new_v4();
        let applied = vec![make_resource("res-a"), make_resource("res-b")];

        let plan = RollbackManager::build_from_applied(apply_id, "tenant-t", &applied);
        let plan_id = mgr.create_plan(plan).await;

        // Execute the first step successfully.
        mgr.execute_step(plan_id, "res-b", true, None).await.unwrap();
        // Execute the second step with a failure.
        mgr.execute_step(plan_id, "res-a", false, Some("cleanup failed".to_string()))
            .await
            .unwrap();

        mgr.complete_plan(plan_id).await.unwrap();

        let fetched = mgr.get(plan_id).await.unwrap();
        assert!(fetched.all_complete());
        assert!(fetched.has_errors());
        assert_eq!(fetched.status, RollbackStatus::PartialFailure);
        assert!(fetched.completed_at.is_some());
    }
}
