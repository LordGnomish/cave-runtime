// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vulnerability audit store + state machine.
//!
//! Mirrors `model/Analysis` + `AnalysisState` + `AnalysisComment` and the
//! `AnalysisRequest → AnalysisResponse` audit flow served by
//! `resources/v1/AnalysisResource`.

use crate::error::{Error, Result};
use crate::models::{AnalysisJustification, AnalysisResponse, AnalysisState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub component: Uuid,
    pub vulnerability: Uuid,
    pub state: AnalysisState,
    pub justification: AnalysisJustification,
    pub response: AnalysisResponse,
    pub details: Option<String>,
    pub suppressed: bool,
    pub last_changed: DateTime<Utc>,
    pub comments: Vec<AnalysisComment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisComment {
    pub commenter: String,
    pub timestamp: DateTime<Utc>,
    pub comment: String,
}

#[derive(Default)]
pub struct AuditStore {
    entries: RwLock<HashMap<(Uuid, Uuid), Analysis>>,
}

impl AuditStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    pub fn upsert(
        &self,
        component: Uuid,
        vulnerability: Uuid,
        state: AnalysisState,
    ) -> Analysis {
        let mut guard = self.entries.write().unwrap();
        let key = (component, vulnerability);
        let now = Utc::now();
        let suppressed_default = matches!(
            state,
            AnalysisState::Resolved | AnalysisState::FalsePositive | AnalysisState::NotAffected
        );
        if let Some(prev) = guard.get(&key).cloned() {
            let updated = Analysis {
                state,
                suppressed: suppressed_default || prev.suppressed,
                last_changed: now,
                ..prev
            };
            guard.insert(key, updated.clone());
            updated
        } else {
            let a = Analysis {
                component,
                vulnerability,
                state,
                justification: AnalysisJustification::NotSet,
                response: AnalysisResponse::NotSet,
                details: None,
                suppressed: suppressed_default,
                last_changed: now,
                comments: Vec::new(),
            };
            guard.insert(key, a.clone());
            a
        }
    }

    pub fn set_state(
        &self,
        component: Uuid,
        vulnerability: Uuid,
        state: AnalysisState,
    ) -> Result<Analysis> {
        let mut guard = self.entries.write().unwrap();
        let a = guard
            .get_mut(&(component, vulnerability))
            .ok_or_else(|| Error::NotFound("analysis".into()))?;
        Self::transition(a, state)?;
        a.last_changed = Utc::now();
        Ok(a.clone())
    }

    /// Only legal upstream transitions are honoured.  See
    /// `AnalysisState.java` Javadoc: resolved/false-positive entries cannot
    /// revert to in-triage without an explicit comment.
    fn transition(a: &mut Analysis, next: AnalysisState) -> Result<()> {
        use AnalysisState::*;
        let illegal = matches!(
            (a.state, next),
            (Resolved, InTriage)
                | (FalsePositive, InTriage)
                | (NotAffected, InTriage)
        );
        if illegal {
            return Err(Error::Invalid(format!(
                "illegal audit transition {:?} → {:?}",
                a.state, next
            )));
        }
        a.state = next;
        if next == Resolved || next == FalsePositive || next == NotAffected {
            a.suppressed = true;
        } else if next == Exploitable || next == InTriage {
            a.suppressed = false;
        }
        Ok(())
    }

    pub fn suppress(&self, component: Uuid, vulnerability: Uuid, value: bool) -> Result<Analysis> {
        let mut guard = self.entries.write().unwrap();
        let a = guard
            .get_mut(&(component, vulnerability))
            .ok_or_else(|| Error::NotFound("analysis".into()))?;
        a.suppressed = value;
        a.last_changed = Utc::now();
        Ok(a.clone())
    }

    pub fn add_comment(
        &self,
        component: Uuid,
        vulnerability: Uuid,
        commenter: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Analysis> {
        let mut guard = self.entries.write().unwrap();
        let a = guard
            .get_mut(&(component, vulnerability))
            .ok_or_else(|| Error::NotFound("analysis".into()))?;
        a.comments.push(AnalysisComment {
            commenter: commenter.into(),
            timestamp: Utc::now(),
            comment: text.into(),
        });
        Ok(a.clone())
    }

    pub fn get(&self, component: Uuid, vulnerability: Uuid) -> Option<Analysis> {
        self.entries
            .read()
            .unwrap()
            .get(&(component, vulnerability))
            .cloned()
    }

    pub fn for_component(&self, component: Uuid) -> Vec<Analysis> {
        self.entries
            .read()
            .unwrap()
            .iter()
            .filter(|((c, _), _)| *c == component)
            .map(|(_, v)| v.clone())
            .collect()
    }

    pub fn is_suppressed(&self, component: Uuid, vulnerability: Uuid) -> bool {
        self.get(component, vulnerability)
            .map(|a| a.suppressed)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_creates_then_updates() {
        let s = AuditStore::new();
        let c = Uuid::new_v4();
        let v = Uuid::new_v4();
        let a1 = s.upsert(c, v, AnalysisState::InTriage);
        assert_eq!(a1.state, AnalysisState::InTriage);
        let a2 = s.upsert(c, v, AnalysisState::Exploitable);
        assert_eq!(a2.state, AnalysisState::Exploitable);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn legal_transition_intriage_to_exploitable() {
        let s = AuditStore::new();
        let c = Uuid::new_v4();
        let v = Uuid::new_v4();
        s.upsert(c, v, AnalysisState::InTriage);
        s.set_state(c, v, AnalysisState::Exploitable).unwrap();
    }

    #[test]
    fn illegal_resolved_to_intriage_blocked() {
        let s = AuditStore::new();
        let c = Uuid::new_v4();
        let v = Uuid::new_v4();
        s.upsert(c, v, AnalysisState::Resolved);
        let err = s.set_state(c, v, AnalysisState::InTriage).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn resolution_auto_suppresses() {
        let s = AuditStore::new();
        let c = Uuid::new_v4();
        let v = Uuid::new_v4();
        s.upsert(c, v, AnalysisState::InTriage);
        s.set_state(c, v, AnalysisState::FalsePositive).unwrap();
        assert!(s.is_suppressed(c, v));
    }

    #[test]
    fn add_comment_persists() {
        let s = AuditStore::new();
        let c = Uuid::new_v4();
        let v = Uuid::new_v4();
        s.upsert(c, v, AnalysisState::InTriage);
        s.add_comment(c, v, "alice", "triaging").unwrap();
        let back = s.get(c, v).unwrap();
        assert_eq!(back.comments.len(), 1);
        assert_eq!(back.comments[0].commenter, "alice");
    }

    #[test]
    fn for_component_filters_by_component() {
        let s = AuditStore::new();
        let c1 = Uuid::new_v4();
        let c2 = Uuid::new_v4();
        let v = Uuid::new_v4();
        s.upsert(c1, v, AnalysisState::InTriage);
        s.upsert(c2, v, AnalysisState::InTriage);
        assert_eq!(s.for_component(c1).len(), 1);
    }

    #[test]
    fn suppress_toggle_persists() {
        let s = AuditStore::new();
        let c = Uuid::new_v4();
        let v = Uuid::new_v4();
        s.upsert(c, v, AnalysisState::InTriage);
        s.suppress(c, v, true).unwrap();
        assert!(s.is_suppressed(c, v));
        s.suppress(c, v, false).unwrap();
        assert!(!s.is_suppressed(c, v));
    }
}
