// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Reflex Engine — escalation chain + SLA timer + bulk approve.
//!
//! Layered atop [`super::reflex`]. Adds the operational features that make
//! the engine usable at scale — pending requests escalate up the chain when
//! their SLA expires, operators bulk-approve a queue, and the dashboard
//! shows breach counts per chain rung.

use super::ViewPersona;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscalationRung {
    pub level: u8,
    pub name: String,
    pub approvers: Vec<String>,
    pub sla_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscalationChain {
    pub id: String,
    pub tenant: String,
    pub rungs: Vec<EscalationRung>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdvancedReflexError {
    #[error("chain {0:?} not found")]
    ChainNotFound(String),
    #[error("chain has no rungs")]
    EmptyChain,
    #[error("rung level must increment by 1")]
    NonContiguousLevels,
    #[error("rung sla must be > 0")]
    InvalidSla,
    #[error("rung must list at least one approver")]
    NoApprovers,
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("action not pending: {0:?}")]
    NotPending(String),
    #[error("approver {approver:?} not on rung {rung}")]
    UnauthorizedApprover { approver: String, rung: u8 },
}

impl EscalationChain {
    pub fn new(id: impl Into<String>, tenant: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tenant: tenant.into(),
            rungs: Vec::new(),
        }
    }

    pub fn add_rung(&mut self, rung: EscalationRung) -> Result<(), AdvancedReflexError> {
        if rung.sla_minutes == 0 {
            return Err(AdvancedReflexError::InvalidSla);
        }
        if rung.approvers.is_empty() {
            return Err(AdvancedReflexError::NoApprovers);
        }
        let expected = self.rungs.len() as u8 + 1;
        if rung.level != expected {
            return Err(AdvancedReflexError::NonContiguousLevels);
        }
        self.rungs.push(rung);
        Ok(())
    }

    pub fn validate(&self) -> Result<(), AdvancedReflexError> {
        if self.rungs.is_empty() {
            return Err(AdvancedReflexError::EmptyChain);
        }
        for (i, rung) in self.rungs.iter().enumerate() {
            if rung.level != (i as u8) + 1 {
                return Err(AdvancedReflexError::NonContiguousLevels);
            }
        }
        Ok(())
    }

    pub fn approver_for_level(&self, level: u8, name: &str) -> bool {
        self.rungs
            .iter()
            .find(|r| r.level == level)
            .map(|r| r.approvers.iter().any(|a| a == name))
            .unwrap_or(false)
    }

    pub fn next_level(&self, current: u8) -> Option<u8> {
        let next = current + 1;
        if self.rungs.iter().any(|r| r.level == next) {
            Some(next)
        } else {
            None
        }
    }

    pub fn sla_for_level(&self, level: u8) -> Option<u32> {
        self.rungs.iter().find(|r| r.level == level).map(|r| r.sla_minutes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAction {
    pub id: String,
    pub tenant: String,
    pub chain_id: String,
    pub current_level: u8,
    pub created_unix: u64,
    pub level_entered_unix: u64,
    pub state: PendingState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingState {
    Pending,
    Approved,
    Denied,
    Escalated,
    Expired,
}

#[derive(Debug, Default)]
pub struct ChainRegistry {
    chains: Vec<EscalationChain>,
}

impl ChainRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(
        &mut self,
        persona: ViewPersona,
        chain: EscalationChain,
    ) -> Result<(), AdvancedReflexError> {
        if !matches!(persona, ViewPersona::Admin) {
            return Err(AdvancedReflexError::Forbidden("chain edits are admin-only"));
        }
        chain.validate()?;
        if let Some(idx) = self
            .chains
            .iter()
            .position(|c| c.id == chain.id && c.tenant == chain.tenant)
        {
            self.chains[idx] = chain;
        } else {
            self.chains.push(chain);
        }
        Ok(())
    }

    pub fn find(&self, tenant: &str, id: &str) -> Option<&EscalationChain> {
        self.chains.iter().find(|c| c.tenant == tenant && c.id == id)
    }

    pub fn count(&self) -> usize {
        self.chains.len()
    }
}

#[derive(Debug, Default)]
pub struct ReflexAdvancedConsole {
    pending: Vec<PendingAction>,
}

impl ReflexAdvancedConsole {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(
        &mut self,
        registry: &ChainRegistry,
        tenant: &str,
        chain_id: &str,
        now: u64,
    ) -> Result<&PendingAction, AdvancedReflexError> {
        let chain = registry
            .find(tenant, chain_id)
            .ok_or_else(|| AdvancedReflexError::ChainNotFound(chain_id.into()))?;
        chain.validate()?;
        let id = format!("act-{:08}", self.pending.len() + 1);
        self.pending.push(PendingAction {
            id,
            tenant: tenant.into(),
            chain_id: chain_id.into(),
            current_level: 1,
            created_unix: now,
            level_entered_unix: now,
            state: PendingState::Pending,
        });
        Ok(self.pending.last().unwrap())
    }

    pub fn approve(
        &mut self,
        registry: &ChainRegistry,
        id: &str,
        approver: &str,
    ) -> Result<&PendingAction, AdvancedReflexError> {
        let action = self
            .pending
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| AdvancedReflexError::NotPending(id.into()))?;
        if action.state != PendingState::Pending {
            return Err(AdvancedReflexError::NotPending(id.into()));
        }
        let chain = registry
            .find(&action.tenant, &action.chain_id)
            .ok_or_else(|| AdvancedReflexError::ChainNotFound(action.chain_id.clone()))?;
        if !chain.approver_for_level(action.current_level, approver) {
            return Err(AdvancedReflexError::UnauthorizedApprover {
                approver: approver.into(),
                rung: action.current_level,
            });
        }
        action.state = PendingState::Approved;
        Ok(&*action)
    }

    pub fn deny(
        &mut self,
        registry: &ChainRegistry,
        id: &str,
        denier: &str,
    ) -> Result<&PendingAction, AdvancedReflexError> {
        let action = self
            .pending
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| AdvancedReflexError::NotPending(id.into()))?;
        if action.state != PendingState::Pending {
            return Err(AdvancedReflexError::NotPending(id.into()));
        }
        let chain = registry
            .find(&action.tenant, &action.chain_id)
            .ok_or_else(|| AdvancedReflexError::ChainNotFound(action.chain_id.clone()))?;
        if !chain.approver_for_level(action.current_level, denier) {
            return Err(AdvancedReflexError::UnauthorizedApprover {
                approver: denier.into(),
                rung: action.current_level,
            });
        }
        action.state = PendingState::Denied;
        Ok(&*action)
    }

    /// Walk all pending actions; for each whose current rung's SLA has
    /// elapsed, advance to the next rung or mark Expired if no further rung.
    /// Returns the number of actions whose level changed.
    pub fn tick_escalations(&mut self, registry: &ChainRegistry, now: u64) -> usize {
        let mut moves = 0;
        for action in self.pending.iter_mut() {
            if action.state != PendingState::Pending {
                continue;
            }
            let chain = match registry.find(&action.tenant, &action.chain_id) {
                Some(c) => c,
                None => continue,
            };
            let sla = match chain.sla_for_level(action.current_level) {
                Some(s) => s,
                None => continue,
            };
            let elapsed_min = (now.saturating_sub(action.level_entered_unix)) / 60;
            if elapsed_min < sla as u64 {
                continue;
            }
            // SLA elapsed — escalate or expire
            match chain.next_level(action.current_level) {
                Some(next) => {
                    action.current_level = next;
                    action.level_entered_unix = now;
                    action.state = PendingState::Escalated;
                    // Re-mark Pending so the new rung can act on it.
                    action.state = PendingState::Pending;
                    moves += 1;
                }
                None => {
                    action.state = PendingState::Expired;
                    moves += 1;
                }
            }
        }
        moves
    }

    /// Bulk-approve all pending actions for which `approver` is on the
    /// current rung. Returns the count approved.
    pub fn bulk_approve(
        &mut self,
        registry: &ChainRegistry,
        approver: &str,
    ) -> usize {
        let mut n = 0;
        for action in self.pending.iter_mut() {
            if action.state != PendingState::Pending {
                continue;
            }
            let chain = match registry.find(&action.tenant, &action.chain_id) {
                Some(c) => c,
                None => continue,
            };
            if !chain.approver_for_level(action.current_level, approver) {
                continue;
            }
            action.state = PendingState::Approved;
            n += 1;
        }
        n
    }

    pub fn pending(&self) -> Vec<&PendingAction> {
        self.pending
            .iter()
            .filter(|a| a.state == PendingState::Pending)
            .collect()
    }

    pub fn breach_summary(&self) -> HashMap<u8, u32> {
        let mut acc: HashMap<u8, u32> = HashMap::new();
        for a in self.pending.iter().filter(|a| a.state == PendingState::Expired) {
            *acc.entry(a.current_level).or_insert(0) += 1;
        }
        acc
    }

    pub fn count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rung(level: u8, approver: &str, sla: u32) -> EscalationRung {
        EscalationRung {
            level,
            name: format!("rung-{level}"),
            approvers: vec![approver.into()],
            sla_minutes: sla,
        }
    }

    fn chain_with_rungs() -> EscalationChain {
        let mut c = EscalationChain::new("approval", "acme");
        c.add_rung(rung(1, "alice", 30)).unwrap();
        c.add_rung(rung(2, "bob", 60)).unwrap();
        c
    }

    fn registry_with_chain() -> ChainRegistry {
        let mut r = ChainRegistry::new();
        r.upsert(ViewPersona::Admin, chain_with_rungs()).unwrap();
        r
    }

    #[test]
    fn add_rung_checks_increment() {
        let mut c = EscalationChain::new("c", "t");
        let err = c.add_rung(rung(2, "alice", 30)).unwrap_err();
        assert_eq!(err, AdvancedReflexError::NonContiguousLevels);
    }

    #[test]
    fn add_rung_zero_sla_rejected() {
        let mut c = EscalationChain::new("c", "t");
        let err = c.add_rung(rung(1, "alice", 0)).unwrap_err();
        assert_eq!(err, AdvancedReflexError::InvalidSla);
    }

    #[test]
    fn add_rung_no_approvers_rejected() {
        let mut c = EscalationChain::new("c", "t");
        let mut r = rung(1, "x", 30);
        r.approvers.clear();
        let err = c.add_rung(r).unwrap_err();
        assert_eq!(err, AdvancedReflexError::NoApprovers);
    }

    #[test]
    fn validate_empty_chain_rejected() {
        let c = EscalationChain::new("c", "t");
        let err = c.validate().unwrap_err();
        assert_eq!(err, AdvancedReflexError::EmptyChain);
    }

    #[test]
    fn validate_chain_with_rungs_ok() {
        let c = chain_with_rungs();
        assert!(c.validate().is_ok());
    }

    #[test]
    fn approver_for_level_finds_listed() {
        let c = chain_with_rungs();
        assert!(c.approver_for_level(1, "alice"));
        assert!(c.approver_for_level(2, "bob"));
        assert!(!c.approver_for_level(1, "bob"));
    }

    #[test]
    fn approver_for_level_unknown_returns_false() {
        let c = chain_with_rungs();
        assert!(!c.approver_for_level(99, "alice"));
    }

    #[test]
    fn next_level_advances() {
        let c = chain_with_rungs();
        assert_eq!(c.next_level(1), Some(2));
        assert_eq!(c.next_level(2), None);
    }

    #[test]
    fn sla_for_level_lookup() {
        let c = chain_with_rungs();
        assert_eq!(c.sla_for_level(1), Some(30));
        assert_eq!(c.sla_for_level(2), Some(60));
        assert_eq!(c.sla_for_level(99), None);
    }

    #[test]
    fn registry_upsert_admin_only() {
        let mut r = ChainRegistry::new();
        let err = r.upsert(ViewPersona::Operator, chain_with_rungs()).unwrap_err();
        assert!(matches!(err, AdvancedReflexError::Forbidden(_)));
    }

    #[test]
    fn registry_upsert_admin_succeeds() {
        let mut r = ChainRegistry::new();
        r.upsert(ViewPersona::Admin, chain_with_rungs()).unwrap();
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn registry_upsert_replaces_same_id_tenant() {
        let mut r = ChainRegistry::new();
        r.upsert(ViewPersona::Admin, chain_with_rungs()).unwrap();
        r.upsert(ViewPersona::Admin, chain_with_rungs()).unwrap();
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn registry_upsert_invalid_chain_rejected() {
        let mut r = ChainRegistry::new();
        let c = EscalationChain::new("empty", "acme");
        let err = r.upsert(ViewPersona::Admin, c).unwrap_err();
        assert_eq!(err, AdvancedReflexError::EmptyChain);
    }

    #[test]
    fn submit_unknown_chain_errors() {
        let r = ChainRegistry::new();
        let mut c = ReflexAdvancedConsole::new();
        let err = c.submit(&r, "acme", "ghost", 0).unwrap_err();
        assert!(matches!(err, AdvancedReflexError::ChainNotFound(_)));
    }

    #[test]
    fn submit_creates_pending_at_level_one() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let a = c.submit(&r, "acme", "approval", 1000).unwrap();
        assert_eq!(a.current_level, 1);
        assert_eq!(a.state, PendingState::Pending);
    }

    #[test]
    fn approve_succeeds_for_listed() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 1000).unwrap().id.clone();
        let a = c.approve(&r, &id, "alice").unwrap();
        assert_eq!(a.state, PendingState::Approved);
    }

    #[test]
    fn approve_unauthorized_at_level() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 1000).unwrap().id.clone();
        let err = c.approve(&r, &id, "bob").unwrap_err();
        assert!(matches!(err, AdvancedReflexError::UnauthorizedApprover { .. }));
    }

    #[test]
    fn approve_already_terminal_rejected() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 1000).unwrap().id.clone();
        c.approve(&r, &id, "alice").unwrap();
        let err = c.approve(&r, &id, "alice").unwrap_err();
        assert!(matches!(err, AdvancedReflexError::NotPending(_)));
    }

    #[test]
    fn deny_records_state() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 1000).unwrap().id.clone();
        let a = c.deny(&r, &id, "alice").unwrap();
        assert_eq!(a.state, PendingState::Denied);
    }

    #[test]
    fn deny_unauthorized() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 1000).unwrap().id.clone();
        let err = c.deny(&r, &id, "carol").unwrap_err();
        assert!(matches!(err, AdvancedReflexError::UnauthorizedApprover { .. }));
    }

    #[test]
    fn tick_escalation_advances_after_sla() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        c.submit(&r, "acme", "approval", 1000).unwrap();
        // sla rung 1 = 30 minutes = 1800 seconds. Tick at 3000 (33min later)
        let moves = c.tick_escalations(&r, 1000 + 31 * 60);
        assert_eq!(moves, 1);
        let a = &c.pending[0];
        assert_eq!(a.current_level, 2);
        assert_eq!(a.state, PendingState::Pending);
    }

    #[test]
    fn tick_escalation_does_not_advance_within_sla() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        c.submit(&r, "acme", "approval", 1000).unwrap();
        let moves = c.tick_escalations(&r, 1000 + 5 * 60);
        assert_eq!(moves, 0);
        let a = &c.pending[0];
        assert_eq!(a.current_level, 1);
    }

    #[test]
    fn tick_escalation_expires_at_top_rung() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        c.submit(&r, "acme", "approval", 0).unwrap();
        // first tick: escalate to level 2
        c.tick_escalations(&r, 31 * 60);
        // second tick: level 2 sla is 60 min — expire
        let moves = c.tick_escalations(&r, 31 * 60 + 61 * 60);
        assert_eq!(moves, 1);
        let a = &c.pending[0];
        assert_eq!(a.state, PendingState::Expired);
    }

    #[test]
    fn tick_escalation_skips_terminal_actions() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 0).unwrap().id.clone();
        c.approve(&r, &id, "alice").unwrap();
        let moves = c.tick_escalations(&r, 100 * 60);
        assert_eq!(moves, 0);
    }

    #[test]
    fn bulk_approve_only_affects_authorized() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        c.submit(&r, "acme", "approval", 0).unwrap();
        c.submit(&r, "acme", "approval", 0).unwrap();
        c.submit(&r, "acme", "approval", 0).unwrap();
        let n = c.bulk_approve(&r, "alice");
        assert_eq!(n, 3);
        assert!(c.pending().is_empty());
    }

    #[test]
    fn bulk_approve_skips_terminal() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 0).unwrap().id.clone();
        c.deny(&r, &id, "alice").unwrap();
        let n = c.bulk_approve(&r, "alice");
        assert_eq!(n, 0);
    }

    #[test]
    fn bulk_approve_skips_unauthorized() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        c.submit(&r, "acme", "approval", 0).unwrap();
        let n = c.bulk_approve(&r, "carol"); // not on rung 1
        assert_eq!(n, 0);
    }

    #[test]
    fn pending_filters_state() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id = c.submit(&r, "acme", "approval", 0).unwrap().id.clone();
        c.submit(&r, "acme", "approval", 0).unwrap();
        c.approve(&r, &id, "alice").unwrap();
        assert_eq!(c.pending().len(), 1);
    }

    #[test]
    fn breach_summary_counts_by_level() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        c.submit(&r, "acme", "approval", 0).unwrap();
        c.tick_escalations(&r, 31 * 60);
        c.tick_escalations(&r, 31 * 60 + 61 * 60);
        let breach = c.breach_summary();
        assert_eq!(breach.get(&2), Some(&1));
    }

    #[test]
    fn submit_assigns_unique_ids() {
        let r = registry_with_chain();
        let mut c = ReflexAdvancedConsole::new();
        let id1 = c.submit(&r, "acme", "approval", 0).unwrap().id.clone();
        let id2 = c.submit(&r, "acme", "approval", 0).unwrap().id.clone();
        assert_ne!(id1, id2);
    }

    #[test]
    fn pending_state_serializes_snake_case() {
        let s = serde_json::to_string(&PendingState::Escalated).unwrap();
        assert_eq!(s, "\"escalated\"");
    }

    #[test]
    fn chain_round_trips_json() {
        let c = chain_with_rungs();
        let s = serde_json::to_string(&c).unwrap();
        let back: EscalationChain = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }
}
