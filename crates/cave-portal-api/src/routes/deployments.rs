// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Deployments — list, get, rollout/rollback. Tenant-scoped.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployStatus {
    Pending,
    Healthy,
    Degraded,
    Failed,
    RollingBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub tenant: String,
    pub app: String,
    pub revision: u64,
    pub image: String,
    pub status: DeployStatus,
    pub replicas_desired: u32,
    pub replicas_ready: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateDeployRequest {
    pub tenant: String,
    pub app: String,
    pub image: String,
    pub replicas: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DeploymentsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid replica count {0}")]
    InvalidReplicas(u32),
    #[error("cannot rollback at revision 1")]
    NoPriorRevision,
}

pub struct DeploymentStore {
    inner: Mutex<HashMap<String, Vec<Deployment>>>, // id -> revision history
}

impl Default for DeploymentStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl DeploymentStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn make_id(tenant: &str, app: &str) -> String {
        format!("{tenant}/{app}")
    }

    pub fn list(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
    ) -> Result<Vec<Deployment>, DeploymentsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let guard = self.inner.lock().unwrap();
        let mut out: Vec<Deployment> = guard
            .iter()
            .filter(|(id, _)| id.starts_with(&format!("{tenant}/")))
            .filter_map(|(_, history)| history.last().cloned())
            .collect();
        out.sort_by(|a, b| a.app.cmp(&b.app));
        Ok(out)
    }

    pub fn get(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        app: &str,
    ) -> Result<Deployment, DeploymentsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let id = Self::make_id(tenant, app);
        let guard = self.inner.lock().unwrap();
        guard
            .get(&id)
            .and_then(|history| history.last().cloned())
            .ok_or_else(|| DeploymentsError::NotFound(id))
    }

    pub fn history(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        app: &str,
    ) -> Result<Vec<Deployment>, DeploymentsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let id = Self::make_id(tenant, app);
        let guard = self.inner.lock().unwrap();
        Ok(guard.get(&id).cloned().unwrap_or_default())
    }

    pub fn deploy(
        &self,
        principal: Option<&Principal>,
        req: CreateDeployRequest,
    ) -> Result<Deployment, DeploymentsError> {
        Guard::cross_persona(Some("deployments:write")).authorize(principal, Some(&req.tenant))?;
        if req.replicas == 0 || req.replicas > 1000 {
            return Err(DeploymentsError::InvalidReplicas(req.replicas));
        }
        let id = Self::make_id(&req.tenant, &req.app);
        let mut guard = self.inner.lock().unwrap();
        let history = guard.entry(id.clone()).or_default();
        let next_rev = history.last().map(|d| d.revision + 1).unwrap_or(1);
        let deployment = Deployment {
            id,
            tenant: req.tenant,
            app: req.app,
            revision: next_rev,
            image: req.image,
            status: DeployStatus::Pending,
            replicas_desired: req.replicas,
            replicas_ready: 0,
        };
        history.push(deployment.clone());
        Ok(deployment)
    }

    pub fn rollback(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        app: &str,
    ) -> Result<Deployment, DeploymentsError> {
        Guard::cross_persona(Some("deployments:rollout")).authorize(principal, Some(tenant))?;
        let id = Self::make_id(tenant, app);
        let mut guard = self.inner.lock().unwrap();
        let history = guard
            .get_mut(&id)
            .ok_or_else(|| DeploymentsError::NotFound(id.clone()))?;
        if history.len() < 2 {
            return Err(DeploymentsError::NoPriorRevision);
        }
        let prev = history[history.len() - 2].clone();
        let mut rolled = prev;
        rolled.revision = history.last().unwrap().revision + 1;
        rolled.status = DeployStatus::RollingBack;
        history.push(rolled.clone());
        Ok(rolled)
    }

    pub fn mark_healthy(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        app: &str,
        ready: u32,
    ) -> Result<Deployment, DeploymentsError> {
        Guard::operator_only().authorize(principal, None)?;
        let id = Self::make_id(tenant, app);
        let mut guard = self.inner.lock().unwrap();
        let history = guard
            .get_mut(&id)
            .ok_or_else(|| DeploymentsError::NotFound(id))?;
        let last = history.last_mut().unwrap();
        last.replicas_ready = ready;
        last.status = if ready >= last.replicas_desired {
            DeployStatus::Healthy
        } else {
            DeployStatus::Degraded
        };
        Ok(last.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn admin() -> Principal {
        Principal::new("a", Persona::Admin)
            .with_role("deployments:write")
            .with_role("deployments:rollout")
    }
    fn dev(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant)
            .with_tenant(t)
            .with_role("deployments:write")
            .with_role("deployments:rollout")
    }
    fn dev_no_role(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t)
    }
    fn op() -> Principal {
        Principal::new("o", Persona::Operator)
    }

    fn req(t: &str, a: &str) -> CreateDeployRequest {
        CreateDeployRequest {
            tenant: t.into(),
            app: a.into(),
            image: "img:1".into(),
            replicas: 3,
        }
    }

    #[test]
    fn store_starts_empty() {
        let s = DeploymentStore::new();
        assert!(s.list(Some(&admin()), "acme").unwrap().is_empty());
    }

    #[test]
    fn deploy_anonymous_denied() {
        let s = DeploymentStore::new();
        let err = s.deploy(None, req("acme", "web")).unwrap_err();
        assert!(matches!(
            err,
            DeploymentsError::Guard(GuardError::Anonymous)
        ));
    }

    #[test]
    fn deploy_without_role_denied() {
        let s = DeploymentStore::new();
        let err = s
            .deploy(Some(&dev_no_role("acme")), req("acme", "web"))
            .unwrap_err();
        assert!(matches!(
            err,
            DeploymentsError::Guard(GuardError::MissingRole(_))
        ));
    }

    #[test]
    fn deploy_succeeds() {
        let s = DeploymentStore::new();
        let d = s.deploy(Some(&dev("acme")), req("acme", "web")).unwrap();
        assert_eq!(d.revision, 1);
        assert_eq!(d.status, DeployStatus::Pending);
        assert_eq!(d.replicas_desired, 3);
    }

    #[test]
    fn deploy_increments_revision() {
        let s = DeploymentStore::new();
        s.deploy(Some(&dev("acme")), req("acme", "web")).unwrap();
        let d2 = s.deploy(Some(&dev("acme")), req("acme", "web")).unwrap();
        assert_eq!(d2.revision, 2);
    }

    #[test]
    fn deploy_zero_replicas_rejected() {
        let s = DeploymentStore::new();
        let mut r = req("acme", "web");
        r.replicas = 0;
        let err = s.deploy(Some(&dev("acme")), r).unwrap_err();
        assert!(matches!(err, DeploymentsError::InvalidReplicas(0)));
    }

    #[test]
    fn deploy_huge_replicas_rejected() {
        let s = DeploymentStore::new();
        let mut r = req("acme", "web");
        r.replicas = 5000;
        let err = s.deploy(Some(&dev("acme")), r).unwrap_err();
        assert!(matches!(err, DeploymentsError::InvalidReplicas(5000)));
    }

    #[test]
    fn deploy_cross_tenant_denied_for_tenant_persona() {
        let s = DeploymentStore::new();
        let err = s
            .deploy(Some(&dev("globex")), req("acme", "web"))
            .unwrap_err();
        assert!(matches!(
            err,
            DeploymentsError::Guard(GuardError::TenantMismatch { .. })
        ));
    }

    #[test]
    fn list_by_tenant_filters() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        s.deploy(Some(&admin()), req("globex", "api")).unwrap();
        let acme = s.list(Some(&admin()), "acme").unwrap();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].app, "web");
    }

    #[test]
    fn list_returns_sorted_by_app() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "zeta")).unwrap();
        s.deploy(Some(&admin()), req("acme", "alpha")).unwrap();
        s.deploy(Some(&admin()), req("acme", "mu")).unwrap();
        let apps: Vec<_> = s
            .list(Some(&admin()), "acme")
            .unwrap()
            .into_iter()
            .map(|d| d.app)
            .collect();
        assert_eq!(apps, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn get_returns_latest_revision() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        let mut r2 = req("acme", "web");
        r2.image = "img:2".into();
        s.deploy(Some(&admin()), r2).unwrap();
        let d = s.get(Some(&admin()), "acme", "web").unwrap();
        assert_eq!(d.revision, 2);
        assert_eq!(d.image, "img:2");
    }

    #[test]
    fn get_not_found() {
        let s = DeploymentStore::new();
        let err = s.get(Some(&admin()), "acme", "ghost").unwrap_err();
        assert!(matches!(err, DeploymentsError::NotFound(_)));
    }

    #[test]
    fn get_tenant_persona_cross_denied() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        let err = s.get(Some(&dev("globex")), "acme", "web").unwrap_err();
        assert!(matches!(
            err,
            DeploymentsError::Guard(GuardError::TenantMismatch { .. })
        ));
    }

    #[test]
    fn history_lists_all_revisions() {
        let s = DeploymentStore::new();
        for i in 0..3 {
            let mut r = req("acme", "web");
            r.image = format!("img:{i}");
            s.deploy(Some(&admin()), r).unwrap();
        }
        let h = s.history(Some(&admin()), "acme", "web").unwrap();
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].revision, 1);
        assert_eq!(h[2].revision, 3);
    }

    #[test]
    fn rollback_requires_two_revisions() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        let err = s.rollback(Some(&admin()), "acme", "web").unwrap_err();
        assert_eq!(err, DeploymentsError::NoPriorRevision);
    }

    #[test]
    fn rollback_creates_new_revision_with_old_image() {
        let s = DeploymentStore::new();
        let mut r1 = req("acme", "web");
        r1.image = "v1".into();
        s.deploy(Some(&admin()), r1).unwrap();
        let mut r2 = req("acme", "web");
        r2.image = "v2".into();
        s.deploy(Some(&admin()), r2).unwrap();
        let rb = s.rollback(Some(&admin()), "acme", "web").unwrap();
        assert_eq!(rb.image, "v1");
        assert_eq!(rb.revision, 3);
        assert_eq!(rb.status, DeployStatus::RollingBack);
    }

    #[test]
    fn rollback_unknown_app() {
        let s = DeploymentStore::new();
        let err = s.rollback(Some(&admin()), "acme", "ghost").unwrap_err();
        assert!(matches!(err, DeploymentsError::NotFound(_)));
    }

    #[test]
    fn mark_healthy_requires_operator() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        let err = s
            .mark_healthy(Some(&dev("acme")), "acme", "web", 3)
            .unwrap_err();
        assert!(matches!(
            err,
            DeploymentsError::Guard(GuardError::PersonaForbidden { .. })
        ));
    }

    #[test]
    fn mark_healthy_updates_status() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        let d = s.mark_healthy(Some(&op()), "acme", "web", 3).unwrap();
        assert_eq!(d.status, DeployStatus::Healthy);
        assert_eq!(d.replicas_ready, 3);
    }

    #[test]
    fn mark_healthy_degraded_when_partial() {
        let s = DeploymentStore::new();
        s.deploy(Some(&admin()), req("acme", "web")).unwrap();
        let d = s.mark_healthy(Some(&op()), "acme", "web", 1).unwrap();
        assert_eq!(d.status, DeployStatus::Degraded);
    }

    #[test]
    fn mark_healthy_unknown_app() {
        let s = DeploymentStore::new();
        let err = s.mark_healthy(Some(&op()), "acme", "ghost", 1).unwrap_err();
        assert!(matches!(err, DeploymentsError::NotFound(_)));
    }

    #[test]
    fn deploy_status_serializes_snake_case() {
        let s = serde_json::to_string(&DeployStatus::RollingBack).unwrap();
        assert_eq!(s, "\"rolling_back\"");
    }

    #[test]
    fn deployment_round_trips_json() {
        let d = Deployment {
            id: "acme/web".into(),
            tenant: "acme".into(),
            app: "web".into(),
            revision: 3,
            image: "img:3".into(),
            status: DeployStatus::Healthy,
            replicas_desired: 5,
            replicas_ready: 5,
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: Deployment = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }
}
