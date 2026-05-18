// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gravitee API Lifecycle Management — draft → published → deprecated → retired
//! state machine, review/approval workflow, changelog, and audit trail.

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct LifecycleStore {
    pub api_versions: HashMap<Uuid, ApiLifecycle>,
    pub reviews: HashMap<Uuid, ReviewRequest>,
    pub audit: Vec<AuditEvent>,
}

impl LifecycleStore {
    pub fn new() -> Self {
        Self {
            api_versions: HashMap::new(),
            reviews: HashMap::new(),
            audit: Vec::new(),
        }
    }

    pub fn create_version(&mut self, req: CreateApiVersionRequest) -> ApiLifecycle {
        let now = chrono::Utc::now();
        let api = ApiLifecycle {
            id: Uuid::new_v4(),
            api_name: req.api_name.clone(),
            version: req.version.clone(),
            state: LifecycleState::Draft,
            spec_id: req.spec_id,
            changelog: Vec::new(),
            migration_guide: req.migration_guide,
            deprecated_at: None,
            retire_at: None,
            created_at: now,
            updated_at: now,
        };
        self.api_versions.insert(api.id, api.clone());
        self.record_audit(
            "api_version",
            api.id,
            "created",
            "system",
            serde_json::json!({ "api_name": req.api_name, "version": req.version }),
        );
        api
    }

    /// Attempt a lifecycle state transition. Returns error string on illegal transition.
    pub fn transition(
        &mut self,
        id: Uuid,
        req: TransitionRequest,
    ) -> Result<ApiLifecycle, String> {
        let api = self.api_versions.get_mut(&id)
            .ok_or_else(|| "api version not found".to_string())?;

        let allowed = match (&api.state, &req.target_state) {
            (LifecycleState::Draft, LifecycleState::PendingReview) => true,
            (LifecycleState::Draft, LifecycleState::Published) => true, // direct publish (no review required)
            (LifecycleState::PendingReview, LifecycleState::Published) => true,
            (LifecycleState::PendingReview, LifecycleState::Draft) => true, // reject → back to draft
            (LifecycleState::Published, LifecycleState::Deprecated) => true,
            (LifecycleState::Deprecated, LifecycleState::Retired) => true,
            _ => false,
        };

        if !allowed {
            return Err(format!(
                "transition from {} to {} is not allowed",
                api.state, req.target_state
            ));
        }

        let prev_state = api.state.clone();
        if req.target_state == LifecycleState::Deprecated {
            api.deprecated_at = Some(chrono::Utc::now());
        }
        api.state = req.target_state.clone();
        api.updated_at = chrono::Utc::now();

        self.record_audit(
            "api_version",
            id,
            "state_changed",
            &req.actor,
            serde_json::json!({
                "from": prev_state.to_string(),
                "to": req.target_state.to_string(),
                "reason": req.reason,
            }),
        );
        Ok(self.api_versions[&id].clone())
    }

    pub fn submit_for_review(
        &mut self,
        api_id: Uuid,
        req: SubmitReviewRequest,
    ) -> Result<ReviewRequest, String> {
        let api = self.api_versions.get_mut(&api_id)
            .ok_or_else(|| "api version not found".to_string())?;

        if api.state != LifecycleState::Draft {
            return Err("only Draft APIs can be submitted for review".into());
        }
        api.state = LifecycleState::PendingReview;
        api.updated_at = chrono::Utc::now();

        let review = ReviewRequest {
            id: Uuid::new_v4(),
            api_lifecycle_id: api_id,
            submitted_by: req.submitted_by.clone(),
            status: ReviewStatus::Pending,
            comments: req.comment.map(|c| vec![ReviewComment {
                author: req.submitted_by.clone(),
                comment: c,
                created_at: chrono::Utc::now(),
            }]).unwrap_or_default(),
            submitted_at: chrono::Utc::now(),
            resolved_at: None,
        };
        self.reviews.insert(review.id, review.clone());
        self.record_audit(
            "review",
            review.id,
            "submitted",
            &req.submitted_by,
            serde_json::json!({ "api_id": api_id }),
        );
        Ok(review)
    }

    pub fn approve_review(
        &mut self,
        review_id: Uuid,
        req: ReviewDecisionRequest,
    ) -> Result<ReviewRequest, String> {
        self.decide_review(review_id, req, ReviewStatus::Approved)
    }

    pub fn reject_review(
        &mut self,
        review_id: Uuid,
        req: ReviewDecisionRequest,
    ) -> Result<ReviewRequest, String> {
        self.decide_review(review_id, req, ReviewStatus::Rejected)
    }

