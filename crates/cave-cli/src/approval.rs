// SPDX-License-Identifier: AGPL-3.0-or-later
//! Approval workflow — `cavectl approval list|approve|cancel|show`.
//!
//! Two-person rule: bir kullanıcı kendi talep ettiği approval'ı approve edemez,
//! en az 2 distinct approver gerekir. Quorum = 2 default; per-action override
//! mümkün. State machine: Pending → (Approved | Cancelled). Approved sonrası
//! ek approve idempotent (no-op).

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalState {
    Pending,
    Approved,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub tenant_id: String,
    pub action: String,
    pub requested_by: String,
    pub approvers: BTreeSet<String>,
    pub quorum: usize,
    pub state: ApprovalState,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

impl ApprovalRecord {
    pub fn remaining(&self) -> usize {
        self.quorum.saturating_sub(self.approvers.len())
    }
}

#[async_trait]
pub trait ApprovalBackend: Send + Sync {
    async fn create(
        &self,
        tenant_id: &str,
        action: &str,
        requested_by: &str,
        quorum: usize,
    ) -> Result<ApprovalRecord>;
    async fn list(&self, tenant_id: &str, state: Option<ApprovalState>) -> Result<Vec<ApprovalRecord>>;
    async fn show(&self, tenant_id: &str, approval_id: &str) -> Result<ApprovalRecord>;
    async fn approve(
        &self,
        tenant_id: &str,
        approval_id: &str,
        approver: &str,
    ) -> Result<ApprovalRecord>;
    async fn cancel(
        &self,
        tenant_id: &str,
        approval_id: &str,
        actor: &str,
    ) -> Result<ApprovalRecord>;
}

#[derive(Default)]
pub struct InMemoryApprovals {
    inner: Arc<RwLock<HashMap<(String, String), ApprovalRecord>>>,
}

impl InMemoryApprovals {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ApprovalBackend for InMemoryApprovals {
    async fn create(
        &self,
        tenant_id: &str,
        action: &str,
        requested_by: &str,
        quorum: usize,
    ) -> Result<ApprovalRecord> {
        if quorum == 0 {
            return Err(anyhow!("quorum must be ≥ 1"));
        }
        let approval_id = Uuid::new_v4().to_string();
        let rec = ApprovalRecord {
            approval_id: approval_id.clone(),
            tenant_id: tenant_id.to_string(),
            action: action.to_string(),
            requested_by: requested_by.to_string(),
            approvers: BTreeSet::new(),
            quorum,
            state: ApprovalState::Pending,
            created_at: Utc::now(),
            closed_at: None,
        };
        self.inner
            .write()
            .insert((tenant_id.to_string(), approval_id), rec.clone());
        Ok(rec)
    }

    async fn list(
        &self,
        tenant_id: &str,
        state: Option<ApprovalState>,
    ) -> Result<Vec<ApprovalRecord>> {
        let s = self.inner.read();
        let mut out: Vec<ApprovalRecord> = s
            .values()
            .filter(|r| r.tenant_id == tenant_id)
            .filter(|r| match state {
                Some(want) => r.state == want,
                None => true,
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    async fn show(&self, tenant_id: &str, approval_id: &str) -> Result<ApprovalRecord> {
        self.inner
            .read()
            .get(&(tenant_id.to_string(), approval_id.to_string()))
            .cloned()
            .ok_or_else(|| anyhow!("approval not found: {tenant_id}/{approval_id}"))
    }

    async fn approve(
        &self,
        tenant_id: &str,
        approval_id: &str,
        approver: &str,
    ) -> Result<ApprovalRecord> {
        let mut s = self.inner.write();
        let rec = s
            .get_mut(&(tenant_id.to_string(), approval_id.to_string()))
            .ok_or_else(|| anyhow!("approval not found: {tenant_id}/{approval_id}"))?;
        match rec.state {
            ApprovalState::Cancelled => return Err(anyhow!("approval is cancelled")),
            ApprovalState::Approved => return Ok(rec.clone()),
            ApprovalState::Pending => {}
        }
        if approver == rec.requested_by {
            return Err(anyhow!(
                "two-person rule: requester '{}' cannot approve own request",
                approver
            ));
        }
        rec.approvers.insert(approver.to_string());
        if rec.approvers.len() >= rec.quorum {
            rec.state = ApprovalState::Approved;
            rec.closed_at = Some(Utc::now());
        }
        Ok(rec.clone())
    }

    async fn cancel(
        &self,
        tenant_id: &str,
        approval_id: &str,
        _actor: &str,
    ) -> Result<ApprovalRecord> {
        let mut s = self.inner.write();
        let rec = s
            .get_mut(&(tenant_id.to_string(), approval_id.to_string()))
            .ok_or_else(|| anyhow!("approval not found: {tenant_id}/{approval_id}"))?;
        match rec.state {
            ApprovalState::Approved => return Err(anyhow!("cannot cancel approved request")),
            ApprovalState::Cancelled => return Ok(rec.clone()),
            ApprovalState::Pending => {}
        }
        rec.state = ApprovalState::Cancelled;
        rec.closed_at = Some(Utc::now());
        Ok(rec.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cite: two-person rule — newly created approval starts Pending with 0 approvers
    #[tokio::test]
    async fn approval_acme_create_starts_pending() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "rotate-prod-key", "alice", 2).await.unwrap();
        assert_eq!(rec.state, ApprovalState::Pending);
        assert_eq!(rec.approvers.len(), 0);
        assert_eq!(rec.remaining(), 2);
    }

    /// cite: two-person rule — quorum 0 rejected
    #[tokio::test]
    async fn approval_globex_quorum_zero_rejected() {
        let tenant_id = "globex";
        let b = InMemoryApprovals::new();
        let err = b.create(tenant_id, "drop-table", "alice", 0).await.unwrap_err();
        assert!(err.to_string().contains("quorum"));
    }

    /// cite: two-person rule — requester cannot self-approve
    #[tokio::test]
    async fn approval_acme_self_approve_rejected() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "rotate-prod-key", "alice", 2).await.unwrap();
        let err = b.approve(tenant_id, &rec.approval_id, "alice").await.unwrap_err();
        assert!(err.to_string().contains("two-person rule"));
    }

    /// cite: two-person rule — single approver leaves state Pending when quorum=2
    #[tokio::test]
    async fn approval_acme_single_approver_stays_pending() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "rotate-prod-key", "alice", 2).await.unwrap();
        let after = b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap();
        assert_eq!(after.state, ApprovalState::Pending);
        assert_eq!(after.remaining(), 1);
    }

    /// cite: two-person rule — quorum reached transitions to Approved
    #[tokio::test]
    async fn approval_acme_quorum_reached_approves() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "rotate-prod-key", "alice", 2).await.unwrap();
        b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap();
        let after = b.approve(tenant_id, &rec.approval_id, "carol").await.unwrap();
        assert_eq!(after.state, ApprovalState::Approved);
        assert!(after.closed_at.is_some());
    }

    /// cite: two-person rule — duplicate approver counts once
    #[tokio::test]
    async fn approval_initech_duplicate_approver_idempotent() {
        let tenant_id = "initech";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "drop-pii", "alice", 2).await.unwrap();
        b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap();
        let after = b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap();
        assert_eq!(after.state, ApprovalState::Pending);
        assert_eq!(after.approvers.len(), 1);
    }

    /// cite: two-person rule — re-approve on already-approved is no-op
    #[tokio::test]
    async fn approval_acme_post_approve_is_noop() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "rotate-prod-key", "alice", 2).await.unwrap();
        b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap();
        b.approve(tenant_id, &rec.approval_id, "carol").await.unwrap();
        let after = b.approve(tenant_id, &rec.approval_id, "dave").await.unwrap();
        assert_eq!(after.state, ApprovalState::Approved);
        assert_eq!(after.approvers.len(), 2);
    }

    /// cite: two-person rule — cancel transitions Pending → Cancelled
    #[tokio::test]
    async fn approval_acme_cancel_transitions_state() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "rotate-prod-key", "alice", 2).await.unwrap();
        let after = b.cancel(tenant_id, &rec.approval_id, "alice").await.unwrap();
        assert_eq!(after.state, ApprovalState::Cancelled);
    }

    /// cite: two-person rule — cancel of approved request rejected
    #[tokio::test]
    async fn approval_globex_cancel_after_approve_rejected() {
        let tenant_id = "globex";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "drop-table", "alice", 2).await.unwrap();
        b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap();
        b.approve(tenant_id, &rec.approval_id, "carol").await.unwrap();
        let err = b.cancel(tenant_id, &rec.approval_id, "alice").await.unwrap_err();
        assert!(err.to_string().contains("approved"));
    }

    /// cite: two-person rule — approve on cancelled rejected
    #[tokio::test]
    async fn approval_initech_approve_after_cancel_rejected() {
        let tenant_id = "initech";
        let b = InMemoryApprovals::new();
        let rec = b.create(tenant_id, "drop-pii", "alice", 2).await.unwrap();
        b.cancel(tenant_id, &rec.approval_id, "alice").await.unwrap();
        let err = b.approve(tenant_id, &rec.approval_id, "bob").await.unwrap_err();
        assert!(err.to_string().contains("cancelled"));
    }

    /// cite: two-person rule — list filtered by Pending excludes approved
    #[tokio::test]
    async fn approval_acme_list_pending_excludes_approved() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let r1 = b.create(tenant_id, "rotate-key", "alice", 2).await.unwrap();
        let _r2 = b.create(tenant_id, "drop-table", "alice", 2).await.unwrap();
        b.approve(tenant_id, &r1.approval_id, "bob").await.unwrap();
        b.approve(tenant_id, &r1.approval_id, "carol").await.unwrap();
        let pending = b.list(tenant_id, Some(ApprovalState::Pending)).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending.iter().all(|r| r.state == ApprovalState::Pending));
    }

    /// cite: two-person rule — list scoped to tenant
    #[tokio::test]
    async fn approval_list_acme_excludes_globex() {
        let b = InMemoryApprovals::new();
        b.create("acme", "rotate-key", "alice", 2).await.unwrap();
        b.create("globex", "rotate-key", "alice", 2).await.unwrap();
        let acme = b.list("acme", None).await.unwrap();
        assert_eq!(acme.len(), 1);
        assert!(acme.iter().all(|r| r.tenant_id == "acme"));
    }

    /// cite: two-person rule — show on missing approval returns descriptive error
    #[tokio::test]
    async fn approval_show_unknown_errors() {
        let tenant_id = "acme";
        let b = InMemoryApprovals::new();
        let err = b.show(tenant_id, "bogus").await.unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }
}
