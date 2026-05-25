// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../authorization/permission/PermissionTicketService.java + Kantara UMA-FedAuthz §3
//
//! UMA 2.0 permission ticket — issued by the AS when a Resource Server
//! requests access on behalf of a requesting party.
//!
//! Per Federated Authz §3, the AS:
//!   1. Accepts a POST to `/uma2/permission` carrying `{resource_id, resource_scopes[]}`.
//!   2. Returns a single-use, opaque, signed ticket bound to the requested permissions.
//!   3. The client later exchanges the ticket at `/token` (grant_type=urn:ietf:params:oauth:grant-type:uma-ticket).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// A permission request entry — one or many of these may share a ticket.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PermissionRequest {
    pub resource_id: String,
    /// `resource_scopes` field per UMA-FedAuthz §3.1.
    #[serde(default)]
    pub resource_scopes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PermissionTicket {
    /// Opaque ticket id (UUID v4).
    pub ticket: String,
    /// All permissions bundled into the ticket.
    pub permissions: Vec<PermissionRequest>,
    /// Resource owner inferred from the resource_set entries.
    pub resource_owner: String,
    /// Issuance timestamp.
    pub issued_at: DateTime<Utc>,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
    /// Already-redeemed flag — single-use tickets per spec.
    pub redeemed: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TicketError {
    #[error("permission ticket request must contain at least one resource_id")]
    Empty,
    #[error("permission ticket {0:?} not found")]
    NotFound(String),
    #[error("permission ticket {0:?} already redeemed (single-use)")]
    AlreadyRedeemed(String),
    #[error("permission ticket {0:?} has expired")]
    Expired(String),
}

pub struct PermissionTicketStore {
    ttl_seconds: i64,
    inner: Mutex<HashMap<String, PermissionTicket>>,
}

impl Default for PermissionTicketStore {
    fn default() -> Self {
        Self::new(300)
    }
}

impl PermissionTicketStore {
    pub fn new(ttl_seconds: i64) -> Self {
        Self {
            ttl_seconds,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Issues a new ticket. `resource_owner` is taken from the caller (the RS
    /// looks it up via the resource registration store) — we accept it as a
    /// parameter to keep this module decoupled.
    pub fn issue(
        &self,
        requests: Vec<PermissionRequest>,
        resource_owner: String,
        now: DateTime<Utc>,
    ) -> Result<PermissionTicket, TicketError> {
        if requests.is_empty() {
            return Err(TicketError::Empty);
        }
        let ticket = PermissionTicket {
            ticket: Uuid::new_v4().to_string(),
            permissions: requests,
            resource_owner,
            issued_at: now,
            expires_at: now + Duration::seconds(self.ttl_seconds),
            redeemed: false,
        };
        self.inner
            .lock()
            .unwrap()
            .insert(ticket.ticket.clone(), ticket.clone());
        Ok(ticket)
    }

    /// Looks up a ticket WITHOUT consuming it (the introspection path).
    pub fn get(&self, ticket: &str) -> Result<PermissionTicket, TicketError> {
        let guard = self.inner.lock().unwrap();
        guard
            .get(ticket)
            .cloned()
            .ok_or_else(|| TicketError::NotFound(ticket.to_string()))
    }

    /// Redeem a ticket — used by the RPT issuance endpoint. Single-use.
    pub fn redeem(
        &self,
        ticket: &str,
        now: DateTime<Utc>,
    ) -> Result<PermissionTicket, TicketError> {
        let mut guard = self.inner.lock().unwrap();
        let entry = guard
            .get_mut(ticket)
            .ok_or_else(|| TicketError::NotFound(ticket.to_string()))?;
        if entry.redeemed {
            return Err(TicketError::AlreadyRedeemed(ticket.to_string()));
        }
        if entry.expires_at <= now {
            return Err(TicketError::Expired(ticket.to_string()));
        }
        entry.redeemed = true;
        Ok(entry.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn req(rs: &str, scopes: &[&str]) -> PermissionRequest {
        PermissionRequest {
            resource_id: rs.into(),
            resource_scopes: scopes.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }

    #[test]
    fn issue_returns_opaque_ticket() {
        let store = PermissionTicketStore::default();
        let t = store
            .issue(vec![req("rs1", &["view"])], "alice".into(), now())
            .unwrap();
        assert!(!t.ticket.is_empty());
        assert_eq!(t.permissions.len(), 1);
        assert!(!t.redeemed);
    }

    #[test]
    fn issue_with_empty_requests_fails() {
        let store = PermissionTicketStore::default();
        let err = store.issue(vec![], "alice".into(), now()).unwrap_err();
        assert_eq!(err, TicketError::Empty);
    }

    #[test]
    fn redeem_marks_used() {
        let store = PermissionTicketStore::default();
        let t = store
            .issue(vec![req("rs1", &["view"])], "alice".into(), now())
            .unwrap();
        let redeemed = store.redeem(&t.ticket, now()).unwrap();
        assert!(redeemed.redeemed);
    }

    #[test]
    fn redeem_twice_fails() {
        let store = PermissionTicketStore::default();
        let t = store
            .issue(vec![req("rs1", &["view"])], "alice".into(), now())
            .unwrap();
        store.redeem(&t.ticket, now()).unwrap();
        let err = store.redeem(&t.ticket, now()).unwrap_err();
        assert!(matches!(err, TicketError::AlreadyRedeemed(_)));
    }

    #[test]
    fn redeem_unknown_fails() {
        let store = PermissionTicketStore::default();
        let err = store.redeem("nope", now()).unwrap_err();
        assert!(matches!(err, TicketError::NotFound(_)));
    }

    #[test]
    fn redeem_after_expiry_fails() {
        let store = PermissionTicketStore::new(60);
        let t = store
            .issue(vec![req("rs1", &["view"])], "alice".into(), now())
            .unwrap();
        let later = now() + Duration::seconds(120);
        let err = store.redeem(&t.ticket, later).unwrap_err();
        assert!(matches!(err, TicketError::Expired(_)));
    }

    #[test]
    fn get_preserves_ticket_state() {
        let store = PermissionTicketStore::default();
        let t = store
            .issue(vec![req("rs1", &["view"])], "alice".into(), now())
            .unwrap();
        let again = store.get(&t.ticket).unwrap();
        assert!(!again.redeemed);
    }

    #[test]
    fn ticket_expires_at_now_plus_ttl() {
        let store = PermissionTicketStore::new(300);
        let t = store
            .issue(vec![req("rs1", &["view"])], "alice".into(), now())
            .unwrap();
        assert_eq!(t.expires_at - t.issued_at, Duration::seconds(300));
    }

    #[test]
    fn multi_permission_ticket_preserves_order() {
        let store = PermissionTicketStore::default();
        let t = store
            .issue(
                vec![
                    req("rs1", &["view"]),
                    req("rs2", &["edit"]),
                    req("rs3", &["delete"]),
                ],
                "alice".into(),
                now(),
            )
            .unwrap();
        assert_eq!(
            t.permissions
                .iter()
                .map(|p| p.resource_id.clone())
                .collect::<Vec<_>>(),
            vec!["rs1", "rs2", "rs3"]
        );
    }
}
