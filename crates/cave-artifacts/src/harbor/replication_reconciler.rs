// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: goharbor/harbor src/controller/replication/controller.go
//
//! Replication policy reconciler.
//!
//! Reads a `ReplicationPolicy` (selector + target + trigger), enumerates the
//! source artifacts that match the selector, and emits one
//! `ReplicationJob` per (artifact × target) pair. Jobs flow through three
//! states: `Pending → Running → Succeeded | Failed`, with a `Failed` job
//! eligible for `requeue_with_retry` until `max_retries` is reached.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Manual,
    EventBased,
    Scheduled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationPolicy {
    pub id: Uuid,
    pub name: String,
    pub source_project: String,
    pub target_registry: String,
    /// Repository-name patterns to include (substring match).
    #[serde(default)]
    pub include_patterns: Vec<String>,
    /// Repository-name patterns to exclude (substring match — checked after include).
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    pub trigger: TriggerKind,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_retries")]
    pub max_retries: u32,
}

fn default_true() -> bool {
    true
}
fn default_retries() -> u32 {
    3
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationJob {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub repository: String,
    pub reference: String,
    pub target_registry: String,
    pub state: JobState,
    pub attempt: u32,
    pub max_retries: u32,
    pub last_error: Option<String>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

impl ReplicationJob {
    pub fn pending(policy: &ReplicationPolicy, repo: &str, reference: &str) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            policy_id: policy.id,
            repository: repo.into(),
            reference: reference.into(),
            target_registry: policy.target_registry.clone(),
            state: JobState::Pending,
            attempt: 0,
            max_retries: policy.max_retries,
            last_error: None,
            created: now,
            updated: now,
        }
    }

    pub fn mark_running(&mut self) {
        self.state = JobState::Running;
        self.attempt += 1;
        self.updated = Utc::now();
    }

    pub fn mark_succeeded(&mut self) {
        self.state = JobState::Succeeded;
        self.last_error = None;
        self.updated = Utc::now();
    }

    pub fn mark_failed(&mut self, reason: impl Into<String>) {
        self.state = JobState::Failed;
        self.last_error = Some(reason.into());
        self.updated = Utc::now();
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.state, JobState::Failed) && self.attempt < self.max_retries
    }

    pub fn requeue_with_retry(&mut self) -> bool {
        if !self.is_retryable() {
            return false;
        }
        self.state = JobState::Pending;
        self.last_error = None;
        self.updated = Utc::now();
        true
    }
}

#[derive(Debug, Clone)]
pub struct SourceArtifact {
    pub repository: String,
    pub reference: String,
}

/// Materialise a list of pending jobs from a policy + the current source
/// inventory. Returns empty when policy is disabled or no artifacts match.
pub fn plan(policy: &ReplicationPolicy, sources: &[SourceArtifact]) -> Vec<ReplicationJob> {
    if !policy.enabled {
        return Vec::new();
    }
    sources
        .iter()
        .filter(|s| matches(policy, &s.repository))
        .map(|s| ReplicationJob::pending(policy, &s.repository, &s.reference))
        .collect()
}

fn matches(policy: &ReplicationPolicy, repo: &str) -> bool {
    let included = policy.include_patterns.is_empty()
        || policy.include_patterns.iter().any(|p| repo.contains(p));
    let excluded = policy.exclude_patterns.iter().any(|p| repo.contains(p));
    included && !excluded
}

