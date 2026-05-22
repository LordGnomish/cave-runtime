// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Environment lifecycle — `cavectl env suspend|resume`.
//!
//! ADR-012 v7 destekleyici (vcluster PR environments). Env tenant'a bağlı,
//! tenant suspended ise env de implicit suspended sayılır. `cascade=true` ile
//! tenant suspend olduğunda tüm env'ler birlikte askıya alınır.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvLifecycleState {
    Active,
    Suspended,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvRecord {
    pub tenant_id: String,
    pub env_id: String,
    pub kind: String,
    pub state: EnvLifecycleState,
    pub suspend_reason: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait EnvBackend: Send + Sync {
    async fn get(&self, tenant_id: &str, env_id: &str) -> Result<EnvRecord>;
    async fn list(&self, tenant_id: &str) -> Result<Vec<EnvRecord>>;
    async fn suspend(
        &self,
        tenant_id: &str,
        env_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<EnvRecord>;
    async fn resume(&self, tenant_id: &str, env_id: &str, actor: &str) -> Result<EnvRecord>;
    async fn cascade_suspend(
        &self,
        tenant_id: &str,
        actor: &str,
        reason: &str,
    ) -> Result<Vec<EnvRecord>>;
}

#[derive(Default)]
pub struct InMemoryEnvBackend {
    inner: Arc<RwLock<HashMap<(String, String), EnvRecord>>>,
}

impl InMemoryEnvBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, record: EnvRecord) {
        let mut s = self.inner.write();
        s.insert((record.tenant_id.clone(), record.env_id.clone()), record);
    }
}

#[async_trait]
impl EnvBackend for InMemoryEnvBackend {
    async fn get(&self, tenant_id: &str, env_id: &str) -> Result<EnvRecord> {
        self.inner
            .read()
            .get(&(tenant_id.to_string(), env_id.to_string()))
            .cloned()
            .ok_or_else(|| anyhow!("env not found: {tenant_id}/{env_id}"))
    }

    async fn list(&self, tenant_id: &str) -> Result<Vec<EnvRecord>> {
        let s = self.inner.read();
        let mut out: Vec<EnvRecord> = s
            .values()
            .filter(|r| r.tenant_id == tenant_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.env_id.cmp(&b.env_id));
        Ok(out)
    }

    async fn suspend(
        &self,
        tenant_id: &str,
        env_id: &str,
        _actor: &str,
        reason: &str,
    ) -> Result<EnvRecord> {
        if reason.trim().is_empty() {
            return Err(anyhow!("suspend reason required"));
        }
        let mut s = self.inner.write();
        let rec = s
            .get_mut(&(tenant_id.to_string(), env_id.to_string()))
            .ok_or_else(|| anyhow!("env not found: {tenant_id}/{env_id}"))?;
        if rec.state == EnvLifecycleState::Archived {
            return Err(anyhow!("cannot suspend archived env"));
        }
        if rec.state == EnvLifecycleState::Suspended {
            return Err(anyhow!("env already suspended"));
        }
        rec.state = EnvLifecycleState::Suspended;
        rec.suspend_reason = Some(reason.to_string());
        rec.updated_at = Utc::now();
        Ok(rec.clone())
    }

    async fn resume(&self, tenant_id: &str, env_id: &str, _actor: &str) -> Result<EnvRecord> {
        let mut s = self.inner.write();
        let rec = s
            .get_mut(&(tenant_id.to_string(), env_id.to_string()))
            .ok_or_else(|| anyhow!("env not found: {tenant_id}/{env_id}"))?;
        if rec.state != EnvLifecycleState::Suspended {
            return Err(anyhow!("env not suspended (state={:?})", rec.state));
        }
        rec.state = EnvLifecycleState::Active;
        rec.suspend_reason = None;
        rec.updated_at = Utc::now();
        Ok(rec.clone())
    }

    async fn cascade_suspend(
        &self,
        tenant_id: &str,
        _actor: &str,
        reason: &str,
    ) -> Result<Vec<EnvRecord>> {
        if reason.trim().is_empty() {
            return Err(anyhow!("cascade reason required"));
        }
        let mut s = self.inner.write();
        let mut changed = vec![];
        for ((t, _), rec) in s.iter_mut() {
            if t != tenant_id {
                continue;
            }
            if rec.state == EnvLifecycleState::Active {
                rec.state = EnvLifecycleState::Suspended;
                rec.suspend_reason = Some(reason.to_string());
                rec.updated_at = Utc::now();
                changed.push(rec.clone());
            }
        }
        changed.sort_by(|a, b| a.env_id.cmp(&b.env_id));
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tenant_id: &str, env_id: &str, state: EnvLifecycleState) -> EnvRecord {
        EnvRecord {
            tenant_id: tenant_id.to_string(),
            env_id: env_id.to_string(),
            kind: "vcluster".to_string(),
            state,
            suspend_reason: None,
            updated_at: Utc::now(),
        }
    }