    fn decide_review(
        &mut self,
        review_id: Uuid,
        req: ReviewDecisionRequest,
        decision: ReviewStatus,
    ) -> Result<ReviewRequest, String> {
        let review = self.reviews.get_mut(&review_id)
            .ok_or_else(|| "review not found".to_string())?;
        if review.status != ReviewStatus::Pending {
            return Err("review is already resolved".into());
        }
        review.status = decision.clone();
        review.resolved_at = Some(chrono::Utc::now());
        review.comments.push(ReviewComment {
            author: req.reviewer.clone(),
            comment: req.comment,
            created_at: chrono::Utc::now(),
        });

        // Auto-transition the API based on decision.
        let api_id = review.api_lifecycle_id;
        let review_clone = review.clone();
        if let Some(api) = self.api_versions.get_mut(&api_id) {
            match decision {
                ReviewStatus::Approved => {
                    api.state = LifecycleState::Published;
                }
                ReviewStatus::Rejected => {
                    api.state = LifecycleState::Draft;
                }
                ReviewStatus::Pending => {}
            }
            api.updated_at = chrono::Utc::now();
        }
        self.record_audit(
            "review",
            review_id,
            if matches!(decision, ReviewStatus::Approved) { "approved" } else { "rejected" },
            &req.reviewer,
            serde_json::json!({ "api_id": api_id }),
        );
        Ok(review_clone)
    }

    pub fn add_changelog(
        &mut self,
        api_id: Uuid,
        req: AddChangelogRequest,
    ) -> Option<ChangelogEntry> {
        let api = self.api_versions.get_mut(&api_id)?;
        let entry = ChangelogEntry {
            version: api.version.clone(),
            date: chrono::Utc::now(),
            description: req.description,
            breaking: req.breaking.unwrap_or(false),
        };
        api.changelog.push(entry.clone());
        api.updated_at = chrono::Utc::now();
        Some(entry)
    }

    fn record_audit(
        &mut self,
        resource_type: &str,
        resource_id: Uuid,
        action: &str,
        actor: &str,
        details: serde_json::Value,
    ) {
        self.audit.push(AuditEvent {
            id: Uuid::new_v4(),
            resource_type: resource_type.to_string(),
            resource_id,
            action: action.to_string(),
            actor: actor.to_string(),
            details,
            occurred_at: chrono::Utc::now(),
        });
    }
}

impl Default for LifecycleStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/v1/gateway/lifecycle/apis", get(list_api_versions).post(create_api_version))
        .route("/api/v1/gateway/lifecycle/apis/{id}", get(get_api_version))
        .route("/api/v1/gateway/lifecycle/apis/{id}/transition", post(transition))
        .route("/api/v1/gateway/lifecycle/apis/{id}/review", post(submit_review))
        .route("/api/v1/gateway/lifecycle/apis/{id}/changelog", post(add_changelog))
        .route("/api/v1/gateway/lifecycle/reviews", get(list_reviews))
        .route("/api/v1/gateway/lifecycle/reviews/{id}/approve", post(approve_review))
        .route("/api/v1/gateway/lifecycle/reviews/{id}/reject", post(reject_review))
        .route("/api/v1/gateway/lifecycle/audit", get(get_audit_trail))
}

async fn list_api_versions(State(state): State<Arc<GatewayState>>) -> Json<Vec<ApiLifecycle>> {
    let store = state.lifecycle.lock().unwrap();
    Json(store.api_versions.values().cloned().collect())
}

async fn create_api_version(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateApiVersionRequest>,
) -> Json<ApiLifecycle> {
    let mut store = state.lifecycle.lock().unwrap();
    Json(store.create_version(req))
}

async fn get_api_version(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.lifecycle.lock().unwrap();
    match store.api_versions.get(&id) {
        Some(a) => Json(serde_json::to_value(a).unwrap()),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn transition(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<TransitionRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.lifecycle.lock().unwrap();
    match store.transition(id, req) {
        Ok(api) => Json(serde_json::to_value(api).unwrap()),
        Err(e) => Json(serde_json::json!({ "error": e })),
    }
}

async fn submit_review(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<SubmitReviewRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.lifecycle.lock().unwrap();
    match store.submit_for_review(id, req) {
        Ok(r) => Json(serde_json::to_value(r).unwrap()),
        Err(e) => Json(serde_json::json!({ "error": e })),
    }
}

async fn add_changelog(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddChangelogRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.lifecycle.lock().unwrap();
    match store.add_changelog(id, req) {
        Some(entry) => Json(serde_json::to_value(entry).unwrap()),
        None => Json(serde_json::json!({ "error": "api version not found" })),
    }
}

async fn list_reviews(State(state): State<Arc<GatewayState>>) -> Json<Vec<ReviewRequest>> {
    let store = state.lifecycle.lock().unwrap();
    Json(store.reviews.values().cloned().collect())
}

async fn approve_review(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ReviewDecisionRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.lifecycle.lock().unwrap();
    match store.approve_review(id, req) {
        Ok(r) => Json(serde_json::to_value(r).unwrap()),
        Err(e) => Json(serde_json::json!({ "error": e })),
    }
}

async fn reject_review(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ReviewDecisionRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.lifecycle.lock().unwrap();
    match store.reject_review(id, req) {
        Ok(r) => Json(serde_json::to_value(r).unwrap()),
        Err(e) => Json(serde_json::json!({ "error": e })),
    }
}

async fn get_audit_trail(State(state): State<Arc<GatewayState>>) -> Json<Vec<AuditEvent>> {
    let store = state.lifecycle.lock().unwrap();
    Json(store.audit.clone())
}