/// Reconcile loop helper: given a set of in-progress jobs, return the subset
/// that is still active (`Pending` or `Running`) and the subset that needs
/// requeue (failed but retryable).
pub fn classify(jobs: &[ReplicationJob]) -> (Vec<&ReplicationJob>, Vec<&ReplicationJob>) {
    let mut active = Vec::new();
    let mut retry = Vec::new();
    for j in jobs {
        match j.state {
            JobState::Pending | JobState::Running => active.push(j),
            JobState::Failed if j.attempt < j.max_retries => retry.push(j),
            _ => {}
        }
    }
    (active, retry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ReplicationPolicy {
        ReplicationPolicy {
            id: Uuid::new_v4(),
            name: "p".into(),
            source_project: "library".into(),
            target_registry: "https://reg.example/".into(),
            include_patterns: vec![],
            exclude_patterns: vec![],
            trigger: TriggerKind::Manual,
            enabled: true,
            max_retries: 2,
        }
    }

    #[test]
    fn plan_emits_one_job_per_source() {
        let sources = vec![
            SourceArtifact {
                repository: "library/nginx".into(),
                reference: "1.25".into(),
            },
            SourceArtifact {
                repository: "library/redis".into(),
                reference: "7".into(),
            },
        ];
        let jobs = plan(&policy(), &sources);
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().all(|j| j.state == JobState::Pending));
    }

    #[test]
    fn plan_skips_disabled_policy() {
        let mut p = policy();
        p.enabled = false;
        let jobs = plan(
            &p,
            &[SourceArtifact {
                repository: "x".into(),
                reference: "y".into(),
            }],
        );
        assert!(jobs.is_empty());
    }

    #[test]
    fn include_filter_restricts_set() {
        let mut p = policy();
        p.include_patterns = vec!["nginx".into()];
        let sources = vec![
            SourceArtifact {
                repository: "library/nginx".into(),
                reference: "1".into(),
            },
            SourceArtifact {
                repository: "library/redis".into(),
                reference: "1".into(),
            },
        ];
        let jobs = plan(&p, &sources);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].repository, "library/nginx");
    }

    #[test]
    fn exclude_filter_drops_match() {
        let mut p = policy();
        p.exclude_patterns = vec!["test".into()];
        let sources = vec![
            SourceArtifact {
                repository: "library/test-image".into(),
                reference: "1".into(),
            },
            SourceArtifact {
                repository: "library/nginx".into(),
                reference: "1".into(),
            },
        ];
        let jobs = plan(&p, &sources);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].repository, "library/nginx");
    }

    #[test]
    fn job_state_transitions_through_lifecycle() {
        let p = policy();
        let mut j = ReplicationJob::pending(&p, "r", "ref");
        assert_eq!(j.state, JobState::Pending);
        assert_eq!(j.attempt, 0);
        j.mark_running();
        assert_eq!(j.state, JobState::Running);
        assert_eq!(j.attempt, 1);
        j.mark_succeeded();
        assert_eq!(j.state, JobState::Succeeded);
    }

    #[test]
    fn failed_job_is_retryable_under_max() {
        let p = policy();
        let mut j = ReplicationJob::pending(&p, "r", "ref");
        j.mark_running();
        j.mark_failed("timeout");
        assert!(j.is_retryable());
        assert!(j.requeue_with_retry());
        assert_eq!(j.state, JobState::Pending);
    }

    #[test]
    fn failed_job_not_retryable_past_max() {
        let mut p = policy();
        p.max_retries = 1;
        let mut j = ReplicationJob::pending(&p, "r", "ref");
        j.mark_running();
        j.mark_failed("e");
        assert!(!j.is_retryable());
        assert!(!j.requeue_with_retry());
    }

    #[test]
    fn classify_buckets_active_and_retryable() {
        let p = policy();
        let mut succeeded = ReplicationJob::pending(&p, "a", "ref");
        succeeded.mark_running();
        succeeded.mark_succeeded();
        let mut retryable = ReplicationJob::pending(&p, "b", "ref");
        retryable.mark_running();
        retryable.mark_failed("e");
        let pending = ReplicationJob::pending(&p, "c", "ref");

        let jobs = vec![succeeded.clone(), retryable.clone(), pending.clone()];
        let (active, retry) = classify(&jobs);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].repository, "c");
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].repository, "b");
    }

    #[test]
    fn policy_serde_roundtrip() {
        let p = policy();
        let j = serde_json::to_string(&p).unwrap();
        let back: ReplicationPolicy = serde_json::from_str(&j).unwrap();
        assert_eq!(back.id, p.id);
        assert_eq!(back.trigger, TriggerKind::Manual);
    }

    #[test]
    fn matches_with_include_and_exclude() {
        let mut p = policy();
        p.include_patterns = vec!["library/".into()];
        p.exclude_patterns = vec!["library/test".into()];
        assert!(matches(&p, "library/nginx"));
        assert!(!matches(&p, "library/test-image"));
        assert!(!matches(&p, "other/redis"));
    }
}
