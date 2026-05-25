// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pooler CRD reconciler.
//!
//! Mirrors CloudNativePG's Pooler — a higher-level CRD that owns a
//! PgBouncer pod fleet for a target [`crate::types::PgInstance`].
//! Where [`crate::pool`] models the in-process connection pool
//! primitives, this module models the *cluster-side* Pooler object:
//! its declarative spec, its observable status, and the reconcile
//! loop that walks the two together.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::types::PoolMode;

#[derive(Debug, thiserror::Error)]
pub enum PoolerError {
    #[error("pooler {0} not found")]
    NotFound(String),
    #[error("pooler {0} already exists")]
    AlreadyExists(String),
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolerPhase {
    Pending,
    Configuring,
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolerSpec {
    /// Target PgInstance id.
    pub instance_id: String,
    /// Desired PgBouncer replica count.
    pub replicas: u32,
    /// PgBouncer pool_mode (Session / Transaction / Statement).
    pub pool_mode: PoolMode,
    /// `default_pool_size` — server-side connections per user/db.
    pub default_pool_size: u32,
    /// `max_client_conn` — total client cap across all pods.
    pub max_client_conn: u32,
    /// `min_pool_size` — warm idle connections kept open.
    pub min_pool_size: u32,
    /// PgBouncer image tag for the pod spec.
    pub image: String,
}

impl PoolerSpec {
    pub fn validate(&self) -> Result<(), PoolerError> {
        if self.replicas == 0 {
            return Err(PoolerError::InvalidSpec("replicas must be >= 1".into()));
        }
        if self.default_pool_size == 0 {
            return Err(PoolerError::InvalidSpec(
                "default_pool_size must be >= 1".into(),
            ));
        }
        if self.max_client_conn < self.default_pool_size {
            return Err(PoolerError::InvalidSpec(
                "max_client_conn must be >= default_pool_size".into(),
            ));
        }
        if self.min_pool_size > self.default_pool_size {
            return Err(PoolerError::InvalidSpec(
                "min_pool_size must be <= default_pool_size".into(),
            ));
        }
        if self.image.is_empty() {
            return Err(PoolerError::InvalidSpec("image must not be empty".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolerCondition {
    pub kind: String, // "Available", "Progressing", "Failed"
    pub status: bool,
    pub reason: String,
    pub message: String,
    pub last_transition: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolerStatus {
    pub phase: PoolerPhase,
    pub ready_replicas: u32,
    pub observed_generation: u64,
    pub conditions: Vec<PoolerCondition>,
    pub last_updated: DateTime<Utc>,
}

impl PoolerStatus {
    fn pending() -> Self {
        Self {
            phase: PoolerPhase::Pending,
            ready_replicas: 0,
            observed_generation: 0,
            conditions: Vec::new(),
            last_updated: Utc::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pooler {
    pub name: String,
    pub spec: PoolerSpec,
    pub status: PoolerStatus,
    pub generation: u64,
}

/// In-memory CRD store + reconciler.
pub struct PoolerManager {
    poolers: Arc<RwLock<HashMap<String, Pooler>>>,
}

impl PoolerManager {
    pub fn new() -> Self {
        Self {
            poolers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create(&self, name: impl Into<String>, spec: PoolerSpec) -> Result<Pooler, PoolerError> {
        spec.validate()?;
        let name = name.into();
        let mut poolers = self.poolers.write().unwrap();
        if poolers.contains_key(&name) {
            return Err(PoolerError::AlreadyExists(name));
        }
        let pooler = Pooler {
            name: name.clone(),
            spec,
            status: PoolerStatus::pending(),
            generation: 1,
        };
        poolers.insert(name, pooler.clone());
        Ok(pooler)
    }

    pub fn get(&self, name: &str) -> Result<Pooler, PoolerError> {
        self.poolers
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| PoolerError::NotFound(name.into()))
    }

    pub fn list(&self) -> Vec<Pooler> {
        self.poolers.read().unwrap().values().cloned().collect()
    }

    pub fn list_for_instance(&self, instance_id: &str) -> Vec<Pooler> {
        self.poolers
            .read()
            .unwrap()
            .values()
            .filter(|p| p.spec.instance_id == instance_id)
            .cloned()
            .collect()
    }

    /// Replace a pooler's spec. Bumps generation so the reconciler
    /// notices the change.
    pub fn update_spec(&self, name: &str, spec: PoolerSpec) -> Result<Pooler, PoolerError> {
        spec.validate()?;
        let mut poolers = self.poolers.write().unwrap();
        let pooler = poolers
            .get_mut(name)
            .ok_or_else(|| PoolerError::NotFound(name.into()))?;
        pooler.spec = spec;
        pooler.generation += 1;
        Ok(pooler.clone())
    }

    pub fn delete(&self, name: &str) -> Result<(), PoolerError> {
        self.poolers
            .write()
            .unwrap()
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| PoolerError::NotFound(name.into()))
    }

    /// One reconcile pass: move status toward spec. `ready_replicas`
    /// is provided by the caller (typically the pod-status feed).
    pub fn reconcile(&self, name: &str, observed_ready: u32) -> Result<Pooler, PoolerError> {
        let mut poolers = self.poolers.write().unwrap();
        let pooler = poolers
            .get_mut(name)
            .ok_or_else(|| PoolerError::NotFound(name.into()))?;
        let desired = pooler.spec.replicas;
        pooler.status.ready_replicas = observed_ready;
        pooler.status.observed_generation = pooler.generation;
        pooler.status.last_updated = Utc::now();
        pooler.status.phase = if observed_ready == 0 {
            PoolerPhase::Configuring
        } else if observed_ready < desired {
            PoolerPhase::Degraded
        } else {
            PoolerPhase::Ready
        };
        push_condition(
            &mut pooler.status.conditions,
            "Available",
            observed_ready == desired,
            if observed_ready == desired {
                "AsExpected"
            } else {
                "BelowDesired"
            },
            &format!("{}/{} replicas ready", observed_ready, desired),
        );
        Ok(pooler.clone())
    }

    /// Render the PgBouncer config that would be projected into each
    /// pod's `pgbouncer.ini`. Mirrors the upstream CNPG template.
    pub fn render_pgbouncer_ini(&self, name: &str) -> Result<String, PoolerError> {
        let p = self.get(name)?;
        let mode = match p.spec.pool_mode {
            PoolMode::Session => "session",
            PoolMode::Transaction => "transaction",
            PoolMode::Statement => "statement",
        };
        Ok(format!(
            concat!(
                "[pgbouncer]\n",
                "listen_addr = 0.0.0.0\n",
                "listen_port = 6432\n",
                "pool_mode = {mode}\n",
                "max_client_conn = {max_client_conn}\n",
                "default_pool_size = {default_pool_size}\n",
                "min_pool_size = {min_pool_size}\n",
                "server_tls_sslmode = verify-ca\n",
                "auth_type = scram-sha-256\n",
                "\n",
                "[databases]\n",
                "* = host=pg-{instance_id} port=5432 dbname=postgres\n",
            ),
            mode = mode,
            max_client_conn = p.spec.max_client_conn,
            default_pool_size = p.spec.default_pool_size,
            min_pool_size = p.spec.min_pool_size,
            instance_id = p.spec.instance_id,
        ))
    }
}

impl Default for PoolerManager {
    fn default() -> Self {
        Self::new()
    }
}

fn push_condition(
    conditions: &mut Vec<PoolerCondition>,
    kind: &str,
    status: bool,
    reason: &str,
    message: &str,
) {
    if let Some(existing) = conditions.iter_mut().find(|c| c.kind == kind) {
        let changed = existing.status != status || existing.reason != reason;
        existing.status = status;
        existing.reason = reason.into();
        existing.message = message.into();
        if changed {
            existing.last_transition = Utc::now();
        }
        return;
    }
    conditions.push(PoolerCondition {
        kind: kind.into(),
        status,
        reason: reason.into(),
        message: message.into(),
        last_transition: Utc::now(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> PoolerSpec {
        PoolerSpec {
            instance_id: "acme-prod".into(),
            replicas: 3,
            pool_mode: PoolMode::Transaction,
            default_pool_size: 20,
            max_client_conn: 200,
            min_pool_size: 5,
            image: "pgbouncer:1.21".into(),
        }
    }

    #[test]
    fn create_validates_and_stores() {
        let m = PoolerManager::new();
        let p = m.create("rw-pool", sample_spec()).unwrap();
        assert_eq!(p.status.phase, PoolerPhase::Pending);
        assert_eq!(p.generation, 1);
    }

    #[test]
    fn create_duplicate_name_refused() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let err = m.create("p", sample_spec()).unwrap_err();
        assert!(matches!(err, PoolerError::AlreadyExists(_)));
    }

    #[test]
    fn invalid_spec_rejected() {
        let mut s = sample_spec();
        s.replicas = 0;
        let m = PoolerManager::new();
        assert!(matches!(
            m.create("p", s).unwrap_err(),
            PoolerError::InvalidSpec(_)
        ));
    }

    #[test]
    fn max_client_conn_below_default_pool_rejected() {
        let mut s = sample_spec();
        s.max_client_conn = 5;
        s.default_pool_size = 20;
        assert!(matches!(s.validate(), Err(PoolerError::InvalidSpec(_))));
    }

    #[test]
    fn reconcile_marks_ready_when_replicas_match() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let p = m.reconcile("p", 3).unwrap();
        assert_eq!(p.status.phase, PoolerPhase::Ready);
        assert_eq!(p.status.ready_replicas, 3);
    }

    #[test]
    fn reconcile_marks_degraded_when_under_desired() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let p = m.reconcile("p", 2).unwrap();
        assert_eq!(p.status.phase, PoolerPhase::Degraded);
    }

    #[test]
    fn reconcile_configuring_when_no_replicas() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let p = m.reconcile("p", 0).unwrap();
        assert_eq!(p.status.phase, PoolerPhase::Configuring);
    }

    #[test]
    fn update_spec_bumps_generation() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let mut s = sample_spec();
        s.replicas = 5;
        let p = m.update_spec("p", s).unwrap();
        assert_eq!(p.generation, 2);
        assert_eq!(p.spec.replicas, 5);
    }

    #[test]
    fn list_for_instance_filters() {
        let m = PoolerManager::new();
        m.create("a", sample_spec()).unwrap();
        let mut other = sample_spec();
        other.instance_id = "other".into();
        m.create("b", other).unwrap();
        assert_eq!(m.list_for_instance("acme-prod").len(), 1);
        assert_eq!(m.list_for_instance("other").len(), 1);
    }

    #[test]
    fn render_pgbouncer_ini_contains_mode_and_pool_settings() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let ini = m.render_pgbouncer_ini("p").unwrap();
        assert!(ini.contains("pool_mode = transaction"));
        assert!(ini.contains("max_client_conn = 200"));
        assert!(ini.contains("default_pool_size = 20"));
        assert!(ini.contains("dbname=postgres"));
    }

    #[test]
    fn delete_removes_pooler() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        m.delete("p").unwrap();
        assert!(matches!(m.get("p").unwrap_err(), PoolerError::NotFound(_)));
    }

    #[test]
    fn condition_transition_timestamp_updates_on_change() {
        let m = PoolerManager::new();
        m.create("p", sample_spec()).unwrap();
        let p1 = m.reconcile("p", 1).unwrap();
        let t1 = p1
            .status
            .conditions
            .iter()
            .find(|c| c.kind == "Available")
            .map(|c| c.last_transition)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let p2 = m.reconcile("p", 3).unwrap();
        let t2 = p2
            .status
            .conditions
            .iter()
            .find(|c| c.kind == "Available")
            .map(|c| c.last_transition)
            .unwrap();
        assert!(
            t2 > t1,
            "transition timestamp must advance on status change"
        );
    }

    #[test]
    fn min_pool_size_above_default_rejected() {
        let mut s = sample_spec();
        s.min_pool_size = 100;
        s.default_pool_size = 20;
        assert!(matches!(s.validate(), Err(PoolerError::InvalidSpec(_))));
    }

    #[test]
    fn empty_image_rejected() {
        let mut s = sample_spec();
        s.image = "".into();
        assert!(matches!(s.validate(), Err(PoolerError::InvalidSpec(_))));
    }
}
