// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! JIT privileged access request workflow.
//!
//! Implements the Teleport-style just-in-time role elevation model:
//! users create time-bound requests for elevated roles; designated approvers
//! accept or deny them; approved requests generate a short-lived session grant.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced by the access-request subsystem.
#[derive(Debug, PartialEq, Clone)]
pub enum RequestError {
    /// The request was already approved or denied.
    AlreadyDecided,
    /// No request found with the given ID.
    NotFound,
    /// The request has already expired.
    Expired,
    /// A required field is missing or invalid.
    InvalidInput(String),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyDecided => write!(f, "request has already been decided"),
            Self::NotFound => write!(f, "access request not found"),
            Self::Expired => write!(f, "access request has expired"),
            Self::InvalidInput(msg) => write!(f, "invalid input: {msg}"),
        }
    }
}

impl std::error::Error for RequestError {}

// ── Domain types ──────────────────────────────────────────────────────────────

/// Lifecycle state of an access request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// Parameters for creating a new access request.
#[derive(Debug, Clone)]
pub struct CreateRequest {
    /// User initiating the request.
    pub user_id: Uuid,
    /// Roles the user is requesting.
    pub requested_roles: Vec<String>,
    /// Human-readable justification.
    pub reason: String,
    /// How long the request (and any resulting grant) should be valid.
    pub ttl: Duration,
}

/// Decision applied by an approver.
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    Approve {
        approver_id: Uuid,
        /// Optional approval note.
        note: Option<String>,
    },
    Deny {
        denier_id: Uuid,
        reason: String,
    },
}

/// A stored access request record.
#[derive(Debug, Clone)]
pub struct AccessRequestRecord {
    /// Unique identifier.
    pub id: Uuid,
    /// Requester.
    pub user_id: Uuid,
    /// Roles being requested.
    pub requested_roles: Vec<String>,
    /// Human justification.
    pub reason: String,
    /// When the request was submitted.
    pub created_at: DateTime<Utc>,
    /// When the request (and any resulting grant) expires.
    pub expires_at: DateTime<Utc>,
    /// Current lifecycle state.
    pub state: RequestState,
    /// Who approved or denied, if decided.
    pub decided_by: Option<Uuid>,
    /// When the decision was made.
    pub decided_at: Option<DateTime<Utc>>,
    /// Approver/denier note or denial reason.
    pub decision_note: Option<String>,
}

impl AccessRequestRecord {
    /// Return true if the request's TTL has elapsed.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Return true if the request is still actionable (pending, not expired).
    pub fn is_actionable(&self) -> bool {
        self.state == RequestState::Pending && !self.is_expired()
    }
}

// ── In-memory store ───────────────────────────────────────────────────────────

/// Thread-safe in-memory store for access requests.
///
/// In production this would be backed by cave-etcd or a database via
/// cave-store; the interface is the same.
#[derive(Debug, Default)]
pub struct AccessRequestStore {
    inner: Arc<RwLock<HashMap<Uuid, AccessRequestRecord>>>,
}

