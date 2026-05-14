// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant lifecycle — suspend/resume support for `cavectl tenant suspend|resume`.
//!
//! ADR-012 v7 destekleyici: tenant operasyonlarında suspend reason zorunlu, audit'e
//! `LifecycleEvent` kaydı düşer. Two-person rule yok burada (approval modülü ayrı).

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantLifecycleState {
    Active,
    Suspended,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantRecord {
    pub tenant_id: String,
    pub display_name: String,
    pub state: TenantLifecycleState,
    pub suspend_reason: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub tenant_id: String,
    pub from: TenantLifecycleState,
    pub to: TenantLifecycleState,
    pub actor: String,
    pub reason: Option<String>,
    pub at: DateTime<Utc>,
}

#[async_trait]
pub trait TenantBackend: Send + Sync {
    async fn get(&self, tenant_id: &str) -> Result<TenantRecord>;
    async fn suspend(&self, tenant_id: &str, actor: &str, reason: &str) -> Result<TenantRecord>;
    async fn resume(&self, tenant_id: &str, actor: &str) -> Result<TenantRecord>;
    async fn events(&self, tenant_id: &str) -> Result<Vec<LifecycleEvent>>;
}

#[derive(Default)]
pub struct InMemoryTenantBackend {
    inner: Arc<RwLock<TenantState>>,
}

#[derive(Default)]
struct TenantState {
    tenants: HashMap<String, TenantRecord>,
    events: Vec<LifecycleEvent>,
}

impl InMemoryTenantBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, record: TenantRecord) {
        let mut s = self.inner.write();
        s.tenants.insert(record.tenant_id.clone(), record);
    }
}

#[async_trait]
impl TenantBackend for InMemoryTenantBackend {
    async fn get(&self, tenant_id: &str) -> Result<TenantRecord> {
        self.inner
            .read()
            .tenants
            .get(tenant_id)
            .cloned()
            .ok_or_else(|| anyhow!("tenant not found: {tenant_id}"))
    }

    async fn suspend(&self, tenant_id: &str, actor: &str, reason: &str) -> Result<TenantRecord> {
        if reason.trim().is_empty() {
            return Err(anyhow!("suspend reason required"));
        }
        let mut s = self.inner.write();
        let rec = s
            .tenants
            .get_mut(tenant_id)
            .ok_or_else(|| anyhow!("tenant not found: {tenant_id}"))?;
        if rec.state == TenantLifecycleState::Terminated {
            return Err(anyhow!("cannot suspend terminated tenant"));
        }
        if rec.state == TenantLifecycleState::Suspended {
            return Err(anyhow!("tenant already suspended"));
        }
        let from = rec.state;
        rec.state = TenantLifecycleState::Suspended;
        rec.suspend_reason = Some(reason.to_string());
        rec.updated_at = Utc::now();
        let cloned = rec.clone();
        s.events.push(LifecycleEvent {
            tenant_id: tenant_id.to_string(),
            from,
            to: TenantLifecycleState::Suspended,
            actor: actor.to_string(),
            reason: Some(reason.to_string()),
            at: cloned.updated_at,
        });
        Ok(cloned)
    }

    async fn resume(&self, tenant_id: &str, actor: &str) -> Result<TenantRecord> {
        let mut s = self.inner.write();
        let rec = s
            .tenants
            .get_mut(tenant_id)
            .ok_or_else(|| anyhow!("tenant not found: {tenant_id}"))?;
        if rec.state != TenantLifecycleState::Suspended {
            return Err(anyhow!("tenant not suspended (state={:?})", rec.state));
        }
        let from = rec.state;
        rec.state = TenantLifecycleState::Active;
        rec.suspend_reason = None;
        rec.updated_at = Utc::now();
        let cloned = rec.clone();
        s.events.push(LifecycleEvent {
            tenant_id: tenant_id.to_string(),
            from,
            to: TenantLifecycleState::Active,
            actor: actor.to_string(),
            reason: None,
            at: cloned.updated_at,
        });
        Ok(cloned)
    }

