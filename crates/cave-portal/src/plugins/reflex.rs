// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Reflex Engine console — action approval workflow.
//!
//! Reflex actions are platform-side automations that require human approval
//! before executing (e.g., "promote canary to 100%", "rotate secret",
//! "drain node"). The console lets the operator see pending requests,
//! approve, deny, or escalate.

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Pending,
    Approved,
    Denied,
    Executing,
    Succeeded,
    Failed,
    Expired,
}

impl ActionStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ActionStatus::Denied
                | ActionStatus::Succeeded
                | ActionStatus::Failed
                | ActionStatus::Expired
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub id: String,
    pub tenant: String,
    pub kind: String,
    pub summary: String,
    pub severity: Severity,
    pub status: ActionStatus,
    pub requested_by: String,
    pub approved_by: Option<String>,
    pub denied_by: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
    pub history: Vec<DecisionEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionEvent {
    pub at: String,
    pub actor: String,
    pub action: DecisionAction,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Created,
    Approved,
    Denied,
    Escalated,
    Expired,
    Executed,
    Failed,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReflexError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("action already terminal: {0:?}")]
    Terminal(ActionStatus),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("approver cannot also be requester")]
    SelfApproval,
    #[error("severity {0:?} requires admin")]
    AdminOnly(Severity),
}

#[derive(Debug, Default)]
pub struct ReflexConsole {
    actions: Vec<ActionRequest>,
}

impl ReflexConsole {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(
        &mut self,
        tenant: impl Into<String>,
        kind: impl Into<String>,
        summary: impl Into<String>,
        severity: Severity,
        requester: impl Into<String>,
    ) -> &ActionRequest {
        let id = format!("act-{:06}", self.actions.len() + 1);
        let requester = requester.into();
        let action = ActionRequest {
            id: id.clone(),
            tenant: tenant.into(),
            kind: kind.into(),
            summary: summary.into(),
            severity,
            status: ActionStatus::Pending,
            requested_by: requester.clone(),
            approved_by: None,
            denied_by: None,
            created_at: "1970-01-01T00:00:00Z".into(),
            decided_at: None,
            history: vec![DecisionEvent {
                at: "1970-01-01T00:00:00Z".into(),
                actor: requester,
                action: DecisionAction::Created,
                note: String::new(),
            }],
        };
        self.actions.push(action);
        self.actions.last().unwrap()
    }

