//! Background jobs — submit, list, cancel. Tenant-scoped.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::routes::rbac::{Guard, GuardError, Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobState::Succeeded | JobState::Failed | JobState::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub tenant: String,
    pub kind: String,
    pub state: JobState,
    pub submitted_at: String,
    pub finished_at: Option<String>,
    pub progress_pct: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitJobRequest {
    pub tenant: String,
    pub kind: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JobsError {
    #[error("guard: {0}")]
    Guard(#[from] GuardError),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid kind: {0:?}")]
    InvalidKind(String),
    #[error("job already terminal: {0:?}")]
    AlreadyTerminal(JobState),
}

const VALID_JOB_KINDS: &[&str] = &[
    "build",
    "deploy",
    "scan",
    "rotate-secret",
    "snapshot",
    "rollout",
    "audit",
];

pub struct JobStore {
    inner: Mutex<HashMap<String, Job>>,
    seq: Mutex<u64>,
}

impl Default for JobStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            seq: Mutex::new(0),
        }
    }
}

impl JobStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
    ) -> Result<Vec<Job>, JobsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let guard = self.inner.lock().unwrap();
        let mut out: Vec<Job> = guard.values().filter(|j| j.tenant == tenant).cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub fn get(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        id: &str,
    ) -> Result<Job, JobsError> {
        Guard::cross_persona(None).authorize(principal, Some(tenant))?;
        let guard = self.inner.lock().unwrap();
        let job = guard
            .get(id)
            .ok_or_else(|| JobsError::NotFound(id.into()))?
            .clone();
        if job.tenant != tenant {
            return Err(JobsError::NotFound(id.into()));
        }
        Ok(job)
    }

    pub fn submit(
        &self,
        principal: Option<&Principal>,
        req: SubmitJobRequest,
    ) -> Result<Job, JobsError> {
        Guard::cross_persona(Some("jobs:submit"))
            .authorize(principal, Some(&req.tenant))?;
        if !VALID_JOB_KINDS.contains(&req.kind.as_str()) {
            return Err(JobsError::InvalidKind(req.kind));
        }
        let mut seq = self.seq.lock().unwrap();
        *seq += 1;
        let id = format!("job-{:06}", *seq);
        drop(seq);
        let job = Job {
            id: id.clone(),
            tenant: req.tenant,
            kind: req.kind,
            state: JobState::Queued,
            submitted_at: "1970-01-01T00:00:00Z".into(),
            finished_at: None,
            progress_pct: 0,
        };
        self.inner.lock().unwrap().insert(id, job.clone());
        Ok(job)
    }

    pub fn transition(
        &self,
        principal: Option<&Principal>,
        id: &str,
        new_state: JobState,
        progress: u8,
    ) -> Result<Job, JobsError> {
        Guard::operator_only().authorize(principal, None)?;
        let mut guard = self.inner.lock().unwrap();
        let job = guard.get_mut(id).ok_or_else(|| JobsError::NotFound(id.into()))?;
        if job.state.is_terminal() {
            return Err(JobsError::AlreadyTerminal(job.state));
        }
        job.state = new_state;
        job.progress_pct = progress.min(100);
        if new_state.is_terminal() {
            job.finished_at = Some("1970-01-01T00:00:00Z".into());
        }
        Ok(job.clone())
    }

    pub fn cancel(
        &self,
        principal: Option<&Principal>,
        tenant: &str,
        id: &str,
    ) -> Result<Job, JobsError> {
        Guard::cross_persona(Some("jobs:cancel"))
            .authorize(principal, Some(tenant))?;
        let mut guard = self.inner.lock().unwrap();
        let job = guard.get_mut(id).ok_or_else(|| JobsError::NotFound(id.into()))?;
        if job.tenant != tenant {
            return Err(JobsError::NotFound(id.into()));
        }
        if job.state.is_terminal() {
            return Err(JobsError::AlreadyTerminal(job.state));
        }
        job.state = JobState::Cancelled;
        job.finished_at = Some("1970-01-01T00:00:00Z".into());
        Ok(job.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::rbac::Persona;

    fn dev(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant)
            .with_tenant(t)
            .with_role("jobs:submit")
            .with_role("jobs:cancel")
    }
    fn dev_no_role(t: &str) -> Principal {
        Principal::new("d", Persona::Tenant).with_tenant(t)
    }
    fn op() -> Principal {
        Principal::new("o", Persona::Operator)
    }
    fn admin() -> Principal {
        Principal::new("a", Persona::Admin).with_role("jobs:submit").with_role("jobs:cancel")
    }

    fn req(t: &str, k: &str) -> SubmitJobRequest {
        SubmitJobRequest { tenant: t.into(), kind: k.into() }
    }

    #[test]
    fn job_state_terminal() {
        assert!(!JobState::Queued.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(JobState::Succeeded.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
    }

    #[test]
    fn submit_anonymous_denied() {
        let s = JobStore::new();
        let err = s.submit(None, req("acme", "build")).unwrap_err();
        assert!(matches!(err, JobsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn submit_without_role_denied() {
        let s = JobStore::new();
        let err = s.submit(Some(&dev_no_role("acme")), req("acme", "build")).unwrap_err();
        assert!(matches!(err, JobsError::Guard(GuardError::MissingRole(_))));
    }

    #[test]
    fn submit_succeeds() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        assert_eq!(j.state, JobState::Queued);
        assert_eq!(j.progress_pct, 0);
        assert!(j.id.starts_with("job-"));
    }

    #[test]
    fn submit_invalid_kind_rejected() {
        let s = JobStore::new();
        let err = s.submit(Some(&dev("acme")), req("acme", "evil")).unwrap_err();
        assert!(matches!(err, JobsError::InvalidKind(_)));
    }

    #[test]
    fn submit_all_valid_kinds_accepted() {
        let s = JobStore::new();
        for k in VALID_JOB_KINDS {
            let r = req("acme", k);
            assert!(s.submit(Some(&dev("acme")), r).is_ok());
        }
    }

    #[test]
    fn submit_cross_tenant_denied() {
        let s = JobStore::new();
        let err = s.submit(Some(&dev("globex")), req("acme", "build")).unwrap_err();
        assert!(matches!(err, JobsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = JobStore::new();
        s.submit(Some(&admin()), req("acme", "build")).unwrap();
        s.submit(Some(&admin()), req("globex", "deploy")).unwrap();
        let acme = s.list(Some(&admin()), "acme").unwrap();
        assert_eq!(acme.len(), 1);
        assert_eq!(acme[0].kind, "build");
    }

    #[test]
    fn list_anonymous_denied() {
        let s = JobStore::new();
        let err = s.list(None, "acme").unwrap_err();
        assert!(matches!(err, JobsError::Guard(GuardError::Anonymous)));
    }

    #[test]
    fn get_returns_job() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let got = s.get(Some(&dev("acme")), "acme", &j.id).unwrap();
        assert_eq!(got.id, j.id);
    }

    #[test]
    fn get_with_wrong_tenant_returns_not_found() {
        let s = JobStore::new();
        let j = s.submit(Some(&admin()), req("acme", "build")).unwrap();
        let err = s.get(Some(&admin()), "globex", &j.id).unwrap_err();
        assert!(matches!(err, JobsError::NotFound(_)));
    }

    #[test]
    fn transition_requires_operator() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let err = s.transition(Some(&dev("acme")), &j.id, JobState::Running, 50).unwrap_err();
        assert!(matches!(err, JobsError::Guard(GuardError::PersonaForbidden { .. })));
    }

    #[test]
    fn transition_to_running_updates_progress() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let updated = s.transition(Some(&op()), &j.id, JobState::Running, 50).unwrap();
        assert_eq!(updated.state, JobState::Running);
        assert_eq!(updated.progress_pct, 50);
        assert!(updated.finished_at.is_none());
    }

    #[test]
    fn transition_to_succeeded_sets_finished_at() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let updated = s.transition(Some(&op()), &j.id, JobState::Succeeded, 100).unwrap();
        assert!(updated.finished_at.is_some());
    }

    #[test]
    fn transition_clamps_progress_to_100() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let updated = s.transition(Some(&op()), &j.id, JobState::Running, 250).unwrap();
        assert_eq!(updated.progress_pct, 100);
    }

    #[test]
    fn transition_after_terminal_rejected() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        s.transition(Some(&op()), &j.id, JobState::Succeeded, 100).unwrap();
        let err = s.transition(Some(&op()), &j.id, JobState::Running, 0).unwrap_err();
        assert!(matches!(err, JobsError::AlreadyTerminal(JobState::Succeeded)));
    }

    #[test]
    fn cancel_succeeds_for_owner() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let c = s.cancel(Some(&dev("acme")), "acme", &j.id).unwrap();
        assert_eq!(c.state, JobState::Cancelled);
    }

    #[test]
    fn cancel_already_terminal_rejected() {
        let s = JobStore::new();
        let j = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        s.transition(Some(&op()), &j.id, JobState::Failed, 0).unwrap();
        let err = s.cancel(Some(&dev("acme")), "acme", &j.id).unwrap_err();
        assert!(matches!(err, JobsError::AlreadyTerminal(JobState::Failed)));
    }

    #[test]
    fn cancel_cross_tenant_denied() {
        let s = JobStore::new();
        let j = s.submit(Some(&admin()), req("acme", "build")).unwrap();
        let err = s.cancel(Some(&dev("globex")), "acme", &j.id).unwrap_err();
        assert!(matches!(err, JobsError::Guard(GuardError::TenantMismatch { .. })));
    }

    #[test]
    fn cancel_unknown_id() {
        let s = JobStore::new();
        let err = s.cancel(Some(&dev("acme")), "acme", "ghost").unwrap_err();
        assert!(matches!(err, JobsError::NotFound(_)));
    }

    #[test]
    fn submit_assigns_unique_ids() {
        let s = JobStore::new();
        let j1 = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        let j2 = s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        assert_ne!(j1.id, j2.id);
    }

    #[test]
    fn list_sorted_by_id() {
        let s = JobStore::new();
        for _ in 0..3 {
            s.submit(Some(&dev("acme")), req("acme", "build")).unwrap();
        }
        let jobs = s.list(Some(&dev("acme")), "acme").unwrap();
        let ids: Vec<&str> = jobs.iter().map(|j| j.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }
}