    /// cite: ADR-012 v7 — env suspend transitions Active → Suspended
    #[tokio::test]
    async fn env_acme_pr42_suspend_transitions() {
        let tenant_id = "acme";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "pr-42", EnvLifecycleState::Active));
        let after = b
            .suspend(tenant_id, "pr-42", "burak", "review-blocked")
            .await
            .unwrap();
        assert_eq!(after.state, EnvLifecycleState::Suspended);
    }

    /// cite: ADR-012 v7 — env suspend rejects empty reason
    #[tokio::test]
    async fn env_globex_pr1_empty_reason_rejected() {
        let tenant_id = "globex";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "pr-1", EnvLifecycleState::Active));
        let err = b.suspend(tenant_id, "pr-1", "burak", "").await.unwrap_err();
        assert!(err.to_string().contains("reason required"));
    }

    /// cite: ADR-012 v7 — archived env cannot be suspended
    #[tokio::test]
    async fn env_initech_archived_cannot_suspend() {
        let tenant_id = "initech";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "stage", EnvLifecycleState::Archived));
        let err = b
            .suspend(tenant_id, "stage", "burak", "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("archived"));
    }

    /// cite: ADR-012 v7 — env resume clears suspend_reason
    #[tokio::test]
    async fn env_acme_pr42_resume_clears_reason() {
        let tenant_id = "acme";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "pr-42", EnvLifecycleState::Active));
        b.suspend(tenant_id, "pr-42", "burak", "review-blocked")
            .await
            .unwrap();
        let after = b.resume(tenant_id, "pr-42", "burak").await.unwrap();
        assert_eq!(after.state, EnvLifecycleState::Active);
        assert!(after.suspend_reason.is_none());
    }

    /// cite: ADR-012 v7 — list returns envs scoped to a single tenant
    #[tokio::test]
    async fn env_list_acme_returns_only_acme_envs() {
        let tenant_id = "acme";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "pr-1", EnvLifecycleState::Active));
        b.seed(record(tenant_id, "pr-2", EnvLifecycleState::Active));
        b.seed(record("globex", "pr-99", EnvLifecycleState::Active));
        let list = b.list(tenant_id).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|r| r.tenant_id == tenant_id));
    }

    /// cite: ADR-012 v7 — cascade_suspend marks all active envs of tenant
    #[tokio::test]
    async fn env_cascade_acme_suspends_all_active_envs() {
        let tenant_id = "acme";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "pr-1", EnvLifecycleState::Active));
        b.seed(record(tenant_id, "pr-2", EnvLifecycleState::Active));
        b.seed(record(tenant_id, "pr-3", EnvLifecycleState::Archived));
        let changed = b
            .cascade_suspend(tenant_id, "burak", "tenant-suspend")
            .await
            .unwrap();
        assert_eq!(changed.len(), 2);
        assert!(
            changed
                .iter()
                .all(|r| r.state == EnvLifecycleState::Suspended)
        );
    }

    /// cite: ADR-012 v7 — cascade does not touch other tenants' envs
    #[tokio::test]
    async fn env_cascade_acme_does_not_affect_globex() {
        let tenant_id = "acme";
        let b = InMemoryEnvBackend::new();
        b.seed(record(tenant_id, "pr-1", EnvLifecycleState::Active));
        b.seed(record("globex", "pr-1", EnvLifecycleState::Active));
        b.cascade_suspend(tenant_id, "burak", "tenant-suspend")
            .await
            .unwrap();
        let globex = b.get("globex", "pr-1").await.unwrap();
        assert_eq!(globex.state, EnvLifecycleState::Active);
    }

    /// cite: ADR-012 v7 — env not found returns descriptive error
    #[tokio::test]
    async fn env_get_unknown_returns_error() {
        let tenant_id = "acme";
        let b = InMemoryEnvBackend::new();
        let err = b.get(tenant_id, "ghost").await.unwrap_err();
        assert!(err.to_string().contains("acme/ghost"));
    }
}