    fn find_mut(&mut self, id: &str) -> Result<&mut ActionRequest, ReflexError> {
        self.actions
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| ReflexError::NotFound(id.into()))
    }

    pub fn find(&self, id: &str) -> Option<&ActionRequest> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn approve(
        &mut self,
        persona: ViewPersona,
        id: &str,
        approver: &str,
        note: &str,
    ) -> Result<&ActionRequest, ReflexError> {
        if !matches!(persona, ViewPersona::Operator | ViewPersona::Admin) {
            return Err(ReflexError::Forbidden("only operator/admin can approve"));
        }
        let action = self.find_mut(id)?;
        if action.status.is_terminal() || action.status == ActionStatus::Approved {
            return Err(ReflexError::Terminal(action.status));
        }
        if action.requested_by == approver {
            return Err(ReflexError::SelfApproval);
        }
        if action.severity == Severity::Critical && persona != ViewPersona::Admin {
            return Err(ReflexError::AdminOnly(Severity::Critical));
        }
        action.status = ActionStatus::Approved;
        action.approved_by = Some(approver.to_string());
        action.decided_at = Some("1970-01-01T00:00:00Z".into());
        action.history.push(DecisionEvent {
            at: "1970-01-01T00:00:00Z".into(),
            actor: approver.into(),
            action: DecisionAction::Approved,
            note: note.into(),
        });
        Ok(action)
    }

    pub fn deny(
        &mut self,
        persona: ViewPersona,
        id: &str,
        denier: &str,
        note: &str,
    ) -> Result<&ActionRequest, ReflexError> {
        if !matches!(persona, ViewPersona::Operator | ViewPersona::Admin) {
            return Err(ReflexError::Forbidden("only operator/admin can deny"));
        }
        let action = self.find_mut(id)?;
        if action.status.is_terminal() {
            return Err(ReflexError::Terminal(action.status));
        }
        action.status = ActionStatus::Denied;
        action.denied_by = Some(denier.to_string());
        action.decided_at = Some("1970-01-01T00:00:00Z".into());
        action.history.push(DecisionEvent {
            at: "1970-01-01T00:00:00Z".into(),
            actor: denier.into(),
            action: DecisionAction::Denied,
            note: note.into(),
        });
        Ok(action)
    }

    pub fn record_execution(
        &mut self,
        id: &str,
        success: bool,
    ) -> Result<&ActionRequest, ReflexError> {
        let action = self.find_mut(id)?;
        if action.status != ActionStatus::Approved && action.status != ActionStatus::Executing {
            return Err(ReflexError::Terminal(action.status));
        }
        action.status = if success {
            ActionStatus::Succeeded
        } else {
            ActionStatus::Failed
        };
        action.history.push(DecisionEvent {
            at: "1970-01-01T00:00:00Z".into(),
            actor: "system".into(),
            action: if success {
                DecisionAction::Executed
            } else {
                DecisionAction::Failed
            },
            note: String::new(),
        });
        Ok(action)
    }

    pub fn pending(&self) -> Vec<&ActionRequest> {
        self.actions
            .iter()
            .filter(|a| a.status == ActionStatus::Pending)
            .collect()
    }

    pub fn for_tenant(&self, tenant: &str) -> Vec<&ActionRequest> {
        self.actions.iter().filter(|a| a.tenant == tenant).collect()
    }

    pub fn count(&self) -> usize {
        self.actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console_with_pending() -> ReflexConsole {
        let mut c = ReflexConsole::new();
        c.submit(
            "acme",
            "rotate-secret",
            "rotate db creds",
            Severity::Medium,
            "alice",
        );
        c
    }

    #[test]
    fn submit_creates_pending() {
        let c = console_with_pending();
        let pending = c.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status, ActionStatus::Pending);
    }

    #[test]
    fn submit_emits_created_event() {
        let c = console_with_pending();
        let action = &c.actions[0];
        assert_eq!(action.history.len(), 1);
        assert_eq!(action.history[0].action, DecisionAction::Created);
    }

    #[test]
    fn approve_tenant_persona_denied() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        let err = c
            .approve(ViewPersona::Tenant, &id, "bob", "lgtm")
            .unwrap_err();
        assert!(matches!(err, ReflexError::Forbidden(_)));
    }

    #[test]
    fn approve_succeeds_for_operator() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        let a = c
            .approve(ViewPersona::Operator, &id, "bob", "lgtm")
            .unwrap();
        assert_eq!(a.status, ActionStatus::Approved);
        assert_eq!(a.approved_by.as_deref(), Some("bob"));
    }

    #[test]
    fn approve_self_rejected() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        let err = c
            .approve(ViewPersona::Operator, &id, "alice", "self")
            .unwrap_err();
        assert_eq!(err, ReflexError::SelfApproval);
    }

    #[test]
    fn approve_critical_requires_admin() {
        let mut c = ReflexConsole::new();
        c.submit("acme", "drain", "drain node", Severity::Critical, "alice");
        let id = c.actions[0].id.clone();
        let err = c
            .approve(ViewPersona::Operator, &id, "bob", "go")
            .unwrap_err();
        assert!(matches!(err, ReflexError::AdminOnly(Severity::Critical)));
    }

    #[test]
    fn approve_critical_admin_succeeds() {
        let mut c = ReflexConsole::new();
        c.submit("acme", "drain", "drain node", Severity::Critical, "alice");
        let id = c.actions[0].id.clone();
        let a = c.approve(ViewPersona::Admin, &id, "bob", "go").unwrap();
        assert_eq!(a.status, ActionStatus::Approved);
    }

    #[test]
    fn approve_terminal_rejected() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        c.deny(ViewPersona::Operator, &id, "bob", "no").unwrap();
        let err = c
            .approve(ViewPersona::Operator, &id, "bob", "x")
            .unwrap_err();
        assert!(matches!(err, ReflexError::Terminal(_)));
    }

    #[test]
    fn deny_persona_check() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        let err = c.deny(ViewPersona::Tenant, &id, "bob", "no").unwrap_err();
        assert!(matches!(err, ReflexError::Forbidden(_)));
    }

    #[test]
    fn deny_records_event() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        c.deny(ViewPersona::Operator, &id, "bob", "policy").unwrap();
        let a = c.find(&id).unwrap();
        assert_eq!(a.status, ActionStatus::Denied);
        assert_eq!(a.denied_by.as_deref(), Some("bob"));
        assert_eq!(a.history.last().unwrap().action, DecisionAction::Denied);
    }

    #[test]
    fn record_execution_success() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        c.approve(ViewPersona::Operator, &id, "bob", "ok").unwrap();
        let a = c.record_execution(&id, true).unwrap();
        assert_eq!(a.status, ActionStatus::Succeeded);
    }

    #[test]
    fn record_execution_failure() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        c.approve(ViewPersona::Operator, &id, "bob", "ok").unwrap();
        let a = c.record_execution(&id, false).unwrap();
        assert_eq!(a.status, ActionStatus::Failed);
    }

    #[test]
    fn record_execution_without_approval_rejected() {
        let mut c = console_with_pending();
        let id = c.actions[0].id.clone();
        let err = c.record_execution(&id, true).unwrap_err();
        assert!(matches!(err, ReflexError::Terminal(_)));
    }

    #[test]
    fn pending_filters_by_status() {
        let mut c = ReflexConsole::new();
        c.submit("acme", "k1", "s", Severity::Low, "alice");
        c.submit("acme", "k2", "s", Severity::Low, "alice");
        let id = c.actions[0].id.clone();
        c.approve(ViewPersona::Operator, &id, "bob", "ok").unwrap();
        assert_eq!(c.pending().len(), 1);
    }

    #[test]
    fn for_tenant_filters() {
        let mut c = ReflexConsole::new();
        c.submit("acme", "k", "s", Severity::Low, "alice");
        c.submit("globex", "k", "s", Severity::Low, "alice");
        assert_eq!(c.for_tenant("acme").len(), 1);
        assert_eq!(c.for_tenant("globex").len(), 1);
    }

    #[test]
    fn count_tracks_total() {
        let mut c = ReflexConsole::new();
        for _ in 0..3 {
            c.submit("acme", "k", "s", Severity::Low, "alice");
        }
        assert_eq!(c.count(), 3);
    }

    #[test]
    fn find_returns_action() {
        let c = console_with_pending();
        let id = c.actions[0].id.clone();
        assert!(c.find(&id).is_some());
        assert!(c.find("ghost").is_none());
    }

    #[test]
    fn unique_ids() {
        let mut c = ReflexConsole::new();
        let id1 = c.submit("a", "k", "s", Severity::Low, "x").id.clone();
        let id2 = c.submit("a", "k", "s", Severity::Low, "x").id.clone();
        assert_ne!(id1, id2);
    }

    #[test]
    fn status_terminal_set() {
        for s in [
            ActionStatus::Denied,
            ActionStatus::Succeeded,
            ActionStatus::Failed,
            ActionStatus::Expired,
        ] {
            assert!(s.is_terminal());
        }
        for s in [
            ActionStatus::Pending,
            ActionStatus::Approved,
            ActionStatus::Executing,
        ] {
            assert!(!s.is_terminal());
        }
    }

    #[test]
    fn severity_serializes_snake_case() {
        let s = serde_json::to_string(&Severity::Critical).unwrap();
        assert_eq!(s, "\"critical\"");
    }
}