impl AccessRequestStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit a new access request.
    ///
    /// Returns the new request's UUID.
    pub fn create(&self, req: CreateRequest) -> Result<Uuid, RequestError> {
        if req.requested_roles.is_empty() {
            return Err(RequestError::InvalidInput(
                "at least one role must be requested".to_string(),
            ));
        }
        if req.reason.trim().is_empty() {
            return Err(RequestError::InvalidInput(
                "reason must not be empty".to_string(),
            ));
        }
        let now = Utc::now();
        let id = Uuid::new_v4();
        let record = AccessRequestRecord {
            id,
            user_id: req.user_id,
            requested_roles: req.requested_roles,
            reason: req.reason,
            created_at: now,
            expires_at: now + req.ttl,
            state: RequestState::Pending,
            decided_by: None,
            decided_at: None,
            decision_note: None,
        };
        self.inner.write().unwrap().insert(id, record);
        Ok(id)
    }

    /// Look up a request by ID.
    pub fn get(&self, id: &Uuid) -> Option<AccessRequestRecord> {
        self.inner.read().unwrap().get(id).cloned()
    }

    /// Apply an approval or denial decision.
    ///
    /// Errors if the request does not exist, has already been decided, or is
    /// expired.
    pub fn decide(&self, id: &Uuid, decision: ApprovalDecision) -> Result<(), RequestError> {
        let mut map = self.inner.write().unwrap();
        let record = map.get_mut(id).ok_or(RequestError::NotFound)?;

        if record.state != RequestState::Pending {
            return Err(RequestError::AlreadyDecided);
        }
        if record.is_expired() {
            record.state = RequestState::Expired;
            return Err(RequestError::Expired);
        }

        let now = Utc::now();
        match decision {
            ApprovalDecision::Approve { approver_id, note } => {
                record.state = RequestState::Approved;
                record.decided_by = Some(approver_id);
                record.decided_at = Some(now);
                record.decision_note = note;
            }
            ApprovalDecision::Deny { denier_id, reason } => {
                record.state = RequestState::Denied;
                record.decided_by = Some(denier_id);
                record.decided_at = Some(now);
                record.decision_note = Some(reason);
            }
        }
        Ok(())
    }

    /// Return all requests currently in the Pending state (and not expired).
    pub fn list_pending(&self) -> Vec<AccessRequestRecord> {
        self.inner
            .read()
            .unwrap()
            .values()
            .filter(|r| r.state == RequestState::Pending && !r.is_expired())
            .cloned()
            .collect()
    }

    /// Return all requests for a given user, sorted newest-first.
    pub fn list_for_user(&self, user_id: &Uuid) -> Vec<AccessRequestRecord> {
        let mut records: Vec<AccessRequestRecord> = self
            .inner
            .read()
            .unwrap()
            .values()
            .filter(|r| &r.user_id == user_id)
            .cloned()
            .collect();
        records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        records
    }

    /// Expire all pending requests whose TTL has elapsed. Returns the count.
    pub fn sweep_expired(&self) -> usize {
        let mut map = self.inner.write().unwrap();
        let now = Utc::now();
        let mut count = 0usize;
        for record in map.values_mut() {
            if record.state == RequestState::Pending && now > record.expires_at {
                record.state = RequestState::Expired;
                count += 1;
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_roles_rejected() {
        let store = AccessRequestStore::new();
        let req = CreateRequest {
            user_id: Uuid::new_v4(),
            requested_roles: vec![],
            reason: "test".to_string(),
            ttl: Duration::hours(1),
        };
        assert!(store.create(req).is_err());
    }

    #[test]
    fn empty_reason_rejected() {
        let store = AccessRequestStore::new();
        let req = CreateRequest {
            user_id: Uuid::new_v4(),
            requested_roles: vec!["admin".to_string()],
            reason: "   ".to_string(),
            ttl: Duration::hours(1),
        };
        assert!(store.create(req).is_err());
    }

    #[test]
    fn sweep_expired_marks_expired_state() {
        let store = AccessRequestStore::new();
        let req = CreateRequest {
            user_id: Uuid::new_v4(),
            requested_roles: vec!["admin".to_string()],
            reason: "test".to_string(),
            ttl: Duration::seconds(-10),
        };
        store.create(req).unwrap();
        let swept = store.sweep_expired();
        assert_eq!(swept, 1);
    }

    #[test]
    fn list_for_user_filters_correctly() {
        let store = AccessRequestStore::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();
        store
            .create(CreateRequest {
                user_id: user_a,
                requested_roles: vec!["role-a".to_string()],
                reason: "test a".to_string(),
                ttl: Duration::hours(1),
            })
            .unwrap();
        store
            .create(CreateRequest {
                user_id: user_b,
                requested_roles: vec!["role-b".to_string()],
                reason: "test b".to_string(),
                ttl: Duration::hours(1),
            })
            .unwrap();
        assert_eq!(store.list_for_user(&user_a).len(), 1);
        assert_eq!(store.list_for_user(&user_b).len(), 1);
    }
}
