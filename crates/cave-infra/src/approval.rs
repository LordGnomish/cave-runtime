// SPDX-License-Identifier: AGPL-3.0-or-later
//! Plan approval workflow — human-in-the-loop gate before applying changes.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── ApprovalStatus ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

// ── ApprovalRequest ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub plan_id: Uuid,
    pub tenant_id: String,
    pub requested_by: Uuid,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: ApprovalStatus,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub comment: Option<String>,
}

impl ApprovalRequest {
    /// Create a new pending approval request that expires after `ttl_hours`.
    pub fn new(plan_id: Uuid, tenant_id: &str, requested_by: Uuid, ttl_hours: i64) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            plan_id,
            tenant_id: tenant_id.to_string(),
            requested_by,
            requested_at: now,
            expires_at: now + Duration::hours(ttl_hours),
            status: ApprovalStatus::Pending,
            reviewed_by: None,
            reviewed_at: None,
            comment: None,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn is_pending(&self) -> bool {
        self.status == ApprovalStatus::Pending && !self.is_expired()
    }
}

// ── ApprovalWorkflow ──────────────────────────────────────────────────────────

/// Thread-safe store and logic for plan approvals.
pub struct ApprovalWorkflow {
    requests: Arc<RwLock<HashMap<Uuid, ApprovalRequest>>>,
}

impl ApprovalWorkflow {
    pub fn new() -> Self {
        Self {
            requests: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store a new approval request and return its ID.
    pub async fn submit(&self, req: ApprovalRequest) -> Uuid {
        let id = req.id;
        self.requests.write().await.insert(id, req);
        id
    }

    /// Approve a pending request.
    pub async fn approve(
        &self,
        request_id: Uuid,
        reviewer: Uuid,
        comment: Option<String>,
    ) -> Result<(), String> {
        let mut guard = self.requests.write().await;
        let req = guard
            .get_mut(&request_id)
            .ok_or_else(|| format!("request {request_id} not found"))?;

        if req.is_expired() {
            return Err("request has expired".to_string());
        }
        if req.status != ApprovalStatus::Pending {
            return Err(format!("request is already {:?}", req.status));
        }

        req.status = ApprovalStatus::Approved;
        req.reviewed_by = Some(reviewer);
        req.reviewed_at = Some(Utc::now());
        req.comment = comment;
        Ok(())
    }

    /// Reject a pending request.
    pub async fn reject(
        &self,
        request_id: Uuid,
        reviewer: Uuid,
        reason: String,
    ) -> Result<(), String> {
        let mut guard = self.requests.write().await;
        let req = guard
            .get_mut(&request_id)
            .ok_or_else(|| format!("request {request_id} not found"))?;

        if req.status != ApprovalStatus::Pending {
            return Err(format!("request is already {:?}", req.status));
        }

        req.status = ApprovalStatus::Rejected;
        req.reviewed_by = Some(reviewer);
        req.reviewed_at = Some(Utc::now());
        req.comment = Some(reason);
        Ok(())
    }

    pub async fn get(&self, request_id: Uuid) -> Option<ApprovalRequest> {
        self.requests.read().await.get(&request_id).cloned()
    }

    /// List all pending (non-expired) requests for a tenant.
    pub async fn pending_for_tenant(&self, tenant_id: &str) -> Vec<ApprovalRequest> {
        self.requests
            .read()
            .await
            .values()
            .filter(|r| r.tenant_id == tenant_id && r.is_pending())
            .cloned()
            .collect()
    }

    /// Return the current approval status for a given plan.
    pub async fn check_approval(&self, plan_id: Uuid) -> ApprovalStatus {
        let guard = self.requests.read().await;
        // Find the most recent request for this plan.
        let req = guard
            .values()
            .filter(|r| r.plan_id == plan_id)
            .max_by_key(|r| r.requested_at);

        match req {
            None => ApprovalStatus::Pending,
            Some(r) if r.is_expired() && r.status == ApprovalStatus::Pending => {
                ApprovalStatus::Expired
            }
            Some(r) => r.status.clone(),
        }
    }
}

impl Default for ApprovalWorkflow {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request(plan_id: Uuid, tenant: &str) -> ApprovalRequest {
        ApprovalRequest::new(plan_id, tenant, Uuid::new_v4(), 24)
    }

    #[tokio::test]
    async fn test_approve_flow() {
        let wf = ApprovalWorkflow::new();
        let plan_id = Uuid::new_v4();
        let req = make_request(plan_id, "tenant-a");
        let req_id = wf.submit(req).await;

        let reviewer = Uuid::new_v4();
        wf.approve(req_id, reviewer, Some("LGTM".to_string()))
            .await
            .unwrap();

        let fetched = wf.get(req_id).await.unwrap();
        assert_eq!(fetched.status, ApprovalStatus::Approved);
        assert_eq!(fetched.reviewed_by, Some(reviewer));
        assert_eq!(fetched.comment.as_deref(), Some("LGTM"));

        let status = wf.check_approval(plan_id).await;
        assert_eq!(status, ApprovalStatus::Approved);
    }

    #[tokio::test]
    async fn test_reject_flow() {
        let wf = ApprovalWorkflow::new();
        let plan_id = Uuid::new_v4();
        let req = make_request(plan_id, "tenant-b");
        let req_id = wf.submit(req).await;

        let reviewer = Uuid::new_v4();
        wf.reject(req_id, reviewer, "Too expensive".to_string())
            .await
            .unwrap();

        let fetched = wf.get(req_id).await.unwrap();
        assert_eq!(fetched.status, ApprovalStatus::Rejected);
        assert_eq!(fetched.comment.as_deref(), Some("Too expensive"));

        // Trying to approve after rejection should fail.
        let err = wf.approve(req_id, reviewer, None).await;
        assert!(err.is_err());
    }
}
