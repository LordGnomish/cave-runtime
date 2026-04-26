//! Indexed Job — per-completion-index scheduler + completion table.
//!
//! Mirrors `pkg/controller/job/indexed_job_utils.go::firstPendingIndexes`
//! plus the per-index status tracking added in KEP-2214 ("Indexed Job"),
//! GA in v1.24 and the canonical default for parallel batch in v1.36.

use crate::types::{Cite, ControllerError, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-index outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexState {
    Pending,
    Active,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedJobSpec {
    pub name: String,
    pub tenant: TenantId,
    pub completions: u32,
    pub parallelism: u32,
    /// Maximum failures per index before the slot is marked Failed
    /// permanently. Mirrors `BackoffLimitPerIndex`.
    pub backoff_limit_per_index: u32,
    pub suspended: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedJobStatus {
    /// `index → state`. Indexes not present are implicitly Pending.
    pub index_states: BTreeMap<u32, IndexState>,
    /// `index → consecutive failure count`.
    pub index_failures: BTreeMap<u32, u32>,
}

impl IndexedJobStatus {
    pub fn state_of(&self, idx: u32) -> IndexState {
        self.index_states.get(&idx).copied().unwrap_or(IndexState::Pending)
    }
    pub fn count(&self, want: IndexState) -> u32 {
        self.index_states.values().filter(|s| **s == want).count() as u32
    }
}

/// Decision returned by [`schedule_next`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexedDecision {
    /// Launch pods for these indexes (sorted ascending).
    Launch(Vec<u32>),
    /// All complete — Job is done.
    Done,
    /// At least one index has exceeded its backoff limit; Job is permanently failed.
    Failed(Vec<u32>),
    /// Suspended — drop everything Active.
    SuspendDrain(Vec<u32>),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IndexedError {
    #[error("invalid spec: {0}")]
    BadSpec(&'static str),
}

/// Mirrors `firstPendingIndexes` + the parallelism cap in
/// `pkg/controller/job/job_controller.go::manageJob`.
///
/// Selects up to `parallelism - active_count` Pending indexes (lowest-first)
/// to launch this pass, unless the job is suspended (drain) or already done
/// (no-op). If any index has crossed its backoff limit, the entire job is
/// reported as Failed (mirrors per-index BackoffLimitPerIndex semantics).
pub fn schedule_next(
    spec: &IndexedJobSpec,
    status: &IndexedJobStatus,
    caller: &TenantId,
) -> Result<IndexedDecision, ControllerError> {
    if caller != &spec.tenant {
        return Err(ControllerError::TenantDenied {
            tenant: caller.clone(),
            kind: "IndexedJob",
            name: spec.name.clone(),
        });
    }
    if spec.completions == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "IndexedJob",
            reason: "completions must be > 0".into(),
        });
    }
    if spec.parallelism == 0 {
        return Err(ControllerError::InvalidSpec {
            kind: "IndexedJob",
            reason: "parallelism must be > 0".into(),
        });
    }

    if spec.suspended {
        let to_drain: Vec<u32> = (0..spec.completions)
            .filter(|i| status.state_of(*i) == IndexState::Active)
            .collect();
        return Ok(IndexedDecision::SuspendDrain(to_drain));
    }

    // Per-index backoff: any index that has overshot its limit fails the job.
    let exceeded: Vec<u32> = status
        .index_failures
        .iter()
        .filter(|(_, n)| **n > spec.backoff_limit_per_index)
        .map(|(i, _)| *i)
        .collect();
    if !exceeded.is_empty() {
        return Ok(IndexedDecision::Failed(exceeded));
    }

    let succeeded = status.count(IndexState::Succeeded);
    if succeeded >= spec.completions {
        return Ok(IndexedDecision::Done);
    }

    let active = status.count(IndexState::Active);
    let budget = spec.parallelism.saturating_sub(active);
    if budget == 0 {
        return Ok(IndexedDecision::Launch(vec![]));
    }

    let mut to_launch = Vec::with_capacity(budget as usize);
    for i in 0..spec.completions {
        if status.state_of(i) == IndexState::Pending {
            to_launch.push(i);
            if to_launch.len() == budget as usize {
                break;
            }
        }
    }
    Ok(IndexedDecision::Launch(to_launch))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/job/indexed_job_utils.go",
    "firstPendingIndexes",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn spec(completions: u32, parallelism: u32, backoff: u32, suspended: bool) -> IndexedJobSpec {
        IndexedJobSpec {
            name: "build".into(),
            tenant: TenantId::new("acme"),
            completions,
            parallelism,
            backoff_limit_per_index: backoff,
            suspended,
        }
    }

    fn status_with(states: &[(u32, IndexState)], failures: &[(u32, u32)]) -> IndexedJobStatus {
        IndexedJobStatus {
            index_states: states.iter().copied().collect(),
            index_failures: failures.iter().copied().collect(),
        }
    }

    #[test]
    fn launches_first_indexes_up_to_parallelism() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "firstPendingIndexes",
            "acme"
        );
        let s = spec(10, 3, 6, false);
        let st = IndexedJobStatus::default();
        assert_eq!(
            schedule_next(&s, &st, &tenant).unwrap(),
            IndexedDecision::Launch(vec![0, 1, 2])
        );
    }

    #[test]
    fn skips_active_and_succeeded_indexes_when_picking_next() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "firstPendingIndexes",
            "acme"
        );
        let s = spec(5, 3, 6, false);
        let st = status_with(
            &[(0, IndexState::Succeeded), (1, IndexState::Active), (2, IndexState::Active)],
            &[],
        );
        // active=2, parallelism=3 → budget=1, lowest pending is 3.
        assert_eq!(
            schedule_next(&s, &st, &tenant).unwrap(),
            IndexedDecision::Launch(vec![3])
        );
    }

    #[test]
    fn launch_returns_empty_when_parallelism_is_saturated() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "manageJob",
            "acme"
        );
        let s = spec(5, 2, 6, false);
        let st = status_with(&[(0, IndexState::Active), (1, IndexState::Active)], &[]);
        assert_eq!(
            schedule_next(&s, &st, &tenant).unwrap(),
            IndexedDecision::Launch(vec![])
        );
    }

    #[test]
    fn done_when_all_completions_succeeded() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/util/job_utils.go",
            "IsJobFinished",
            "acme"
        );
        let s = spec(3, 3, 6, false);
        let st = status_with(
            &[(0, IndexState::Succeeded), (1, IndexState::Succeeded), (2, IndexState::Succeeded)],
            &[],
        );
        assert_eq!(schedule_next(&s, &st, &tenant).unwrap(), IndexedDecision::Done);
    }

    #[test]
    fn per_index_backoff_breach_reports_failed_with_offending_indexes() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "BackoffLimitPerIndex",
            "acme"
        );
        let s = spec(5, 2, 2, false);
        // Index 1 has 3 consecutive failures, > backoff_limit_per_index=2.
        let st = status_with(&[], &[(1, 3)]);
        let d = schedule_next(&s, &st, &tenant).unwrap();
        assert_eq!(d, IndexedDecision::Failed(vec![1]));
    }

    #[test]
    fn at_threshold_failures_do_not_fail_yet() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/indexed_job_utils.go",
            "BackoffLimitPerIndex",
            "acme"
        );
        let s = spec(5, 2, 2, false);
        // Exactly 2 failures = at threshold, not over.
        let st = status_with(&[], &[(1, 2)]);
        let d = schedule_next(&s, &st, &tenant).unwrap();
        assert!(matches!(d, IndexedDecision::Launch(_)));
    }

    #[test]
    fn suspended_drains_active_indexes() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "ensureJobConditionStatus",
            "acme"
        );
        let s = spec(5, 5, 6, true);
        let st = status_with(&[(0, IndexState::Active), (2, IndexState::Active)], &[]);
        assert_eq!(
            schedule_next(&s, &st, &tenant).unwrap(),
            IndexedDecision::SuspendDrain(vec![0, 2])
        );
    }

    #[test]
    fn invalid_spec_zero_completions_is_rejected() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "validateJobSpec",
            "acme"
        );
        let s = spec(0, 1, 6, false);
        let err = schedule_next(&s, &IndexedJobStatus::default(), &tenant).unwrap_err();
        assert!(matches!(err, ControllerError::InvalidSpec { .. }));
    }

    #[test]
    fn cross_tenant_caller_is_refused() {
        let (_cite, attacker) = test_ctx!(
            "pkg/controller/job/job_controller.go",
            "tenantCheck",
            "tenant-attacker"
        );
        let s = spec(3, 3, 6, false);
        let err = schedule_next(&s, &IndexedJobStatus::default(), &attacker).unwrap_err();
        assert!(matches!(err, ControllerError::TenantDenied { .. }));
    }
}
