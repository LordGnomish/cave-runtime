//! LLM infrastructure planner — turns a parsed intent into an actionable plan.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::providers::ResourceType;

// ── PlannedResource ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedResource {
    /// Logical ID within this plan.
    pub id: String,
    pub resource_type: ResourceType,
    pub provider: String,
    pub name: String,
    pub spec: serde_json::Value,
    /// Logical IDs of resources this one depends on.
    pub depends_on: Vec<String>,
    pub estimated_cost_usd: Option<f64>,
}

// ── PlanStatus ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStatus {
    Draft,
    PendingApproval,
    Approved,
    Rejected,
    Applying,
    Applied,
    Failed(String),
}

// ── InfraPlan ─────────────────────────────────────────────────────────────────

/// A complete infrastructure change-set derived from an intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfraPlan {
    pub id: Uuid,
    pub intent_id: Uuid,
    pub tenant_id: String,
    pub resources_to_create: Vec<PlannedResource>,
    pub resources_to_update: Vec<PlannedResource>,
    /// IDs of resources that should be destroyed.
    pub resources_to_destroy: Vec<String>,
    pub estimated_cost_usd: Option<f64>,
    pub status: PlanStatus,
    pub created_at: DateTime<Utc>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
}

impl InfraPlan {
    pub fn new(intent_id: Uuid, tenant_id: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            intent_id,
            tenant_id: tenant_id.to_string(),
            resources_to_create: Vec::new(),
            resources_to_update: Vec::new(),
            resources_to_destroy: Vec::new(),
            estimated_cost_usd: None,
            status: PlanStatus::Draft,
            created_at: Utc::now(),
            approved_by: None,
            approved_at: None,
        }
    }

    /// Sum the estimated costs of all resources in this plan.
    pub fn total_estimated_cost(&self) -> f64 {
        let create: f64 = self
            .resources_to_create
            .iter()
            .filter_map(|r| r.estimated_cost_usd)
            .sum();
        let update: f64 = self
            .resources_to_update
            .iter()
            .filter_map(|r| r.estimated_cost_usd)
            .sum();
        create + update
    }

    /// Total number of resource operations planned.
    pub fn resource_count(&self) -> usize {
        self.resources_to_create.len()
            + self.resources_to_update.len()
            + self.resources_to_destroy.len()
    }
}

// ── PlannerError ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PlannerError {
    #[error("LLM unavailable: {0}")]
    LlmUnavailable(String),
    #[error("Cannot plan: {0}")]
    CannotPlan(String),
    #[error("Rate limited")]
    RateLimited,
}

// ── InfraPlanner trait ────────────────────────────────────────────────────────

/// Planners turn an intent into a concrete `InfraPlan`.
#[async_trait::async_trait]
pub trait InfraPlanner: Send + Sync {
    async fn generate_plan(
        &self,
        intent: &crate::intent::InfraIntent,
    ) -> Result<InfraPlan, PlannerError>;
}

// ── LlmPlanner ────────────────────────────────────────────────────────────────

/// HTTP-based LLM planner (calls Claude API or compatible endpoint).
pub struct LlmPlanner {
    pub api_endpoint: String,
    pub api_key: String,
    client: reqwest::Client,
}

impl LlmPlanner {
    pub fn new(api_endpoint: &str, api_key: &str) -> Self {
        Self {
            api_endpoint: api_endpoint.to_string(),
            api_key: api_key.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl InfraPlanner for LlmPlanner {
    async fn generate_plan(
        &self,
        intent: &crate::intent::InfraIntent,
    ) -> Result<InfraPlan, PlannerError> {
        let body = serde_json::json!({
            "model": "claude-3-5-sonnet-latest",
            "messages": [{
                "role": "user",
                "content": format!(
                    "Generate an infrastructure plan for: {}",
                    intent.description
                )
            }]
        });

        let resp = self
            .client
            .post(&self.api_endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| PlannerError::LlmUnavailable(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(PlannerError::RateLimited);
        }

        if !resp.status().is_success() {
            return Err(PlannerError::LlmUnavailable(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        // A real implementation would parse the LLM response; here we return
        // an empty draft plan to keep the HTTP path compiling.
        Ok(InfraPlan::new(intent.id, &intent.tenant_id))
    }
}

// ── MockPlanner ───────────────────────────────────────────────────────────────

/// Deterministic mock planner for tests.
pub struct MockPlanner {
    pub resources: Vec<PlannedResource>,
}

impl MockPlanner {
    pub fn new(resources: Vec<PlannedResource>) -> Self {
        Self { resources }
    }
}

#[async_trait::async_trait]
impl InfraPlanner for MockPlanner {
    async fn generate_plan(
        &self,
        intent: &crate::intent::InfraIntent,
    ) -> Result<InfraPlan, PlannerError> {
        let mut plan = InfraPlan::new(intent.id, &intent.tenant_id);
        plan.resources_to_create = self.resources.clone();
        // Populate the aggregate cost field.
        let total = plan.total_estimated_cost();
        if total > 0.0 {
            plan.estimated_cost_usd = Some(total);
        }
        Ok(plan)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::InfraIntent;

    fn make_planned_resource(id: &str, cost: f64) -> PlannedResource {
        PlannedResource {
            id: id.to_string(),
            resource_type: ResourceType::Vm,
            provider: "mock".to_string(),
            name: format!("resource-{id}"),
            spec: serde_json::json!({}),
            depends_on: vec![],
            estimated_cost_usd: Some(cost),
        }
    }

    #[tokio::test]
    async fn test_mock_planner_generates_plan() {
        let resources = vec![
            make_planned_resource("r1", 10.0),
            make_planned_resource("r2", 20.0),
        ];
        let planner = MockPlanner::new(resources);

        let user = uuid::Uuid::new_v4();
        let intent = InfraIntent::new("deploy two vms", "t1", user);

        let plan = planner.generate_plan(&intent).await.unwrap();
        assert_eq!(plan.intent_id, intent.id);
        assert_eq!(plan.tenant_id, "t1");
        assert_eq!(plan.resources_to_create.len(), 2);
    }

    #[tokio::test]
    async fn test_plan_resource_count_and_cost() {
        let resources = vec![
            make_planned_resource("a", 5.0),
            make_planned_resource("b", 15.0),
        ];
        let planner = MockPlanner::new(resources);

        let user = uuid::Uuid::new_v4();
        let intent = InfraIntent::new("two small vms", "t2", user);
        let plan = planner.generate_plan(&intent).await.unwrap();

        assert_eq!(plan.resource_count(), 2);
        assert!((plan.total_estimated_cost() - 20.0).abs() < f64::EPSILON);
    }
}