    async fn events(&self, tenant_id: &str) -> Result<Vec<LifecycleEvent>> {
        Ok(self
            .inner
            .read()
            .events
            .iter()
            .filter(|e| e.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tenant_id: &str, state: TenantLifecycleState) -> TenantRecord {
        TenantRecord {
            tenant_id: tenant_id.to_string(),
            display_name: format!("{tenant_id} display"),
            state,
            suspend_reason: None,
            updated_at: Utc::now(),
        }
    }

    /// cite: ADR-012 v7 — tenant suspend lifecycle, default state Active
    #[tokio::test]
    async fn tenant_acme_seed_returns_active() {
        let tenant_id = "acme";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        let got = b.get(tenant_id).await.unwrap();
        assert_eq!(got.tenant_id, tenant_id);
        assert_eq!(got.state, TenantLifecycleState::Active);
    }

    /// cite: ADR-012 v7 — suspend transitions Active → Suspended
    #[tokio::test]
    async fn tenant_acme_suspend_transitions_state() {
        let tenant_id = "acme";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        let after = b.suspend(tenant_id, "burak", "billing-overdue").await.unwrap();
        assert_eq!(after.state, TenantLifecycleState::Suspended);
        assert_eq!(after.suspend_reason.as_deref(), Some("billing-overdue"));
    }

    /// cite: ADR-012 v7 — suspend requires non-empty reason
    #[tokio::test]
    async fn tenant_globex_suspend_empty_reason_rejected() {
        let tenant_id = "globex";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        let err = b.suspend(tenant_id, "burak", "   ").await.unwrap_err();
        assert!(err.to_string().contains("reason required"));
    }

    /// cite: ADR-012 v7 — already-suspended tenant cannot be re-suspended
    #[tokio::test]
    async fn tenant_initech_double_suspend_rejected() {
        let tenant_id = "initech";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        b.suspend(tenant_id, "burak", "audit-hold").await.unwrap();
        let err = b.suspend(tenant_id, "burak", "audit-hold").await.unwrap_err();
        assert!(err.to_string().contains("already suspended"));
    }

    /// cite: ADR-012 v7 — terminated tenant cannot transition to suspended
    #[tokio::test]
    async fn tenant_dunder_terminated_cannot_suspend() {
        let tenant_id = "dunder";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Terminated));
        let err = b.suspend(tenant_id, "burak", "x").await.unwrap_err();
        assert!(err.to_string().contains("terminated"));
    }

    /// cite: ADR-012 v7 — resume transitions Suspended → Active and clears reason
    #[tokio::test]
    async fn tenant_acme_resume_clears_reason() {
        let tenant_id = "acme";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        b.suspend(tenant_id, "burak", "billing-overdue").await.unwrap();
        let after = b.resume(tenant_id, "burak").await.unwrap();
        assert_eq!(after.state, TenantLifecycleState::Active);
        assert!(after.suspend_reason.is_none());
    }

    /// cite: ADR-012 v7 — resume rejects when not suspended
    #[tokio::test]
    async fn tenant_globex_resume_when_active_rejected() {
        let tenant_id = "globex";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        let err = b.resume(tenant_id, "burak").await.unwrap_err();
        assert!(err.to_string().contains("not suspended"));
    }

    /// cite: ADR-012 v7 — events log captures suspend + resume in order
    #[tokio::test]
    async fn tenant_acme_events_record_full_cycle() {
        let tenant_id = "acme";
        let b = InMemoryTenantBackend::new();
        b.seed(record(tenant_id, TenantLifecycleState::Active));
        b.suspend(tenant_id, "burak", "audit-hold").await.unwrap();
        b.resume(tenant_id, "burak").await.unwrap();
        let events = b.events(tenant_id).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].to, TenantLifecycleState::Suspended);
        assert_eq!(events[1].to, TenantLifecycleState::Active);
        assert_eq!(events[0].reason.as_deref(), Some("audit-hold"));
        assert!(events[1].reason.is_none());
    }

    /// cite: ADR-012 v7 — get on missing tenant returns descriptive error
    #[tokio::test]
    async fn tenant_unknown_get_errors() {
        let tenant_id = "ghost";
        let b = InMemoryTenantBackend::new();
        let err = b.get(tenant_id).await.unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }
}
