// SPDX-License-Identifier: AGPL-3.0-or-later
//
// UMA 2.0 Permission Endpoint — UMA-Grant §3.2.
//
// The resource server POSTs to `/authz/protection/permission` to obtain a
// permission ticket on behalf of an unauthenticated requesting party.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/authorization/protection/permission/PermissionService.java

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::UmaError;

/// UMA-Grant §3.2.1 — request body. May be a single resource (object) or a
/// batch (array of objects).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionRequest {
    pub resource_id: String,
    #[serde(default)]
    pub resource_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<serde_json::Value>,
}

/// UMA-Grant §3.2.2 — response: an opaque, time-bounded ticket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionTicketResponse {
    pub ticket: String,
}

#[derive(Debug, Clone)]
pub struct PermissionTicket {
    pub ticket: String,
    pub realm: String,
    pub permissions: Vec<PermissionRequest>,
    pub created_at: i64,
    pub ttl_seconds: i64,
    /// The audience the ticket was minted for — almost always the client_id
    /// of the resource server.
    pub audience: Option<String>,
}

#[derive(Clone, Default)]
pub struct PermissionTicketStore {
    inner: Arc<Mutex<HashMap<String, PermissionTicket>>>, // ticket -> entry
}

impl PermissionTicketStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a permission ticket for the supplied permission request(s).
    pub fn mint(
        &self,
        realm: &str,
        permissions: Vec<PermissionRequest>,
        audience: Option<String>,
        ttl_seconds: i64,
    ) -> Result<PermissionTicketResponse, UmaError> {
        if permissions.is_empty() {
            return Err(UmaError::InvalidRequest("empty permission set"));
        }
        for p in &permissions {
            if p.resource_id.trim().is_empty() {
                return Err(UmaError::InvalidRequest("resource_id required"));
            }
        }
        let ticket = format!("PT-{}", Uuid::new_v4());
        let entry = PermissionTicket {
            ticket: ticket.clone(),
            realm: realm.to_string(),
            permissions,
            created_at: chrono::Utc::now().timestamp(),
            ttl_seconds,
            audience,
        };
        self.inner.lock().unwrap().insert(ticket.clone(), entry);
        Ok(PermissionTicketResponse { ticket })
    }

    /// Consume a ticket (used at RPT issuance time). Returns the tracked
    /// permissions; the ticket is removed (one-shot).
    pub fn consume(&self, ticket: &str) -> Result<PermissionTicket, UmaError> {
        let mut g = self.inner.lock().unwrap();
        let entry = g.remove(ticket).ok_or(UmaError::InvalidGrant)?;
        let now = chrono::Utc::now().timestamp();
        if now > entry.created_at + entry.ttl_seconds {
            return Err(UmaError::InvalidGrant);
        }
        Ok(entry)
    }

    /// Peek a ticket without consuming — useful for the portal inspector.
    pub fn peek(&self, ticket: &str) -> Option<PermissionTicket> {
        self.inner.lock().unwrap().get(ticket).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perm(rid: &str, scopes: &[&str]) -> PermissionRequest {
        PermissionRequest {
            resource_id: rid.into(),
            resource_scopes: scopes.iter().map(|s| s.to_string()).collect(),
            claims: None,
        }
    }

    // upstream: uma-grant §3.2.2 — minting returns a non-empty ticket.
    #[test]
    fn mint_returns_ticket() {
        let store = PermissionTicketStore::new();
        let resp = store
            .mint("r1", vec![perm("rid-1", &["view"])], Some("rs-client".into()), 60)
            .unwrap();
        assert!(resp.ticket.starts_with("PT-"));
    }

    // upstream: uma-grant §3.2.1 — empty permission set is malformed.
    #[test]
    fn mint_rejects_empty() {
        let store = PermissionTicketStore::new();
        let err = store.mint("r1", vec![], None, 60).unwrap_err();
        assert!(matches!(err, UmaError::InvalidRequest(_)));
    }

    // upstream: uma-grant §3.2.1 — resource_id is required per element.
    #[test]
    fn mint_rejects_missing_resource_id() {
        let store = PermissionTicketStore::new();
        let err = store.mint("r1", vec![perm("", &["x"])], None, 60).unwrap_err();
        assert!(matches!(err, UmaError::InvalidRequest(_)));
    }

    // upstream: uma-grant §3.3 — ticket is one-shot; consuming twice fails.
    #[test]
    fn consume_is_one_shot() {
        let store = PermissionTicketStore::new();
        let t = store.mint("r1", vec![perm("rid-1", &["view"])], None, 60).unwrap();
        store.consume(&t.ticket).unwrap();
        let err = store.consume(&t.ticket).unwrap_err();
        assert_eq!(err, UmaError::InvalidGrant);
    }

    // upstream: uma-grant §3.2.2 — expired ticket cannot be redeemed.
    #[test]
    fn expired_ticket_fails_consume() {
        let store = PermissionTicketStore::new();
        let t = store
            .mint("r1", vec![perm("rid-1", &["view"])], None, /*ttl=*/ -1)
            .unwrap();
        let err = store.consume(&t.ticket).unwrap_err();
        assert_eq!(err, UmaError::InvalidGrant);
    }
}
